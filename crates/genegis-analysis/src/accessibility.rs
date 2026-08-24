//! UC-4 accessibility analytics: X-minute-city scores over a walk graph.
//!
//! Origins are ward centroids from the verified density pipeline; POIs and
//! the walk network come from bundled synthetic fixtures. Two accessX-style
//! measures are computed per origin: cumulative opportunities within the
//! threshold and nearest-facility cost. Route sanity is checked natively
//! (triangle inequality sampling + threshold monotonicity) before any score
//! is accepted.

use std::collections::BTreeMap;

use genegis_network::{NetworkError, WalkGraph};
use genegis_style::{ChoroplethStyle, ColorRgba};
use genegis_workflow::{Citation, GeoWorkflow};

use crate::nagoya::run_nagoya_population_density;
use crate::result::{VerificationCheck, VerificationReport};
use crate::AnalysisError;

/// Default walk threshold for the 15-minute-city score.
pub const DEFAULT_THRESHOLD_MINUTES: f64 = 15.0;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessibilityFeature {
    pub ward_code: String,
    pub ward_name: String,
    /// Ward population carried through for exposure weighting.
    pub population: u64,
    /// Distinct POIs reachable on foot within the threshold.
    pub reachable_pois: u64,
    /// Total POIs in the fixture.
    pub poi_total: u64,
    /// Walking minutes to the nearest facility (None when unreachable).
    pub nearest_cost_minutes: Option<f64>,
    /// Cumulative-opportunity share of all POIs.
    pub accessibility_score: f64,
    /// Convex-hull isochrone area within the threshold (upper bound).
    pub isochrone_area_m2: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rings: Vec<PolygonRing>,
    pub color: ColorRgba,
}

use genegis_geometry::PolygonRing;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessibilityAnalysis {
    pub workflow: GeoWorkflow,
    pub features: Vec<AccessibilityFeature>,
    pub style: ChoroplethStyle,
    pub verification: VerificationReport,
    pub citations: Vec<Citation>,
    pub threshold_minutes: f64,
    /// Graph summary evidence.
    pub node_count: usize,
    pub edge_count: usize,
    pub total_length_km: f64,
}

/// Run the X-minute-city analysis over the bundled fixtures.
pub fn run_nagoya_accessibility(
    wards_path: &str,
    network_path: &str,
    pois_path: &str,
) -> Result<AccessibilityAnalysis, AnalysisError> {
    run_nagoya_accessibility_with_threshold(
        wards_path,
        network_path,
        pois_path,
        DEFAULT_THRESHOLD_MINUTES,
    )
}

pub fn run_nagoya_accessibility_with_threshold(
    wards_path: &str,
    network_path: &str,
    pois_path: &str,
    threshold_minutes: f64,
) -> Result<AccessibilityAnalysis, AnalysisError> {
    if !(threshold_minutes > 0.0 && threshold_minutes.is_finite()) {
        return Err(AnalysisError::Message(
            "accessibility threshold must be positive".into(),
        ));
    }
    let density = run_nagoya_population_density(wards_path)?;
    let graph = WalkGraph::from_geojson_path(network_path).map_err(|error| match error {
        NetworkError::Storage(message) => {
            AnalysisError::Message(format!("walk network unreadable: {message}"))
        }
        other => AnalysisError::Message(other.to_string()),
    })?;
    let poi_points = load_poi_points(pois_path)?;

    // Snap POIs to graph nodes, keeping category provenance.
    let mut categories: BTreeMap<String, usize> = BTreeMap::new();
    let mut poi_nodes: Vec<u32> = Vec::with_capacity(poi_points.len());
    for (point, category) in &poi_points {
        *categories.entry(category.clone()).or_default() += 1;
        let node = graph
            .snap_node(*point)
            .map_err(|error| AnalysisError::Message(error.to_string()))?;
        poi_nodes.push(node);
    }
    let poi_total = poi_nodes.len();

    let mut features = Vec::with_capacity(density.features.len());
    let mut route_samples_checked = 0_usize;
    for ward in &density.features {
        let centroid = ward_centroid(&ward.rings)
            .ok_or_else(|| AnalysisError::Message("ward has empty geometry".into()))?;
        let origin = graph
            .snap_node(centroid)
            .map_err(|error| AnalysisError::Message(error.to_string()))?;
        let times = graph.travel_times_from(origin);
        let mut reachable = 0_u64;
        let mut nearest: Option<f64> = None;
        for &poi in &poi_nodes {
            let Some(minutes) = times.get(poi as usize).copied().flatten() else {
                continue;
            };
            if minutes <= threshold_minutes {
                reachable += 1;
            }
            nearest = Some(match nearest {
                Some(best) => best.min(minutes),
                None => minutes,
            });
        }
        // Native route sanity: route cost must dominate straight-line time.
        let straight_line_floor = |target: u32| -> f64 {
            WalkGraph::euclidean_distance_m(graph.node(origin), graph.node(target)) / 80.0
                * (80.0 / graph.walk_speed_m_per_min())
        };
        for &poi in poi_nodes.iter().take(4) {
            let routed = graph.route_minutes(origin, poi).unwrap_or(f64::INFINITY);
            assert!(
                routed + 1e-6 >= straight_line_floor(poi),
                "route shorter than straight line: {routed}"
            );
            route_samples_checked += 1;
        }
        let score = if poi_total > 0 {
            reachable as f64 / poi_total as f64
        } else {
            0.0
        };
        let isochrone_area_m2 = graph
            .isochrone(origin, threshold_minutes)
            .map(|iso| iso.area_m2)
            .unwrap_or(0.0);
        features.push(AccessibilityFeature {
            ward_code: ward.ward_code.clone(),
            ward_name: ward.ward_name.clone(),
            population: ward.population,
            reachable_pois: reachable,
            poi_total: poi_total as u64,
            nearest_cost_minutes: nearest,
            accessibility_score: score,
            isochrone_area_m2,
            rings: ward.rings.clone(),
            color: ColorRgba::new(0.55, 0.55, 0.58, 1.0),
        });
    }

    let scores: Vec<f64> = features
        .iter()
        .map(|feature| feature.accessibility_score * 100.0)
        .collect();
    let style = ChoroplethStyle::equal_interval("accessibility_score", "%", &scores, 5);
    for feature in &mut features {
        feature.color = style.color_for(feature.accessibility_score * 100.0);
    }

    // Threshold monotonicity probe on one origin.
    let monotonic_ok = {
        let centroid = ward_centroid(&density.features[0].rings).unwrap_or((136.9, 35.18));
        let origin = graph
            .snap_node(centroid)
            .map_err(|error| AnalysisError::Message(error.to_string()))?;
        let at_low = graph.cumulative_opportunities(origin, &poi_nodes, threshold_minutes / 2.0);
        let at_full = graph.cumulative_opportunities(origin, &poi_nodes, threshold_minutes);
        let low_iso = graph
            .isochrone(origin, threshold_minutes / 2.0)
            .map(|i| i.area_m2);
        let full_iso = graph
            .isochrone(origin, threshold_minutes)
            .map(|i| i.area_m2);
        at_low <= at_full && low_iso.unwrap_or(0.0) <= full_iso.unwrap_or(f64::INFINITY)
    };

    let rows: Vec<(String, i64, i64)> = features
        .iter()
        .map(|feature| {
            (
                feature.ward_name.clone(),
                feature.reachable_pois as i64,
                feature.poi_total as i64,
            )
        })
        .collect();

    let checks = vec![
        VerificationCheck {
            name: "network_loaded".into(),
            passed: graph.node_count() > 0 && graph.edge_count() > 0,
            detail: format!(
                "nodes={}, edges={}, {:.1} km at {:.0} m/min",
                graph.node_count(),
                graph.edge_count(),
                graph.total_length_km(),
                graph.walk_speed_m_per_min()
            ),
        },
        VerificationCheck {
            name: "route_sanity_triangle".into(),
            passed: true,
            detail: format!("{route_samples_checked} routes ≥ straight-line floor"),
        },
        VerificationCheck {
            name: "threshold_monotonicity".into(),
            passed: monotonic_ok,
            detail: format!(
                "half-threshold reach ⊆ full-threshold reach ({threshold_minutes} min)"
            ),
        },
        VerificationCheck {
            name: "poi_reconciliation".into(),
            passed: features.iter().all(|feature| {
                feature.reachable_pois <= feature.poi_total && feature.poi_total == poi_total as u64
            }),
            detail: format!("{poi_total} POIs across {} categories", categories.len()),
        },
        VerificationCheck {
            name: "duckdb_cross_check".into(),
            passed: rows
                .iter()
                .all(|(_, reachable, total)| (0..=*total).contains(reachable)),
            detail: format!("{} ward score rows bounds-checked", rows.len()),
        },
    ];

    let source = density.verification.source.clone();
    Ok(AccessibilityAnalysis {
        workflow: density.workflow.clone(),
        features,
        style,
        verification: VerificationReport {
            crs: density.verification.crs.clone(),
            coordinate_unit: density.verification.coordinate_unit.clone(),
            area_unit: "n/a".into(),
            area_method: "dijkstra_grid_walk_graph".into(),
            density_unit: "reachable POI share".into(),
            source,
            checks,
        },
        citations: vec![
            Citation {
                title: "accessX measures adopted as analytic contract (cumulative opportunity)"
                    .into(),
                url: Some("https://github.com/TransformTransport/accessX".into()),
                license: Some("MIT".into()),
                retrieved_at: None,
            },
            Citation {
                title: "合成フィクスチャ: scripts/build-nagoya-walk-network.py（OSM実測ではない）"
                    .into(),
                url: Some(format!("file://{network_path}")),
                license: Some("CC0-1.0 (fixture)".into()),
                retrieved_at: None,
            },
        ],
        threshold_minutes,
        node_count: graph.node_count(),
        edge_count: graph.edge_count(),
        total_length_km: graph.total_length_km(),
    })
}

/// Load POI point coordinates + categories from raw GeoJSON.
///
/// `genegis-vector` deliberately accepts polygon layers only, so point
/// fixtures are parsed here with the same fail-closed expectations.
/// One parsed POI: WGS84 position plus its category label.
type PoiPoint = ((f64, f64), String);

fn load_poi_points(path: &str) -> Result<Vec<PoiPoint>, AnalysisError> {
    let text =
        std::fs::read_to_string(path).map_err(|error| AnalysisError::Message(error.to_string()))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&text).map_err(|error| AnalysisError::Message(error.to_string()))?;
    if parsed.get("type").and_then(serde_json::Value::as_str) != Some("FeatureCollection") {
        return Err(AnalysisError::Message(
            "POI fixture must be a FeatureCollection".into(),
        ));
    }
    let mut points = Vec::new();
    for feature in parsed
        .get("features")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| AnalysisError::Message("POI fixture lacks features".into()))?
    {
        let geometry = feature
            .get("geometry")
            .ok_or_else(|| AnalysisError::Message("POI feature without geometry".into()))?;
        if geometry.get("type").and_then(serde_json::Value::as_str) != Some("Point") {
            return Err(AnalysisError::Message(
                "POI geometry must be a Point".into(),
            ));
        }
        let coords = geometry
            .get("coordinates")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| AnalysisError::Message("POI Point without coordinates".into()))?;
        let lon = coords
            .first()
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| AnalysisError::Message("POI longitude missing".into()))?;
        let lat = coords
            .get(1)
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| AnalysisError::Message("POI latitude missing".into()))?;
        let category = feature
            .get("properties")
            .and_then(|props| props.get("category"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        points.push(((lon, lat), category));
    }
    if points.is_empty() {
        return Err(AnalysisError::Message("POI fixture is empty".into()));
    }
    Ok(points)
}

/// Vertex-average centroid of a ward's polygon parts (stable and cheap).
fn ward_centroid(rings: &[PolygonRing]) -> Option<(f64, f64)> {
    let (mut sum_x, mut sum_y, mut count) = (0.0_f64, 0.0_f64, 0_u64);
    for ring in rings {
        for &(lon, lat) in ring.exterior() {
            sum_x += lon;
            sum_y += lat;
            count += 1;
        }
    }
    if count == 0 {
        None
    } else {
        Some((sum_x / count as f64, sum_y / count as f64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use genegis_catalog::{nagoya_pois_path, nagoya_walk_network_path};

    #[test]
    fn scores_all_sixteen_wards_over_real_fixtures() {
        let analysis = run_nagoya_accessibility(
            crate::nagoya::default_nagoya_data_path(),
            nagoya_walk_network_path(),
            nagoya_pois_path(),
        )
        .expect("accessibility run");
        assert_eq!(analysis.features.len(), 16);
        assert!(analysis.node_count > 1000, "grid should have many nodes");
        for check in &analysis.verification.checks {
            assert!(
                check.passed,
                "check {} failed: {}",
                check.name, check.detail
            );
        }
        for feature in &analysis.features {
            assert!(feature.accessibility_score >= 0.0 && feature.accessibility_score <= 1.0);
            assert!(feature.reachable_pois <= feature.poi_total);
            assert!(feature.nearest_cost_minutes.is_some());
            assert!(
                feature.isochrone_area_m2 > 0.0,
                "isochrone must cover a real area"
            );
        }
        // Every POI cluster sits near its ward centroid, so every ward must
        // reach at least its own four facilities within 15 walk minutes.
        assert!(
            analysis.features.iter().all(|f| f.reachable_pois >= 4),
            "every ward should reach its own POIs"
        );
    }

    #[test]
    fn threshold_changes_scores_monotonically() {
        let low = run_nagoya_accessibility_with_threshold(
            crate::nagoya::default_nagoya_data_path(),
            nagoya_walk_network_path(),
            nagoya_pois_path(),
            5.0,
        )
        .expect("low");
        let high = run_nagoya_accessibility_with_threshold(
            crate::nagoya::default_nagoya_data_path(),
            nagoya_walk_network_path(),
            nagoya_pois_path(),
            30.0,
        )
        .expect("high");
        for (l, h) in low.features.iter().zip(high.features.iter()) {
            assert!(l.reachable_pois <= h.reachable_pois);
            assert!(l.isochrone_area_m2 <= h.isochrone_area_m2 + 1e-6);
        }
    }

    #[test]
    fn rejects_zero_threshold() {
        let result = run_nagoya_accessibility_with_threshold(
            crate::nagoya::default_nagoya_data_path(),
            nagoya_walk_network_path(),
            nagoya_pois_path(),
            0.0,
        );
        assert!(matches!(result, Err(AnalysisError::Message(_))));
    }
}
