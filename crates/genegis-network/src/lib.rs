//! Walk-network routing engine for accessibility analytics (UC-4).
//!
//! Loads a routable graph from GeoJSON LineStrings, snaps arbitrary lon/lat
//! points to graph nodes, and answers shortest-path questions with
//! Dijkstra. Every cost is minutes derived from segment length and the
//! fixture's declared walk speed, so downstream scores are reproducible.

pub mod isochrone;

pub use isochrone::{convex_hull, Isochrone};

use std::collections::BinaryHeap;

use thiserror::Error;

/// Fail-closed routing error.
#[derive(Debug, Error)]
pub enum NetworkError {
    /// Graph source could not be read or parsed.
    #[error("network storage failed: {0}")]
    Storage(String),
    /// The network file violates the expected GeoJSON line structure.
    #[error("invalid walk network: {0}")]
    Invalid(String),
    /// Requested analysis is impossible on this graph.
    #[error("routing failed: {0}")]
    Routing(String),
}

/// A routable walk graph in WGS84 lon/lat.
#[derive(Debug, Clone)]
pub struct WalkGraph {
    nodes: Vec<(f64, f64)>,
    /// Adjacency lists: (neighbour node index, cost minutes).
    adjacency: Vec<Vec<(u32, f64)>>,
    edge_count: usize,
    total_length_m: f64,
    walk_speed_m_per_min: f64,
}

const METERS_PER_DEG_LAT: f64 = 111_320.0;

fn meters_per_deg_lon(lat: f64) -> f64 {
    METERS_PER_DEG_LAT * lat.to_radians().cos()
}

fn segment_length_m(from: (f64, f64), to: (f64, f64)) -> f64 {
    let dlon = (to.0 - from.0).abs() * meters_per_deg_lon((from.1 + to.1) / 2.0);
    let dlat = (to.1 - from.1).abs() * METERS_PER_DEG_LAT;
    (dlon * dlon + dlat * dlat).sqrt()
}

impl WalkGraph {
    /// Build a graph from GeoJSON text containing LineString features.
    ///
    /// Each consecutive coordinate pair becomes an undirected edge; shared
    /// vertices are merged by rounding coordinates to 1e-6 degrees so grid
    /// streets intersect at their crossing points. Edge costs default to
    /// `length / speed` unless a feature declares `walk_min` per segment.
    pub fn from_geojson_str(text: &str) -> Result<Self, NetworkError> {
        let parsed: serde_json::Value =
            serde_json::from_str(text).map_err(|error| NetworkError::Invalid(error.to_string()))?;
        if parsed.get("type").and_then(serde_json::Value::as_str) != Some("FeatureCollection") {
            return Err(NetworkError::Invalid(
                "expected a FeatureCollection of LineString features".into(),
            ));
        }
        let walk_speed = parsed
            .get("walk_speed_m_per_min")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(80.0);
        if walk_speed <= 0.0 {
            return Err(NetworkError::Invalid("non-positive walk speed".into()));
        }
        let features = parsed
            .get("features")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| NetworkError::Invalid("missing features array".into()))?;

        let mut nodes: Vec<(f64, f64)> = Vec::new();
        let mut node_index: std::collections::HashMap<(i64, i64), u32> =
            std::collections::HashMap::new();
        let mut edges: Vec<((u32, u32), f64)> = Vec::new();
        let mut total_length_m = 0.0_f64;
        let quantize = |value: f64| (value * 1e6).round() as i64;

        for feature in features {
            let geometry = feature
                .get("geometry")
                .ok_or_else(|| NetworkError::Invalid("feature without geometry".into()))?;
            if geometry.get("type").and_then(serde_json::Value::as_str) != Some("LineString") {
                continue;
            }
            let coords = geometry
                .get("coordinates")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| NetworkError::Invalid("LineString without coordinates".into()))?;
            let declared_walk_min = feature
                .get("properties")
                .and_then(|props| props.get("walk_min"))
                .and_then(serde_json::Value::as_f64);
            let mut previous: Option<u32> = None;
            let segments = coords.len().saturating_sub(1) as f64;
            for coord in coords {
                let values = coord
                    .as_array()
                    .ok_or_else(|| NetworkError::Invalid("coordinate pair expected".into()))?;
                let lon = values
                    .first()
                    .and_then(serde_json::Value::as_f64)
                    .ok_or_else(|| NetworkError::Invalid("longitude missing".into()))?;
                let lat = values
                    .get(1)
                    .and_then(serde_json::Value::as_f64)
                    .ok_or_else(|| NetworkError::Invalid("latitude missing".into()))?;
                let key = (quantize(lon), quantize(lat));
                let index = match node_index.get(&key) {
                    Some(existing) => *existing,
                    None => {
                        let index = nodes.len() as u32;
                        nodes.push((lon, lat));
                        node_index.insert(key, index);
                        index
                    }
                };
                if let Some(from) = previous {
                    let length = segment_length_m(nodes[from as usize], nodes[index as usize]);
                    let cost = declared_walk_min
                        .map(|total| total / segments.max(1.0))
                        .unwrap_or(length / walk_speed);
                    edges.push(((from, index), cost));
                    total_length_m += length;
                }
                previous = Some(index);
            }
        }

        if nodes.is_empty() || edges.is_empty() {
            return Err(NetworkError::Invalid(
                "network produced no routable nodes or edges".into(),
            ));
        }

        let mut adjacency: Vec<Vec<(u32, f64)>> = vec![Vec::new(); nodes.len()];
        for ((from, to), cost) in &edges {
            adjacency[*from as usize].push((*to, *cost));
            adjacency[*to as usize].push((*from, *cost));
        }

        Ok(Self {
            nodes,
            adjacency,
            edge_count: edges.len(),
            total_length_m,
            walk_speed_m_per_min: walk_speed,
        })
    }

    /// Load a walk graph from a local GeoJSON path.
    pub fn from_geojson_path(path: &str) -> Result<Self, NetworkError> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| NetworkError::Storage(error.to_string()))?;
        Self::from_geojson_str(&text)
    }

    /// Number of merged intersection nodes.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Number of undirected street segments.
    pub fn edge_count(&self) -> usize {
        self.edge_count
    }

    /// Declared walking speed in metres per minute.
    pub fn walk_speed_m_per_min(&self) -> f64 {
        self.walk_speed_m_per_min
    }

    /// Total encoded street length in kilometres.
    pub fn total_length_km(&self) -> f64 {
        self.total_length_m / 1000.0
    }

    /// Index of the nearest graph node using planar degree distance.
    ///
    /// Degree-space snapping keeps the comparison isotropic enough at city
    /// scale while avoiding trigonometry per candidate.
    pub fn snap_node(&self, point: (f64, f64)) -> Result<u32, NetworkError> {
        let lat_scale = meters_per_deg_lon(point.1).max(1e-9);
        let (px, py) = (point.0 * lat_scale, point.1 * METERS_PER_DEG_LAT);
        let (mut best, mut best_distance) = (None, f64::INFINITY);
        for (index, &(lon, lat)) in self.nodes.iter().enumerate() {
            let dx = lon * lat_scale - px;
            let dy = lat * METERS_PER_DEG_LAT - py;
            let distance = dx * dx + dy * dy;
            if distance < best_distance {
                best_distance = distance;
                best = Some(index as u32);
            }
        }
        best.ok_or_else(|| NetworkError::Routing("graph has no nodes".into()))
    }

    /// Straight-line distance in metres between two lon/lat points.
    pub fn euclidean_distance_m(from: (f64, f64), to: (f64, f64)) -> f64 {
        segment_length_m(from, to)
    }

    /// Node coordinates by index.
    pub fn node(&self, index: u32) -> (f64, f64) {
        self.nodes[index as usize]
    }

    /// Dijkstra travel-time (minutes) from one origin to every reachable node.
    pub fn travel_times_from(&self, origin: u32) -> Vec<Option<f64>> {
        #[derive(PartialEq)]
        struct QueueEntry {
            cost: f64,
            node: u32,
        }
        impl Eq for QueueEntry {}
        impl PartialOrd for QueueEntry {
            fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
                Some(self.cmp(other))
            }
        }
        impl Ord for QueueEntry {
            fn cmp(&self, other: &Self) -> std::cmp::Ordering {
                other
                    .cost
                    .partial_cmp(&self.cost)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| self.node.cmp(&other.node))
            }
        }

        let mut distances: Vec<Option<f64>> = vec![None; self.nodes.len()];
        distances[origin as usize] = Some(0.0);
        let mut heap = BinaryHeap::new();
        heap.push(QueueEntry {
            cost: 0.0,
            node: origin,
        });
        while let Some(entry) = heap.pop() {
            if let Some(known) = distances[entry.node as usize] {
                if entry.cost > known {
                    continue;
                }
            }
            for &(neighbour, edge_cost) in &self.adjacency[entry.node as usize] {
                let next = entry.cost + edge_cost;
                let neighbour_index = neighbour as usize;
                if distances[neighbour_index].map_or(true, |known| next < known) {
                    distances[neighbour_index] = Some(next);
                    heap.push(QueueEntry {
                        cost: next,
                        node: neighbour,
                    });
                }
            }
        }
        distances
    }

    /// Shortest-path cost in minutes between two snapped nodes.
    pub fn route_minutes(&self, from: u32, to: u32) -> Result<f64, NetworkError> {
        self.travel_times_from(from)
            .get(to as usize)
            .copied()
            .flatten()
            .ok_or_else(|| NetworkError::Routing(format!("node {to} is unreachable from {from}")))
    }

    /// Count of supplied POI nodes within `threshold_minutes` of the origin.
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

    /// Scale every undirected edge cost by a hazard factor derived from its
    /// endpoints (UC-1 evacuation): `cost *= factor(from, to)`.
    ///
    /// Factors must be finite and non-negative; factors ≥ 1 model slowed or
    /// detoured movement through hazard zones without breaking Dijkstra.
    /// Returns the number of undirected edges whose factor exceeded 1.0.
    pub fn scale_edge_costs<F>(&mut self, mut factor: F) -> Result<usize, NetworkError>
    where
        F: FnMut((f64, f64), (f64, f64)) -> f64,
    {
        let mut scaled = 0_usize;
        for from_index in 0..self.adjacency.len() {
            let from = self.nodes[from_index];
            for slot in &mut self.adjacency[from_index] {
                let to = self.nodes[slot.0 as usize];
                let factor = factor(from, to);
                if !factor.is_finite() || factor < 0.0 {
                    return Err(NetworkError::Invalid(format!(
                        "edge cost factor must be finite and non-negative, got {factor}"
                    )));
                }
                if factor > 1.0 + 1e-12 && slot.0 as usize > from_index {
                    scaled += 1;
                }
                slot.1 *= factor;
            }
        }
        Ok(scaled)
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

    fn graph() -> WalkGraph {
        WalkGraph::from_geojson_str(GRID).expect("grid graph")
    }

    #[test]
    fn merges_shared_vertices_into_a_grid_graph() {
        let graph = graph();
        // 3 columns x 2 rows of intersections
        assert_eq!(graph.node_count(), 6);
        // 2 horizontals (2 segments each) + 3 verticals (1 segment each)
        assert_eq!(graph.edge_count(), 7);
        // Horizontals sit at lat 0/0.0072 (avg ~0.0036 deg), verticals at
        // lon 0..0.009; compute expected length from the engine's own model.
        let m_per_deg_lon = WalkGraph::euclidean_distance_m((0.0, 0.0), (1.0, 0.0));
        let horizontal_km = 2.0 * 2.0 * 0.0045 * m_per_deg_lon / 1000.0;
        let vertical_km = 3.0 * 0.0072 * METERS_PER_DEG_LAT / 1000.0;
        assert!((graph.total_length_km() - (horizontal_km + vertical_km)).abs() < 0.01);
    }

    #[test]
    fn routes_along_grid_with_monotonic_costs() {
        let graph = graph();
        let origin = graph.snap_node((0.0, 0.0)).unwrap();
        let target = graph.snap_node((0.009, 0.0072)).unwrap();
        let minutes = graph.route_minutes(origin, target).expect("route");
        // manhattan distance ~ 0.009 deg lon (~800m) + 0.0072 deg lat (~800m)
        assert!(minutes > 18.0 && minutes < 24.0, "minutes={minutes}");
        let closer = graph
            .route_minutes(origin, graph.snap_node((0.0045, 0.0)).unwrap())
            .expect("route");
        assert!(closer < minutes);
    }

    #[test]
    fn triangle_inequality_holds_on_sampled_triples() {
        let graph = graph();
        let nodes: Vec<u32> = (0..graph.node_count() as u32).collect();
        for a in &nodes {
            for b in &nodes {
                let ab = graph.route_minutes(*a, *b).expect("reachable");
                for c in &nodes {
                    let bc = graph.route_minutes(*b, *c).expect("reachable");
                    let ac = graph.route_minutes(*a, *c).expect("reachable");
                    assert!(
                        ac <= ab + bc + 1e-9,
                        "triangle violated: {ac} > {ab} + {bc}"
                    );
                }
            }
        }
    }

    #[test]
    fn cumulative_opportunities_respect_threshold_monotonicity() {
        let graph = graph();
        let origin = graph.snap_node((0.0, 0.0)).unwrap();
        let pois = vec![
            graph.snap_node((0.0045, 0.0)).unwrap(),
            graph.snap_node((0.0, 0.0072)).unwrap(),
            graph.snap_node((0.009, 0.0072)).unwrap(),
        ];
        let at_5 = graph.cumulative_opportunities(origin, &pois, 5.0);
        let at_10 = graph.cumulative_opportunities(origin, &pois, 10.0);
        let at_30 = graph.cumulative_opportunities(origin, &pois, 30.0);
        assert!(at_5 <= at_10);
        assert!(at_10 <= at_30);
        assert_eq!(at_30, pois.len());
    }

    #[test]
    fn rejects_empty_and_malformed_networks() {
        assert!(matches!(
            WalkGraph::from_geojson_str("{\"type\":\"FeatureCollection\",\"features\":[]}"),
            Err(NetworkError::Invalid(_))
        ));
        assert!(matches!(
            WalkGraph::from_geojson_str("not json"),
            Err(NetworkError::Invalid(_))
        ));
    }

    #[test]
    fn uniform_scaling_multiplies_all_routes() {
        let graph = graph();
        let origin = graph.snap_node((0.0, 0.0)).unwrap();
        let target = graph.snap_node((0.009, 0.0072)).unwrap();
        let baseline = graph.route_minutes(origin, target).unwrap();

        let mut penalized = graph.clone();
        let scaled = penalized.scale_edge_costs(|_, _| 2.0).unwrap();
        assert_eq!(scaled, graph.edge_count());
        let after = penalized.route_minutes(origin, target).unwrap();
        assert!((after - baseline * 2.0).abs() < 1e-9);
    }

    #[test]
    fn flood_penalty_pushes_routes_onto_clean_detours() {
        // Penalize the middle north-south column ten-fold; a detour through
        // the outer columns must beat the direct flooded segment.
        let mut graph = graph();
        let scaled = graph
            .scale_edge_costs(|from, to| {
                if (from.0 - to.0).abs() < 1e-9 && (from.0 - 0.0045).abs() < 1e-9 {
                    10.0
                } else {
                    1.0
                }
            })
            .unwrap();
        assert_eq!(scaled, 1, "only the middle column segment is flooded");

        let origin = graph.snap_node((0.0045, 0.0)).unwrap();
        let target = graph.snap_node((0.0045, 0.0072)).unwrap();
        let baseline = WalkGraph::from_geojson_str(GRID)
            .unwrap()
            .route_minutes(origin, target)
            .unwrap();
        let flooded = graph.route_minutes(origin, target).unwrap();
        assert!(flooded > baseline, "flooded route must cost more");
        // Detour exists: two clean sides of a grid cell (~30 min) instead of
        // the penalized column (~100 min).
        assert!(flooded < baseline * 10.0 - 1.0);
    }

    #[test]
    fn penalties_never_shorten_any_route() {
        let baseline = graph();
        let mut penalized = baseline.clone();
        penalized
            .scale_edge_costs(|from, to| 1.0 + ((from.0 + to.0) * 1000.0).fract().abs().max(0.0))
            .unwrap();
        for a in 0..baseline.node_count() as u32 {
            let clean = baseline.travel_times_from(a);
            let slow = penalized.travel_times_from(a);
            for (c, s) in clean.iter().zip(slow.iter()) {
                if let (Some(c), Some(s)) = (c, s) {
                    assert!(*s >= *c - 1e-9, "{s} < {c}");
                }
            }
        }
    }

    #[test]
    fn rejects_non_finite_factors() {
        let mut graph = graph();
        assert!(matches!(
            graph.scale_edge_costs(|_, _| f64::NAN),
            Err(NetworkError::Invalid(_))
        ));
        assert!(matches!(
            graph.scale_edge_costs(|_, _| -1.0),
            Err(NetworkError::Invalid(_))
        ));
    }
}
