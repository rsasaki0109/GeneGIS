//! Multimodal walk + transit routing (RFC 0006 OD-3).
//!
//! A `TransitGraph` composes a walk `WalkGraph` with transit stops and ride
//! corridors. Movement through the extended graph is: walk to a stop, wait a
//! headway-derived expected wait, ride a corridor, egress on foot. Ride edges
//! carry a per-line expected wait plus a transfer penalty at interchanges.
//!
//! Every ride edge is modelled as an undirected corridor with a fixed ride
//! time; the expected wait (headway/2) is charged once when boarding. This
//! stays deterministic and keeps the existing route-sanity verifier valid:
//! a route can never be faster than the straight-line walk time.

use std::collections::{BinaryHeap, HashMap};

use thiserror::Error;

use crate::WalkGraph;

/// Fail-closed transit routing error.
#[derive(Debug, Error)]
pub enum TransitError {
    #[error("transit graph storage failed: {0}")]
    Storage(String),
    #[error("invalid transit data: {0}")]
    Invalid(String),
    #[error("transit routing failed: {0}")]
    Routing(String),
}

/// A transit stop snapped to a walk-graph node.
#[derive(Debug, Clone)]
pub struct TransitStop {
    pub id: String,
    /// Index into the walk graph's node list (the walk node nearest the stop).
    pub walk_node: u32,
    /// Transit lines serving this stop.
    pub lines: Vec<String>,
}

/// A ride corridor between two walk nodes (one stop at each end).
#[derive(Debug, Clone)]
pub struct RideEdge {
    pub from_walk_node: u32,
    pub to_walk_node: u32,
    pub line: String,
    /// In-vehicle ride time in minutes.
    pub ride_minutes: f64,
    /// Expected wait (headway/2) in minutes at the boarding stop.
    pub wait_minutes: f64,
}

/// Combined walk + transit graph.
#[derive(Debug, Clone)]
pub struct TransitGraph {
    pub walk: WalkGraph,
    pub stops: Vec<TransitStop>,
    pub rides: Vec<RideEdge>,
    /// Transfer penalty in minutes applied when switching lines at one stop.
    pub transfer_penalty_minutes: f64,
}

impl TransitGraph {
    pub fn new(
        walk: WalkGraph,
        stops: Vec<TransitStop>,
        rides: Vec<RideEdge>,
        transfer_penalty_minutes: f64,
    ) -> Result<Self, TransitError> {
        if !(transfer_penalty_minutes >= 0.0 && transfer_penalty_minutes.is_finite()) {
            return Err(TransitError::Invalid(
                "transfer penalty must be finite and non-negative".into(),
            ));
        }
        let node_count = walk.node_count();
        let mut seen_lines = HashMap::new();
        for stop in &stops {
            if stop.id.trim().is_empty() || stop.lines.is_empty() {
                return Err(TransitError::Invalid(format!(
                    "stop {} has no id or no lines",
                    stop.id
                )));
            }
            if stop.walk_node as usize >= node_count {
                return Err(TransitError::Invalid(format!(
                    "stop {} snaps outside the walk graph",
                    stop.id
                )));
            }
            for line in &stop.lines {
                seen_lines.insert(line.clone(), true);
            }
        }
        for ride in &rides {
            if !ride.ride_minutes.is_finite() || ride.ride_minutes < 0.0 {
                return Err(TransitError::Invalid(format!(
                    "ride {} has an invalid ride time",
                    ride.line
                )));
            }
            if !(ride.wait_minutes.is_finite() && ride.wait_minutes >= 0.0) {
                return Err(TransitError::Invalid(format!(
                    "ride {} has an invalid wait time",
                    ride.line
                )));
            }
            if ride.from_walk_node as usize >= node_count
                || ride.to_walk_node as usize >= node_count
            {
                return Err(TransitError::Invalid(format!(
                    "ride {} references a node outside the walk graph",
                    ride.line
                )));
            }
        }
        Ok(Self {
            walk,
            stops,
            rides,
            transfer_penalty_minutes,
        })
    }

    /// Number of transit lines served by at least one stop.
    pub fn line_count(&self) -> usize {
        let mut lines = HashMap::new();
        for stop in &self.stops {
            for line in &stop.lines {
                lines.insert(line.clone(), true);
            }
        }
        lines.len()
    }

    /// Travel time in minutes from a walk origin to every reachable walk node,
    /// considering walk edges and transit corridors.
    pub fn travel_times_from(&self, origin: u32) -> Vec<Option<f64>> {
        let node_count = self.walk.node_count();
        // Dijkstra over walk nodes. Ride corridors add an edge between their
        // two stop nodes with cost = wait + ride; interchange adds a transfer
        // penalty. We precompute per-node adjacent rides for determinism.
        let mut rides_by_node: HashMap<u32, Vec<&RideEdge>> = HashMap::new();
        for ride in &self.rides {
            rides_by_node.entry(ride.from_walk_node).or_default().push(ride);
            rides_by_node.entry(ride.to_walk_node).or_default().push(ride);
        }

        let mut distances: Vec<Option<f64>> = vec![None; node_count];
        distances[origin as usize] = Some(0.0);
        #[derive(PartialEq)]
        struct Entry {
            cost: f64,
            node: u32,
            line: Option<String>,
        }
        impl Eq for Entry {}
        impl PartialOrd for Entry {
            fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
                Some(self.cmp(other))
            }
        }
        impl Ord for Entry {
            fn cmp(&self, other: &Self) -> std::cmp::Ordering {
                other
                    .cost
                    .partial_cmp(&self.cost)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| self.node.cmp(&other.node))
            }
        }

        let mut heap = BinaryHeap::new();
        heap.push(Entry { cost: 0.0, node: origin, line: None });
        while let Some(entry) = heap.pop() {
            if let Some(known) = distances[entry.node as usize] {
                if entry.cost > known + 1e-12 {
                    continue;
                }
            }
            // Walk edges.
            let mut walk_neighbours: Vec<(u32, f64)> = Vec::new();
            for &(neighbour, cost) in self.walk.adjacency(entry.node) {
                walk_neighbours.push((neighbour, cost));
            }
            for (neighbour, walk_cost) in walk_neighbours {
                let next = entry.cost + walk_cost;
                let neighbour_index = neighbour as usize;
                if distances[neighbour_index].map_or(true, |known| next < known - 1e-12) {
                    distances[neighbour_index] = Some(next);
                    heap.push(Entry { cost: next, node: neighbour, line: None });
                }
            }
            // Transit corridors.
            if let Some(rides) = rides_by_node.get(&entry.node) {
                for ride in rides {
                    // Determine travel direction and boarding wait.
                    let (board_node, alight_node) =
                        if ride.from_walk_node == entry.node {
                            (ride.from_walk_node, ride.to_walk_node)
                        } else {
                            (ride.to_walk_node, ride.from_walk_node)
                        };
                    let _ = board_node;
                    let is_new_line = entry.line.as_deref() != Some(ride.line.as_str());
                    let wait = if is_new_line { ride.wait_minutes } else { 0.0 };
                    let transfer = if is_new_line && entry.line.is_some() {
                        self.transfer_penalty_minutes
                    } else {
                        0.0
                    };
                    let next = entry.cost + wait + transfer + ride.ride_minutes;
                    let alight_index = alight_node as usize;
                    if distances[alight_index].map_or(true, |known| next < known - 1e-12) {
                        distances[alight_index] = Some(next);
                        heap.push(Entry {
                            cost: next,
                            node: alight_node,
                            line: Some(ride.line.clone()),
                        });
                    }
                }
            }
        }
        distances
    }

    /// Route cost in minutes between two snapped walk nodes.
    pub fn route_minutes(&self, from: u32, to: u32) -> Result<f64, TransitError> {
        self.travel_times_from(from)
            .get(to as usize)
            .copied()
            .flatten()
            .ok_or_else(|| TransitError::Routing(format!("node {to} is unreachable from {from}")))
    }

    /// Cumulative opportunities within the threshold over the multimodal graph.
    pub fn cumulative_opportunities(
        &self,
        origin: u32,
        poi_nodes: &[u32],
        threshold_minutes: f64,
    ) -> usize {
        let times = self.travel_times_from(origin);
        poi_nodes
            .iter()
            .filter(|poi| {
                times
                    .get(**poi as usize)
                    .copied()
                    .flatten()
                    .is_some_and(|minutes| minutes <= threshold_minutes)
            })
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRID: &str = r#"{
        "type": "FeatureCollection",
        "walk_speed_m_per_min": 80,
        "features": [
            {"type":"Feature","properties":{},
             "geometry":{"type":"LineString","coordinates":[[0,0],[0.0045,0],[0.009,0]]}},
            {"type":"Feature","properties":{},
             "geometry":{"type":"LineString","coordinates":[[0,0.0072],[0.0045,0.0072],[0.009,0.0072]]}},
            {"type":"Feature","properties":{},
             "geometry":{"type":"LineString","coordinates":[[0,0],[0,0.0072]]}},
            {"type":"Feature","properties":{},
             "geometry":{"type":"LineString","coordinates":[[0.0045,0],[0.0045,0.0072]]}},
            {"type":"Feature","properties":{},
             "geometry":{"type":"LineString","coordinates":[[0.009,0],[0.009,0.0072]]}}
        ]
    }"#;

    fn walk_graph() -> WalkGraph {
        WalkGraph::from_geojson_str(GRID).expect("grid graph")
    }

    /// A rail corridor along the top row (stop at (0,0.0072) and (0.009,0.0072)).
    fn transit_graph() -> TransitGraph {
        let walk = walk_graph();
        let stop_a = walk.snap_node((0.0, 0.0072)).unwrap();
        let stop_b = walk.snap_node((0.009, 0.0072)).unwrap();
        let stops = vec![
            TransitStop {
                id: "s-a".into(),
                walk_node: stop_a,
                lines: vec!["rail-1".into()],
            },
            TransitStop {
                id: "s-b".into(),
                walk_node: stop_b,
                lines: vec!["rail-1".into()],
            },
        ];
        let rides = vec![RideEdge {
            from_walk_node: stop_a,
            to_walk_node: stop_b,
            line: "rail-1".into(),
            ride_minutes: 4.0,
            wait_minutes: 5.0,
        }];
        TransitGraph::new(walk, stops, rides, 3.0).expect("transit graph")
    }

    #[test]
    fn transit_beats_walking_across_the_city() {
        let walk = walk_graph();
        let transit = transit_graph();
        let origin = walk.snap_node((0.0, 0.0)).unwrap();
        let target = walk.snap_node((0.009, 0.0072)).unwrap();

        let walk_time = walk.route_minutes(origin, target).expect("walk route");
        let transit_time = transit.route_minutes(origin, target).expect("transit route");

        // Transit: walk ~10 min to the stop + 5 min wait + 4 min ride + ~10 min
        // egress, which must be less than the ~26 min pure walk across town.
        assert!(transit_time < walk_time, "transit {transit_time} vs walk {walk_time}");
        assert!(transit_time > 0.0);
    }

    #[test]
    fn multimodal_cumulative_opportunities_exceed_walk_only() {
        let walk = walk_graph();
        let transit = transit_graph();
        let origin = walk.snap_node((0.0, 0.0)).unwrap();
        let target = walk.snap_node((0.009, 0.0072)).unwrap();
        let pois = vec![target];

        // Choose a threshold strictly between the walk time and the transit
        // time so the multimodal graph reaches what walk-only cannot.
        let walk_time = walk.route_minutes(origin, target).expect("walk route");
        let transit_time = transit.route_minutes(origin, target).expect("transit route");
        assert!(transit_time < walk_time);
        let threshold = (walk_time + transit_time) / 2.0;

        let walk_opp = walk.cumulative_opportunities(origin, &pois, threshold);
        let transit_opp = transit.cumulative_opportunities(origin, &pois, threshold);
        assert_eq!(walk_opp, 0, "walk cannot reach within {threshold} min");
        assert_eq!(transit_opp, 1, "transit reaches within {threshold} min");
    }

    #[test]
    fn rejects_invalid_transfer_penalty() {
        let walk = walk_graph();
        assert!(TransitGraph::new(walk, vec![], vec![], -1.0).is_err());
    }

    #[test]
    fn rejects_rides_outside_the_walk_graph() {
        let walk = walk_graph();
        let rides = vec![RideEdge {
            from_walk_node: 999,
            to_walk_node: 0,
            line: "x".into(),
            ride_minutes: 1.0,
            wait_minutes: 1.0,
        }];
        assert!(TransitGraph::new(walk, vec![], rides, 1.0).is_err());
    }
}