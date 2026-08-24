//! Isochrone extraction (RFC 0005 platform gap #1 final piece).
//!
//! Given an origin and a travel-time threshold, the reachable node set is
//! wrapped in a convex hull — a planning-level overestimate for grid street
//! networks that stays deterministic, allocation-cheap and monotone in the
//! threshold. Callers must treat `area_m2` as an upper bound; concave
//! alpha-shapes are deferred until a real consumer needs them.

use crate::{NetworkError, WalkGraph};

/// An isochrone polygon around one origin.
#[derive(Debug, Clone, PartialEq)]
pub struct Isochrone {
    pub origin: u32,
    pub threshold_minutes: f64,
    /// Hull vertices in lon/lat, counter-clockwise, first point not repeated.
    pub vertices: Vec<(f64, f64)>,
    /// Nodes reachable within the threshold.
    pub reachable_nodes: usize,
    /// Shoelace area of the hull in square metres.
    pub area_m2: f64,
}

impl WalkGraph {
    /// Compute the convex-hull isochrone around `origin`.
    ///
    /// Fails when the origin has no reachable nodes or the threshold is not
    /// finite and positive.
    pub fn isochrone(
        &self,
        origin: u32,
        threshold_minutes: f64,
    ) -> Result<Isochrone, NetworkError> {
        if !(threshold_minutes.is_finite() && threshold_minutes > 0.0) {
            return Err(NetworkError::Routing(format!(
                "isochrone threshold must be positive and finite, got {threshold_minutes}"
            )));
        }
        let times = self.travel_times_from(origin);
        let reachable: Vec<(f64, f64)> = times
            .iter()
            .zip(self.nodes.iter())
            .filter_map(|(cost, node)| {
                cost.filter(|minutes| *minutes <= threshold_minutes)
                    .map(|_| *node)
            })
            .collect();
        if reachable.is_empty() {
            return Err(NetworkError::Routing(
                "origin reaches no nodes within the threshold".into(),
            ));
        }
        let vertices = convex_hull(reachable);
        let area_m2 = polygon_area_m2(&vertices);
        Ok(Isochrone {
            origin,
            threshold_minutes,
            reachable_nodes: vertices_len_hint(&times, threshold_minutes),
            vertices,
            area_m2,
        })
    }
}

fn vertices_len_hint(times: &[Option<f64>], threshold: f64) -> usize {
    times
        .iter()
        .filter(|cost| cost.is_some_and(|minutes| minutes <= threshold))
        .count()
}

/// Andrew monotone chain hull. Returns counter-clockwise vertices without
/// repeating the first point at the end.
pub fn convex_hull(mut points: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    points.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    });
    points.dedup();
    if points.len() < 3 {
        return points;
    }

    let cross = |o: (f64, f64), a: (f64, f64), b: (f64, f64)| {
        (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
    };

    let mut lower: Vec<(f64, f64)> = Vec::with_capacity(points.len());
    for &point in &points {
        while let [.., b, c] = lower[..] {
            if cross(b, c, point) <= 0.0 {
                lower.pop();
            } else {
                break;
            }
        }
        lower.push(point);
    }
    let mut upper: Vec<(f64, f64)> = Vec::with_capacity(points.len());
    for &point in points.iter().rev() {
        while let [.., b, c] = upper[..] {
            if cross(b, c, point) <= 0.0 {
                upper.pop();
            } else {
                break;
            }
        }
        upper.push(point);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// Shoelace area in square metres using degree-aware metric scaling.
pub fn polygon_area_m2(vertices: &[(f64, f64)]) -> f64 {
    if vertices.len() < 3 {
        return 0.0;
    }
    // Metric scaling evaluated once at the ring's mean latitude.
    let mean_lat = vertices.iter().map(|v| v.1).sum::<f64>() / vertices.len() as f64;
    let lat_scale = 111_320.0_f64;
    let lon_scale = 111_320.0 * mean_lat.to_radians().cos();
    let mut twice_area = 0.0_f64;
    for index in 0..vertices.len() {
        let a = vertices[index];
        let b = vertices[(index + 1) % vertices.len()];
        twice_area += a.0 * lon_scale * (b.1 * lat_scale) - b.0 * lon_scale * (a.1 * lat_scale);
    }
    (twice_area / 2.0).abs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hull_keeps_only_extreme_corners() {
        let points = vec![
            (0.0, 0.0),
            (4.0, 0.0),
            (4.0, 4.0),
            (0.0, 4.0),
            (2.0, 2.0), // interior
            (2.0, 0.5), // edge-adjacent interior
        ];
        let hull = convex_hull(points);
        assert_eq!(hull.len(), 4);
        assert!(hull.contains(&(0.0, 0.0)));
        assert!(hull.contains(&(4.0, 4.0)));
    }

    #[test]
    fn degenerate_inputs_return_vertices_unchanged() {
        assert_eq!(convex_hull(vec![]), Vec::<(f64, f64)>::new());
        assert_eq!(convex_hull(vec![(1.0, 1.0)]), vec![(1.0, 1.0)]);
        assert_eq!(convex_hull(vec![(0.0, 0.0), (1.0, 1.0)]).len(), 2);
    }

    #[test]
    fn shoelace_matches_known_square() {
        let square = vec![(0.0, 0.0), (0.01, 0.0), (0.01, 0.0072), (0.0, 0.0072)];
        let area = polygon_area_m2(&square);
        let width_m = 0.01 * 111_320.0 * 0.0036_f64.to_radians().cos();
        let height_m = 0.0072 * 111_320.0;
        assert!((area - width_m * height_m).abs() / (width_m * height_m) < 1e-6);
    }

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
    fn rejects_bad_thresholds() {
        let graph = graph();
        assert!(graph.isochrone(0, 0.0).is_err());
        assert!(graph.isochrone(0, f64::NAN).is_err());
    }

    #[test]
    fn isochrone_area_grows_monotonically_and_covers_all_nodes() {
        let graph = graph();
        let origin = graph.snap_node((0.0, 0.0)).unwrap();
        let small = graph.isochrone(origin, 16.0).unwrap();
        let mid = graph.isochrone(origin, 25.0).unwrap();
        let full = graph.isochrone(origin, 60.0).unwrap();

        assert!(small.area_m2 > 0.0);
        assert!(small.area_m2 <= mid.area_m2 + 1e-6);
        assert!(mid.area_m2 <= full.area_m2 + 1e-6);
        assert_eq!(full.reachable_nodes, graph.node_count());

        // The full-threshold hull must contain every node: check each lies on
        // or inside the hull by verifying no hull edge crosses to its left.
        for index in 0..graph.node_count() {
            let p = graph.node(index as u32);
            assert!(
                point_in_convex_polygon(p, &full.vertices),
                "node {index} escaped the hull"
            );
        }
    }

    #[test]
    fn origin_node_is_contained() {
        let graph = graph();
        let origin_point = graph.node(graph.snap_node((0.0, 0.0)).unwrap());
        let iso = graph
            .isochrone(graph.snap_node((0.0, 0.0)).unwrap(), 12.0)
            .unwrap();
        assert!(point_in_convex_polygon(origin_point, &iso.vertices));
    }

    fn point_in_convex_polygon(point: (f64, f64), hull: &[(f64, f64)]) -> bool {
        if hull.len() < 3 {
            return false;
        }
        let mut sign = 0.0_f64;
        for index in 0..hull.len() {
            let a = hull[index];
            let b = hull[(index + 1) % hull.len()];
            let cross = (b.0 - a.0) * (point.1 - a.1) - (b.1 - a.1) * (point.0 - a.0);
            if cross.abs() < 1e-15 {
                continue;
            }
            let this = cross.signum();
            if sign == 0.0 {
                sign = this;
            } else if this != sign {
                return false;
            }
        }
        true
    }
}
