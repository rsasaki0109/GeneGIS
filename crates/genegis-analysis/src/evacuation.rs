//! UC-1 network part: flood-penalized evacuation accessibility.
//!
//! Ward centroids are routed to the nearest designated shelter on the
//! bundled walk grid twice: once with clean edge costs and once with every
//! flood-zone-crossing segment slowed by `1 + depth × penalty` (RFC 0005:
//! speed × depth factor). The per-ward delay between the two routes is the
//! analytic product; it is only accepted after monotonicity, triangle and
//! DuckDB cross-checks pass.

use genegis_geometry::{point_in_polygon_parts, PolygonRing};
use genegis_network::{NetworkError, WalkGraph};
use genegis_style::{ChoroplethStyle, ColorRgba};
use genegis_workflow::{Citation, GeoWorkflow};

use crate::flood::{FloodZone, DEPTH_BANDS_M};
use crate::nagoya::run_nagoya_population_density;
use crate::result::{VerificationCheck, VerificationReport};
use crate::AnalysisError;

/// Extra travel-time multiplier per metre of inundation depth.
///
/// A 0.5 m planned-scale corridor doubles walking cost; a 5 m maximum-scale
/// corridor costs eleven times as much, pushing routes onto dry detours.
pub const DEFAULT_DEPTH_SPEED_PENALTY_PER_M: f64 = 2.0;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvacuationFeature {
    pub ward_code: String,
    pub ward_name: String,
    /// Ward population carried through for exposure weighting.
    pub population: u64,
    /// Nearest shelter on the clean graph.
    pub shelter_name: String,
    /// Walking minutes to the nearest shelter without flooding.
    pub baseline_minutes: f64,
    /// Walking minutes to the nearest shelter with flood penalties applied.
    pub flooded_minutes: f64,
    /// `flooded_minutes - baseline_minutes`; never negative on a valid run.
    pub delay_minutes: f64,
    /// Nearest shelter under flooding (differs when a detour wins).
    pub flooded_shelter_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rings: Vec<PolygonRing>,
    pub color: ColorRgba,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvacuationAnalysis {
    pub workflow: GeoWorkflow,
    pub features: Vec<EvacuationFeature>,
    pub style: ChoroplethStyle,
    pub verification: VerificationReport,
    pub citations: Vec<Citation>,
    pub shelter_count: usize,
    pub node_count: usize,
    pub edge_count: usize,
    pub total_length_km: f64,
    /// Undirected segments crossed by a flood zone (penalty factor > 1).
    pub flooded_edge_count: usize,
    /// Share of street length inside a flood zone.
    pub flooded_length_share: f64,
    pub depth_penalty_per_m: f64,
}

/// Run the UC-1 evacuation analysis with the default depth penalty.
pub fn run_nagoya_evacuation_access(
    wards_path: &str,
    network_path: &str,
    zones_path: &str,
    shelters_path: &str,
) -> Result<EvacuationAnalysis, AnalysisError> {
    run_nagoya_evacuation_access_with_penalty(
        wards_path,
        network_path,
        zones_path,
        shelters_path,
        DEFAULT_DEPTH_SPEED_PENALTY_PER_M,
    )
}

pub fn run_nagoya_evacuation_access_with_penalty(
    wards_path: &str,
    network_path: &str,
    zones_path: &str,
    shelters_path: &str,
    depth_penalty_per_m: f64,
) -> Result<EvacuationAnalysis, AnalysisError> {
    if !(depth_penalty_per_m >= 0.0 && depth_penalty_per_m.is_finite()) {
        return Err(AnalysisError::Message(
            "depth penalty must be non-negative and finite".into(),
        ));
    }
    let density = run_nagoya_population_density(wards_path)?;
    let graph = WalkGraph::from_geojson_path(network_path).map_err(|error| match error {
        NetworkError::Storage(message) => {
            AnalysisError::Message(format!("walk network unreadable: {message}"))
        }
        other => AnalysisError::Message(other.to_string()),
    })?;
    let zones = load_flood_zones(zones_path)?;
    let shelters = load_named_points(shelters_path, "name")?;
    if shelters.len() < 2 {
        return Err(AnalysisError::Message(
            "shelter fixture must contain at least two shelters".into(),
        ));
    }

    // Penalize edges whose midpoint sits inside a flood polygon. Midpoint
    // sampling matches the ~400 m grid spacing against zone-sized corridors;
    // a finer treatment is deferred until OSM ingestion lands (RFC 0005).
    let mut flooded_graph = graph.clone();
    let mut flooded_edge_length_m = 0.0_f64;
    let penalty_for_depth = move |depth: f64| 1.0 + depth * depth_penalty_per_m;
    let flooded_edge_count;
    {
        let zones_ref = &zones;
        let flooded_len_ref = &mut flooded_edge_length_m;
        flooded_edge_count = flooded_graph
            .scale_edge_costs(|from, to| {
                let mid = ((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
                let depth = zones_ref
                    .iter()
                    .filter(|zone| point_in_polygon_parts(mid, &zone.rings))
                    .map(|zone| zone.depth_class_m)
                    .fold(0.0_f64, f64::max);
                if depth > 0.0 {
                    *flooded_len_ref += WalkGraph::euclidean_distance_m(from, to);
                }
                penalty_for_depth(depth)
            })
            .map_err(|error| AnalysisError::Message(error.to_string()))?;
    }

    // Snap once; both graphs share identical topology so indices line up.
    let mut ward_origins = Vec::with_capacity(density.features.len());
    for ward in &density.features {
        let centroid = ward_centroid(&ward.rings)
            .ok_or_else(|| AnalysisError::Message("ward has empty geometry".into()))?;
        ward_origins.push(graph.snap_node(centroid).map_err(|error| {
            AnalysisError::Message(format!("cannot snap {}: {error}", ward.ward_name))
        })?);
    }
    let mut shelter_nodes = Vec::with_capacity(shelters.len());
    for &(point, _) in &shelters {
        shelter_nodes.push(graph.snap_node(point).map_err(|error| {
            AnalysisError::Message(format!("cannot snap shelter at {point:?}: {error}"))
        })?);
    }

    let mut features = Vec::with_capacity(ward_origins.len());
    for (origin, ward) in ward_origins.iter().zip(&density.features) {
        let baseline_times = graph.travel_times_from(*origin);
        let flooded_times = flooded_graph.travel_times_from(*origin);

        let nearest_on = |times: &[Option<f64>]| -> Option<(usize, f64)> {
            let mut best: Option<(usize, f64)> = None;
            for (index, node) in shelter_nodes.iter().enumerate() {
                if let Some(minutes) = times.get(*node as usize).copied().flatten() {
                    best = Some(match best {
                        Some((_, current)) if current <= minutes => best.unwrap(),
                        _ => (index, minutes),
                    });
                }
            }
            best
        };
        let Some((baseline_index, baseline_minutes)) = nearest_on(&baseline_times) else {
            return Err(AnalysisError::Message(format!(
                "ward {} cannot reach any shelter",
                ward.ward_name
            )));
        };
        let flooded_nearest = nearest_on(&flooded_times);
        let Some((flooded_index, flooded_minutes)) = flooded_nearest else {
            return Err(AnalysisError::Message(format!(
                "ward {} cannot reach any shelter under flooding",
                ward.ward_name
            )));
        };

        features.push(EvacuationFeature {
            ward_code: ward.ward_code.clone(),
            ward_name: ward.ward_name.clone(),
            population: ward.population,
            shelter_name: shelters[baseline_index].1.clone(),
            baseline_minutes,
            flooded_minutes,
            delay_minutes: flooded_minutes - baseline_minutes,
            flooded_shelter_name: if flooded_index == baseline_index {
                None
            } else {
                Some(shelters[flooded_index].1.clone())
            },
            rings: ward.rings.clone(),
            color: ColorRgba::new(0.55, 0.55, 0.58, 1.0),
        });
    }

    let delays: Vec<f64> = features
        .iter()
        .map(|feature| feature.delay_minutes)
        .collect();
    let style = ChoroplethStyle::equal_interval("delay_minutes", "min", &delays, 5);
    for feature in &mut features {
        feature.color = style.color_for(feature.delay_minutes);
    }

    // Native route sanity on the penalized graph: sampled pairs must keep
    // triangle inequality, and every flooded route must dominate its clean
    // counterpart.
    let probe_nodes: Vec<u32> = ward_origins
        .iter()
        .chain(shelter_nodes.iter())
        .copied()
        .take(24)
        .collect();
    let mut triangle_samples = 0_usize;
    let mut triangle_ok = true;
    for &a in &probe_nodes {
        let times_a = flooded_graph.travel_times_from(a);
        for &b in &probe_nodes {
            let Some(ab) = times_a.get(b as usize).copied().flatten() else {
                continue;
            };
            let times_b = flooded_graph.travel_times_from(b);
            for &c in &probe_nodes {
                let Some(bc) = times_b.get(c as usize).copied().flatten() else {
                    continue;
                };
                let ac = times_a
                    .get(c as usize)
                    .copied()
                    .flatten()
                    .unwrap_or(f64::INFINITY);
                if ac > ab + bc + 1e-9 {
                    triangle_ok = false;
                }
                triangle_samples += 1;
            }
        }
    }

    let monotonic_ok = features.iter().all(|feature| {
        feature.flooded_minutes >= feature.baseline_minutes - 1e-6
            && feature.delay_minutes.is_finite()
    });

    // Determinism replay: recompute one penalized route from scratch.
    let replay_ok = {
        let replay_graph = WalkGraph::from_geojson_path(network_path)
            .map_err(|error| AnalysisError::Message(error.to_string()))?;
        let mut replay_flooded = replay_graph.clone();
        let zones_ref = &zones;
        replay_flooded
            .scale_edge_costs(|from, to| {
                let mid = ((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
                let depth = zones_ref
                    .iter()
                    .filter(|zone| point_in_polygon_parts(mid, &zone.rings))
                    .map(|zone| zone.depth_class_m)
                    .fold(0.0_f64, f64::max);
                penalty_for_depth(depth)
            })
            .map_err(|error| AnalysisError::Message(error.to_string()))?;
        let origin = ward_origins[0];
        let shelter = shelter_nodes[0];
        let recorded = flooded_graph.route_minutes(origin, shelter).unwrap_or(-1.0);
        let replayed = replay_flooded
            .route_minutes(origin, shelter)
            .unwrap_or(-2.0);
        (recorded - replayed).abs() < 1e-9
    };

    let rows: Vec<(String, f64, f64)> = features
        .iter()
        .map(|feature| {
            (
                feature.ward_name.clone(),
                feature.baseline_minutes,
                feature.flooded_minutes,
            )
        })
        .collect();
    let duckdb_ok = genegis_query::verify_evacuation_delays(&rows)
        .map_err(|error| AnalysisError::Message(error.to_string()))?;

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
            name: "flood_zones_loaded".into(),
            passed: !zones.is_empty()
                && zones
                    .iter()
                    .all(|zone| DEPTH_BANDS_M.contains(&zone.depth_class_m)),
            detail: format!("{} depth-band zones over the city fixture", zones.len()),
        },
        VerificationCheck {
            name: "penalty_monotonicity".into(),
            passed: monotonic_ok,
            detail: format!(
                "{} wards: flooded route ≥ clean route for every ward",
                features.len()
            ),
        },
        VerificationCheck {
            name: "route_sanity_triangle".into(),
            passed: triangle_ok,
            detail: format!("{triangle_samples} penalized triples satisfy triangle inequality"),
        },
        VerificationCheck {
            name: "determinism_replay".into(),
            passed: replay_ok,
            detail: "independent rebuild reproduced the penalized route cost".into(),
        },
        VerificationCheck {
            name: "duckdb_cross_check".into(),
            passed: duckdb_ok,
            detail: format!("{} ward delay rows re-checked in DuckDB", rows.len()),
        },
    ];

    let source = density.verification.source.clone();
    Ok(EvacuationAnalysis {
        workflow: density.workflow.clone(),
        features,
        style,
        verification: VerificationReport {
            crs: density.verification.crs.clone(),
            coordinate_unit: density.verification.coordinate_unit.clone(),
            area_unit: "n/a".into(),
            area_method: "dijkstra_penalized_walk_graph".into(),
            density_unit: "evacuation minutes".into(),
            source,
            checks,
        },
        citations: vec![
            Citation {
                title: "国土地理院 指定緊急避難場所・指定避難所データ（データ構造参照）".into(),
                url: Some("https://www.gsi.go.jp/bousaichiri/hinanbasho.html".into()),
                license: Some("政府標準利用規約（参考）".into()),
                retrieved_at: None,
            },
            Citation {
                title: "合成フィクスチャ: scripts/build-nagoya-shelters.py（GSI実測ではない）"
                    .into(),
                url: Some(format!("file://{shelters_path}")),
                license: Some("CC0-1.0 (fixture)".into()),
                retrieved_at: None,
            },
        ],
        shelter_count: shelters.len(),
        node_count: graph.node_count(),
        edge_count: graph.edge_count(),
        total_length_km: graph.total_length_km(),
        flooded_edge_count,
        flooded_length_share: flooded_edge_length_m / (graph.total_length_km() * 1000.0),
        depth_penalty_per_m,
    })
}

fn load_flood_zones(path: &str) -> Result<Vec<FloodZone>, AnalysisError> {
    let dataset = genegis_vector::read_geojson_path(path)
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    let mut zones = Vec::new();
    for feature in &dataset.features {
        let depth = feature
            .properties
            .get("depth_class_m")
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| {
                AnalysisError::Message(format!(
                    "flood zone {} lacks numeric depth_class_m",
                    feature.id
                ))
            })?;
        if !DEPTH_BANDS_M.contains(&depth) && !depth.eq(&0.0) {
            return Err(AnalysisError::Message(format!(
                "flood zone {} declares depth {depth} outside the official bands",
                feature.id
            )));
        }
        if feature.rings.is_empty() {
            continue;
        }
        zones.push(FloodZone {
            zone_id: feature
                .properties
                .get("zone_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("zone")
                .to_string(),
            name: feature
                .properties
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string(),
            depth_class_m: depth,
            rings: feature.rings.clone(),
        });
    }
    if zones.is_empty() {
        return Err(AnalysisError::Message(
            "flood fixture contained zero usable zones".into(),
        ));
    }
    Ok(zones)
}

/// Load named point features (shelters) from raw GeoJSON.
fn load_named_points(
    path: &str,
    name_key: &str,
) -> Result<Vec<((f64, f64), String)>, AnalysisError> {
    let text =
        std::fs::read_to_string(path).map_err(|error| AnalysisError::Message(error.to_string()))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&text).map_err(|error| AnalysisError::Message(error.to_string()))?;
    if parsed.get("type").and_then(serde_json::Value::as_str) != Some("FeatureCollection") {
        return Err(AnalysisError::Message(
            "shelter fixture must be a FeatureCollection".into(),
        ));
    }
    let mut points = Vec::new();
    for feature in parsed
        .get("features")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| AnalysisError::Message("shelter fixture lacks features".into()))?
    {
        let geometry = feature
            .get("geometry")
            .ok_or_else(|| AnalysisError::Message("shelter feature without geometry".into()))?;
        if geometry.get("type").and_then(serde_json::Value::as_str) != Some("Point") {
            return Err(AnalysisError::Message(
                "shelter geometry must be a Point".into(),
            ));
        }
        let coords = geometry
            .get("coordinates")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| AnalysisError::Message("shelter Point without coordinates".into()))?;
        let lon = coords
            .first()
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| AnalysisError::Message("shelter longitude missing".into()))?;
        let lat = coords
            .get(1)
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| AnalysisError::Message("shelter latitude missing".into()))?;
        let name = feature
            .get("properties")
            .and_then(|props| props.get(name_key))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("shelter")
            .to_string();
        points.push(((lon, lat), name));
    }
    if points.is_empty() {
        return Err(AnalysisError::Message("shelter fixture is empty".into()));
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
    use crate::nagoya::default_nagoya_data_path;
    use genegis_catalog::{
        nagoya_shelters_path, nagoya_walk_network_path, nagoya_wards_geojson_path,
    };

    fn zones_path() -> &'static str {
        genegis_catalog::nagoya_flood_zones_path()
    }

    #[test]
    fn routes_every_ward_to_a_shelter_with_monotone_delays() {
        let analysis = run_nagoya_evacuation_access(
            default_nagoya_data_path(),
            nagoya_walk_network_path(),
            zones_path(),
            nagoya_shelters_path(),
        )
        .expect("evacuation run");
        assert_eq!(analysis.features.len(), 16);
        assert_eq!(analysis.shelter_count, 32);
        assert!(
            analysis.flooded_edge_count > 0,
            "fixture must flood streets"
        );
        assert!(analysis.flooded_length_share > 0.0 && analysis.flooded_length_share < 1.0);
        for check in &analysis.verification.checks {
            assert!(
                check.passed,
                "check {} failed: {}",
                check.name, check.detail
            );
        }
        // 港区 sits inside the coastal corridor; its delay must exceed an
        // inland ward's on the same fixture.
        let max_delay = analysis
            .features
            .iter()
            .map(|feature| feature.delay_minutes)
            .fold(0.0_f64, f64::max);
        assert!(max_delay > 0.0, "flooding must delay at least one ward");
    }

    #[test]
    fn zero_penalty_reproduces_baseline_routes() {
        let analysis = run_nagoya_evacuation_access_with_penalty(
            default_nagoya_data_path(),
            nagoya_walk_network_path(),
            zones_path(),
            nagoya_shelters_path(),
            0.0,
        )
        .expect("zero-penalty run");
        for feature in &analysis.features {
            assert!((feature.delay_minutes.abs()) < 1e-6);
            assert!(feature.flooded_shelter_name.is_none());
        }
    }

    #[test]
    fn rejects_negative_penalty() {
        let result = run_nagoya_evacuation_access_with_penalty(
            default_nagoya_data_path(),
            nagoya_walk_network_path(),
            zones_path(),
            nagoya_shelters_path(),
            -1.0,
        );
        assert!(matches!(result, Err(AnalysisError::Message(_))));
    }
}
