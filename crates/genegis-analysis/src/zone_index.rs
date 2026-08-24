//! Uniform-grid spatial index over flood zones (real A31a layers carry
//! tens of thousands of polygons; brute-force scanning per sample point is
//! infeasible). Semantics are unchanged for small fixtures: a lookup returns
//! the deepest zone whose polygon contains the point.

use std::collections::HashMap;

use genegis_geometry::{point_in_polygon_parts, PolygonRing};

use crate::flood::FloodZone;
use crate::AnalysisError;

/// Bucket edge in degrees (~1 km at Nagoya's latitude).
const CELL_DEG: f64 = 0.01;

/// Spatially indexed flood-zone layer.
pub struct ZoneIndex {
    zones: Vec<FloodZone>,
    bounds: Vec<[(f64, f64); 2]>,
    buckets: HashMap<(i64, i64), Vec<u32>>,
}

fn ring_bounds(rings: &[PolygonRing]) -> Option<[(f64, f64); 2]> {
    let mut min = (f64::INFINITY, f64::INFINITY);
    let mut max = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for ring in rings {
        for &(x, y) in ring.exterior() {
            min.0 = min.0.min(x);
            min.1 = min.1.min(y);
            max.0 = max.0.max(x);
            max.1 = max.1.max(y);
        }
    }
    if min.0 > max.0 {
        None
    } else {
        Some([min, max])
    }
}

fn cell_of(value: f64) -> i64 {
    (value / CELL_DEG).floor() as i64
}

impl ZoneIndex {
    /// Build the index, failing on empty input.
    pub fn build(zones: Vec<FloodZone>) -> Result<Self, AnalysisError> {
        if zones.is_empty() {
            return Err(AnalysisError::Message("flood zone layer is empty".into()));
        }
        let mut buckets: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
        let mut bounds = Vec::with_capacity(zones.len());
        for (index, zone) in zones.iter().enumerate() {
            let Some(zone_bounds) = ring_bounds(&zone.rings) else {
                continue;
            };
            let (min_cell, max_cell) = (
                (cell_of(zone_bounds[0].0), cell_of(zone_bounds[0].1)),
                (cell_of(zone_bounds[1].0), cell_of(zone_bounds[1].1)),
            );
            for cell_x in min_cell.0..=max_cell.0 {
                for cell_y in min_cell.1..=max_cell.1 {
                    buckets
                        .entry((cell_x, cell_y))
                        .or_default()
                        .push(index as u32);
                }
            }
            bounds.push(zone_bounds);
        }
        Ok(Self {
            zones,
            bounds,
            buckets,
        })
    }

    pub fn len(&self) -> usize {
        self.zones.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.zones.is_empty()
    }

    /// Deepest inundation depth at the point (0.0 when dry).
    pub fn depth_at(&self, point: (f64, f64)) -> f64 {
        let key = (cell_of(point.0), cell_of(point.1));
        let Some(candidates) = self.buckets.get(&key) else {
            return 0.0;
        };
        let mut deepest = 0.0_f64;
        for &index in candidates {
            let bounds = &self.bounds[index as usize];
            if point.0 < bounds[0].0
                || point.0 > bounds[1].0
                || point.1 < bounds[0].1
                || point.1 > bounds[1].1
            {
                continue;
            }
            let zone = &self.zones[index as usize];
            if zone.depth_class_m > deepest && point_in_polygon_parts(point, &zone.rings) {
                deepest = zone.depth_class_m;
            }
        }
        deepest
    }

    /// Zone metadata snapshot (id, name, depth) for evidence payloads.
    pub fn summaries(&self) -> Vec<(String, String, f64)> {
        self.zones
            .iter()
            .map(|zone| (zone.zone_id.clone(), zone.name.clone(), zone.depth_class_m))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flood::DEPTH_BANDS_M;

    fn zone(id: &str, depth: f64, ring: &[(f64, f64)]) -> FloodZone {
        FloodZone {
            zone_id: id.into(),
            name: id.into(),
            depth_class_m: depth,
            rings: vec![PolygonRing::new(ring.to_vec())],
        }
    }

    #[test]
    fn matches_brute_force_on_overlapping_zones() {
        let zones = vec![
            zone("a", 0.5, &[(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)]),
            zone("b", 3.0, &[(1.0, 1.0), (3.0, 1.0), (3.0, 3.0), (1.0, 3.0)]),
            zone("c", 5.0, &[(2.4, 2.4), (2.6, 2.4), (2.6, 2.6), (2.4, 2.6)]),
        ];
        let index = ZoneIndex::build(zones.clone()).expect("index");
        for x in [0.0, 0.5, 1.2, 1.9, 2.5, 2.9, 3.1] {
            for y in [0.0, 0.5, 1.2, 1.9, 2.5, 2.9, 3.1] {
                let brute = zones
                    .iter()
                    .filter(|zone| point_in_polygon_parts((x, y), &zone.rings))
                    .map(|zone| zone.depth_class_m)
                    .fold(0.0_f64, f64::max);
                assert!((index.depth_at((x, y)) - brute).abs() < 1e-12, "at {x},{y}");
            }
        }
    }

    #[test]
    fn rejects_empty_layer() {
        assert!(ZoneIndex::build(Vec::new()).is_err());
        let index = ZoneIndex::build(vec![zone(
            "a",
            DEPTH_BANDS_M[0],
            &[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)],
        )])
        .expect("index");
        assert_eq!(index.len(), 1);
        assert_eq!(index.depth_at((5.0, 5.0)), 0.0);
    }
}
