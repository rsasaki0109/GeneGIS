//! UC-1 flood exposure overlay: per-ward population exposure to flood zones.
//!
//! Wards come from the verified north-star density pipeline; flood zones are
//! a bundled synthetic fixture approximating 想定最大規模 depth-band corridors
//! (see `scripts/build-nagoya-flood-zones.py`). Exposure is computed by
//! deterministic even-odd grid sampling of every ward polygon against the
//! zone layer, then cross-checked in DuckDB.

use genegis_crs::Crs;
use genegis_geometry::{point_in_polygon_parts, PolygonRing};
use genegis_style::{ChoroplethStyle, ColorRgba};
use genegis_vector::read_geojson_path;
use genegis_workflow::{Citation, GeoWorkflow};

use crate::nagoya::run_nagoya_population_density;
use crate::result::{VerificationCheck, VerificationReport};
use crate::zone_index::ZoneIndex;
use crate::AnalysisError;

/// Deterministic sampling resolution per ward bbox axis.
pub const DEFAULT_SAMPLES_PER_AXIS: u32 = 96;

/// Depth bands mirrored from 重ねるハザードマップ legend conventions.
pub const DEPTH_BANDS_M: [f64; 3] = [0.5, 3.0, 5.0];

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FloodZone {
    /// Stable zone identity from the fixture.
    pub zone_id: String,
    /// Human-readable zone name.
    pub name: String,
    /// Assumed maximum inundation depth in metres for this zone.
    pub depth_class_m: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rings: Vec<PolygonRing>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FloodExposureFeature {
    pub ward_code: String,
    pub ward_name: String,
    /// Total ward population from the census fixture.
    pub population: u64,
    /// Estimated population inside any flood zone.
    pub exposed_population: u64,
    /// Fraction of in-ward sample points that fell inside a flood zone.
    pub exposure_rate: f64,
    /// Deepest zone class reached by this ward, 0.0 when dry.
    pub max_depth_class_m: f64,
    /// Sample points classified as inside the ward polygon.
    pub sampled_points: u64,
    /// Sample points that also fell inside at least one zone.
    pub flooded_samples: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rings: Vec<PolygonRing>,
    pub color: ColorRgba,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FloodExposureAnalysis {
    pub workflow: GeoWorkflow,
    pub features: Vec<FloodExposureFeature>,
    pub style: ChoroplethStyle,
    pub verification: VerificationReport,
    pub citations: Vec<Citation>,
    /// Number of flood zones loaded from the fixture.
    pub zone_count: usize,
    /// Sampling resolution used along each bbox axis.
    pub samples_per_axis: u32,
}

/// Run the flood exposure overlay over the bundled Nagoya fixtures.
pub fn run_nagoya_flood_exposure(
    wards_path: &str,
    zones_path: &str,
) -> Result<FloodExposureAnalysis, AnalysisError> {
    run_nagoya_flood_exposure_with_options(wards_path, zones_path, DEFAULT_SAMPLES_PER_AXIS)
}

pub fn run_nagoya_flood_exposure_with_options(
    wards_path: &str,
    zones_path: &str,
    samples_per_axis: u32,
) -> Result<FloodExposureAnalysis, AnalysisError> {
    if samples_per_axis < 8 {
        return Err(AnalysisError::Message(
            "flood exposure requires at least 8 samples per axis".into(),
        ));
    }
    let density = run_nagoya_population_density(wards_path)?;
    let zones_dataset =
        read_geojson_path(zones_path).map_err(|error| AnalysisError::Message(error.to_string()))?;
    let mut zones = Vec::with_capacity(zones_dataset.features.len());
    for feature in &zones_dataset.features {
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
        let zone_id = feature
            .properties
            .get("zone_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("zone")
            .to_string();
        let name = feature
            .properties
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string();
        if feature.rings.is_empty() {
            return Err(AnalysisError::Message(format!(
                "flood zone {zone_id} carries no polygon geometry"
            )));
        }
        zones.push(FloodZone {
            zone_id,
            name,
            depth_class_m: depth,
            rings: feature.rings.clone(),
        });
    }
    if zones.is_empty() {
        return Err(AnalysisError::Message(
            "flood fixture contained zero usable zones".into(),
        ));
    }

    let zone_index = ZoneIndex::build(zones)?;
    let mut features = Vec::with_capacity(density.features.len());
    for ward in &density.features {
        let (min_lon, min_lat, max_lon, max_lat) = ward_bbox(&ward.rings);
        let mut flooded = 0_u64;
        let mut sampled = 0_u64;
        let mut max_depth = 0.0_f64;
        for column in 0..samples_per_axis {
            for row in 0..samples_per_axis {
                let lon = min_lon
                    + (max_lon - min_lon) * (column as f64 + 0.5) / f64::from(samples_per_axis);
                let lat = min_lat
                    + (max_lat - min_lat) * (row as f64 + 0.5) / f64::from(samples_per_axis);
                let point = (lon, lat);
                if !point_in_polygon_parts(point, &ward.rings) {
                    continue;
                }
                sampled += 1;
                let deepest_here = zone_index.depth_at(point);
                if deepest_here > 0.0 {
                    flooded += 1;
                    max_depth = max_depth.max(deepest_here);
                }
            }
        }
        let exposure_rate = if sampled > 0 {
            flooded as f64 / sampled as f64
        } else {
            0.0
        };
        let exposed_population = (ward.population as f64 * exposure_rate).round() as u64;
        features.push(FloodExposureFeature {
            ward_code: ward.ward_code.clone(),
            ward_name: ward.ward_name.clone(),
            population: ward.population,
            exposed_population,
            exposure_rate,
            max_depth_class_m: max_depth,
            sampled_points: sampled,
            flooded_samples: flooded,
            rings: ward.rings.clone(),
            color: ColorRgba::new(0.55, 0.55, 0.58, 1.0),
        });
    }

    let rates_percent: Vec<f64> = features
        .iter()
        .map(|feature| feature.exposure_rate * 100.0)
        .collect();
    let style = ChoroplethStyle::equal_interval("exposure_rate", "%", &rates_percent, 5);
    for feature in &mut features {
        feature.color = style.color_for(feature.exposure_rate * 100.0);
    }

    let rows: Vec<(String, u64, u64, f64)> = features
        .iter()
        .map(|feature| {
            (
                feature.ward_name.clone(),
                feature.exposed_population,
                feature.population,
                feature.exposure_rate,
            )
        })
        .collect();
    let duckdb_ok = genegis_query::verify_flood_exposure(&rows)
        .map_err(|error| AnalysisError::Message(error.to_string()))?;

    let total_population: u64 = features.iter().map(|feature| feature.population).sum();
    let total_exposed: u64 = features
        .iter()
        .map(|feature| feature.exposed_population)
        .sum();
    let determinism_probe =
        resample_first_ward(&density.features[0], &zone_index, samples_per_axis);

    let checks = vec![
        VerificationCheck {
            name: "flood_zones_loaded".into(),
            passed: true,
            detail: format!(
                "{} zones across depth bands {:?}",
                zone_index.len(),
                DEPTH_BANDS_M
            ),
        },
        VerificationCheck {
            name: "sample_coverage".into(),
            passed: features.iter().all(|feature| feature.sampled_points > 0),
            detail: format!(
                "min sampled points per ward: {}",
                features
                    .iter()
                    .map(|feature| feature.sampled_points)
                    .min()
                    .unwrap_or(0)
            ),
        },
        VerificationCheck {
            name: "population_bounds".into(),
            passed: total_exposed <= total_population,
            detail: format!("exposed={total_exposed} <= total={total_population}"),
        },
        VerificationCheck {
            name: "determinism_replay".into(),
            passed: determinism_probe == (features[0].flooded_samples, features[0].sampled_points),
            detail: format!(
                "first-ward replay {:?} vs recorded {:?}",
                determinism_probe,
                (features[0].flooded_samples, features[0].sampled_points)
            ),
        },
        VerificationCheck {
            name: "duckdb_cross_check".into(),
            passed: duckdb_ok,
            detail: format!("{} ward rows re-aggregated in DuckDB", rows.len()),
        },
    ];

    let source = density.verification.source.clone();
    let crs = Crs::parse(&density.verification.crs)
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    Ok(FloodExposureAnalysis {
        workflow: density.workflow.clone(),
        features,
        style,
        verification: VerificationReport {
            crs: density.verification.crs.clone(),
            coordinate_unit: crs.coordinate_unit().to_string(),
            area_unit: "km²".into(),
            area_method: "grid_sampling_even_odd_point_in_polygon".into(),
            density_unit: "exposed persons".into(),
            source,
            checks,
        },
        citations: vec![
            Citation {
                title: "ハザードマップポータルサイト 洪水浸水想定区域（想定最大規模）".into(),
                url: Some(
                    "https://disaportaldata.gsi.go.jp/raster/01_flood_l2_shinsuishin_Kuni_data/"
                        .into(),
                ),
                license: Some("国土交通省 各地方整備局等・都道府県".into()),
                retrieved_at: None,
            },
            Citation {
                title: "国土数値情報 洪水浸水想定区域データ (A32)".into(),
                url: Some("https://nlftp.mlit.go.jp/ksj/gml/datalist/KsjTmplt-A32.html".into()),
                license: Some("政府標準利用規約（約款第2の規定）".into()),
                retrieved_at: None,
            },
            Citation {
                title:
                    "合成フィクスチャ: scripts/build-nagoya-flood-zones.py（実測データではない）"
                        .into(),
                url: Some(file_url(zones_path)),
                license: Some("CC0-1.0 (fixture)".into()),
                retrieved_at: None,
            },
        ],
        zone_count: zone_index.len(),
        samples_per_axis,
    })
}

fn resample_first_ward(
    ward: &crate::result::DensityFeature,
    zone_index: &ZoneIndex,
    samples_per_axis: u32,
) -> (u64, u64) {
    let (min_lon, min_lat, max_lon, max_lat) = ward_bbox(&ward.rings);
    let mut flooded = 0_u64;
    let mut sampled = 0_u64;
    for column in 0..samples_per_axis {
        for row in 0..samples_per_axis {
            let lon =
                min_lon + (max_lon - min_lon) * (column as f64 + 0.5) / f64::from(samples_per_axis);
            let lat =
                min_lat + (max_lat - min_lat) * (row as f64 + 0.5) / f64::from(samples_per_axis);
            if !point_in_polygon_parts((lon, lat), &ward.rings) {
                continue;
            }
            sampled += 1;
            if zone_index.depth_at((lon, lat)) > 0.0 {
                flooded += 1;
            }
        }
    }
    (flooded, sampled)
}

fn ward_bbox(rings: &[PolygonRing]) -> (f64, f64, f64, f64) {
    let mut bounds = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for ring in rings {
        for &(lon, lat) in ring.exterior() {
            bounds.0 = bounds.0.min(lon);
            bounds.1 = bounds.1.min(lat);
            bounds.2 = bounds.2.max(lon);
            bounds.3 = bounds.3.max(lat);
        }
    }
    if bounds.0 > bounds.2 {
        (0.0, 0.0, 0.0, 0.0)
    } else {
        bounds
    }
}

fn file_url(path: &str) -> String {
    if path.starts_with("http") {
        path.to_string()
    } else {
        format!("file://{path}")
    }
}

/// Projected viewport shared by every SVG ring of one map.
struct SvgFrame {
    min_x: f64,
    min_y: f64,
    dx: f64,
    dy: f64,
    inner_w: f64,
    inner_h: f64,
}

impl SvgFrame {
    fn path(&self, ring: &[(f64, f64)]) -> String {
        let mut parts = Vec::new();
        for (index, &(x, y)) in ring.iter().enumerate() {
            let sx = PAD + (x - self.min_x) / self.dx * self.inner_w;
            let sy = PAD + ((self.min_y + self.dy) - y) / self.dy * self.inner_h;
            let command = if index == 0 { "M" } else { "L" };
            parts.push(format!("{command} {sx:.2} {sy:.2}"));
        }
        parts.push("Z".into());
        parts.join(" ")
    }
}

const PAD: f64 = 40.0;

/// Render the exposure analysis into a standalone verified HTML map.
pub fn export_flood_html_map(analysis: &FloodExposureAnalysis, title: &str) -> String {
    const WIDTH: f64 = 960.0;
    const HEIGHT: f64 = 660.0;
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for feature in &analysis.features {
        for &(lon, lat) in feature.rings.iter().flat_map(|r| r.exterior()) {
            min_x = min_x.min(lon);
            min_y = min_y.min(lat);
            max_x = max_x.max(lon);
            max_y = max_y.max(lat);
        }
    }
    let frame = SvgFrame {
        min_x,
        min_y,
        dx: (max_x - min_x).max(1e-9),
        dy: (max_y - min_y).max(1e-9),
        inner_w: WIDTH - PAD * 2.0,
        inner_h: HEIGHT - PAD * 2.0,
    };

    let mut paths = String::new();
    for feature in &analysis.features {
        for part in &feature.rings {
            let mut d = frame.path(part.exterior());
            for hole in part.holes() {
                d.push(' ');
                d.push_str(&frame.path(hole));
            }
            paths.push_str(&format!(
                r##"<path d="{d}" fill="{fill}" fill-rule="evenodd" stroke="#1a1a1a" stroke-width="0.5"><title>{name}: 曝露率 {rate:.1}%（{exposed}/{total}人, 最大深 {depth:.1}m）</title></path>"##,
                fill = feature.color.to_hex(),
                name = escape_xml(&feature.ward_name),
                rate = feature.exposure_rate * 100.0,
                exposed = feature.exposed_population,
                total = feature.population,
                depth = feature.max_depth_class_m,
            ));
        }
    }

    let mut legend = String::new();
    for item in &analysis.style.legend {
        legend.push_str(&format!(
            r#"<div class="legend-item"><span class="swatch" style="background:{}"></span><span>{}</span></div>"#,
            item.color.to_hex(),
            escape_xml(&item.label)
        ));
    }
    let mut checks = String::new();
    for check in &analysis.verification.checks {
        let mark = if check.passed { "✓" } else { "✗" };
        checks.push_str(&format!(
            "<li><strong>{mark}</strong> {} — {}</li>",
            escape_xml(&check.name),
            escape_xml(&check.detail)
        ));
    }
    let mut cites = String::new();
    for citation in &analysis.citations {
        cites.push_str(&format!(
            "<li><a href=\"{}\">{}</a> ({})</li>",
            escape_xml(citation.url.as_deref().unwrap_or("#")),
            escape_xml(&citation.title),
            escape_xml(citation.license.as_deref().unwrap_or(""))
        ));
    }

    format!(
        r##"<!DOCTYPE html>
<html lang="ja">
<head>
  <meta charset="utf-8" />
  <title>{title}</title>
  <style>
    body {{ font-family: system-ui, sans-serif; margin: 0; background: #0f1720; color: #e8eef5; }}
    header {{ padding: 16px 24px; border-bottom: 1px solid #243041; }}
    main {{ display: grid; grid-template-columns: 1fr 320px; gap: 16px; padding: 16px 24px; }}
    .panel {{ background: #162231; border: 1px solid #243041; border-radius: 8px; padding: 16px; }}
    .legend-item {{ display: flex; align-items: center; gap: 8px; margin: 6px 0; }}
    .swatch {{ width: 18px; height: 18px; border-radius: 3px; border: 1px solid #444; }}
    svg {{ width: 100%; height: auto; background: #0b121a; border-radius: 8px; }}
    h2 {{ font-size: 14px; text-transform: uppercase; letter-spacing: 0.08em; color: #8aa0b5; }}
    ul {{ padding-left: 18px; }}
    a {{ color: #7ec8ff; }}
  </style>
</head>
<body>
  <header>
    <h1>{title}</h1>
    <p>GeneGIS UC-1 — 洪水浸水リスク×人口曝露（DuckDB 検証付き）</p>
  </header>
  <main>
    <section class="panel">
      <svg viewBox="0 0 {WIDTH} {HEIGHT}" xmlns="http://www.w3.org/2000/svg">{paths}</svg>
    </section>
    <aside>
      <div class="panel">
        <h2>Legend</h2>
        {legend}
      </div>
      <div class="panel">
        <h2>Verification</h2>
        <ul>{checks}</ul>
      </div>
      <div class="panel">
        <h2>Sources</h2>
        <ul>{cites}</ul>
      </div>
    </aside>
  </main>
</body>
</html>"##,
        title = escape_xml(title),
    )
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nagoya::default_nagoya_data_path;

    fn zones_path() -> &'static str {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/nagoya-population-density/data/nagoya-flood-zones.geojson"
        )
    }

    #[test]
    fn computes_exposure_over_real_fixtures() {
        let analysis = run_nagoya_flood_exposure(default_nagoya_data_path(), zones_path())
            .expect("flood exposure");
        assert_eq!(analysis.zone_count, 6);
        assert_eq!(analysis.features.len(), 16);
        for check in &analysis.verification.checks {
            assert!(
                check.passed,
                "check {} failed: {}",
                check.name, check.detail
            );
        }
        // Southern lowland wards must be exposed; mountainous northern wards
        // may stay dry. At least one ward of each kind is expected.
        let exposed: Vec<_> = analysis
            .features
            .iter()
            .filter(|feature| feature.exposure_rate > 0.0)
            .collect();
        assert!(
            !exposed.is_empty(),
            "no ward was exposed; overlay is broken"
        );
        let dry = analysis
            .features
            .iter()
            .filter(|feature| feature.exposure_rate == 0.0)
            .count();
        assert!(dry > 0, "every ward flooded; sampling or PIP is broken");
        for feature in &analysis.features {
            assert!(feature.exposure_rate >= 0.0 && feature.exposure_rate <= 1.0);
            assert!(feature.exposed_population <= feature.population);
        }
        let html = export_flood_html_map(&analysis, "名古屋市 洪水リスク");
        assert!(html.contains("洪水浸水リスク"));
        assert!(html.contains("duckdb_cross_check"));
    }

    #[test]
    fn determinism_across_runs() {
        let first =
            run_nagoya_flood_exposure(default_nagoya_data_path(), zones_path()).expect("first run");
        let second = run_nagoya_flood_exposure_with_options(
            default_nagoya_data_path(),
            zones_path(),
            DEFAULT_SAMPLES_PER_AXIS,
        )
        .expect("second run");
        let left: Vec<(u64, u64)> = first
            .features
            .iter()
            .map(|feature| (feature.flooded_samples, feature.sampled_points))
            .collect();
        let right: Vec<(u64, u64)> = second
            .features
            .iter()
            .map(|feature| (feature.flooded_samples, feature.sampled_points))
            .collect();
        assert_eq!(left, right);
    }
}
