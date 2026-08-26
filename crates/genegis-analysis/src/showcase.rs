//! README showcase frame renderer (docs/assets GIF source).
//!
//! Runs every flagship analysis over the bundled fixtures and rasterizes
//! one PNG per use case through the same resvg pipeline as `export`.
//! Frames are deterministic: identical fixtures yield identical bytes, so
//! the committed GIF can always be regenerated bit-stable modulo palette.

use genegis_geometry::PolygonRing;
use genegis_style::ColorRgba;

use crate::accessibility::run_nagoya_accessibility;
use crate::evacuation::run_nagoya_evacuation_access;
use crate::export::export_png_map;
use crate::flood::run_nagoya_flood_exposure;
use crate::nagoya::{default_nagoya_data_path, run_nagoya_population_density};
use crate::ndvi::run_nagoya_ndvi_timeseries;
use crate::AnalysisError;

/// One rendered showcase frame.
pub struct ShowcaseFrame {
    pub name: &'static str,
    pub png: Vec<u8>,
}

const FRAME_W: f64 = 960.0;
const FRAME_H: f64 = 600.0;
const PAD: f64 = 46.0;

/// Render every flagship use case as a PNG frame.
pub fn render_usecase_frames() -> Result<Vec<ShowcaseFrame>, AnalysisError> {
    let wards = default_nagoya_data_path();
    let mut frames = Vec::new();

    let density = run_nagoya_population_density(wards)?;
    frames.push(ShowcaseFrame {
        name: "density",
        png: export_png_map(&density, "UC-0 人口密度 — 北極星デモ")
            .map_err(|error| AnalysisError::Message(format!("density render failed: {error}")))?,
    });

    let flood = run_nagoya_flood_exposure(wards, genegis_catalog::nagoya_flood_zones_path())?;
    let legend = legend_of(&flood.style);
    frames.push(ShowcaseFrame {
        name: "flood",
        png: render_wards_png(
            "UC-1 洪水浸水リスク × 人口曝露",
            &legend,
            &flood
                .features
                .iter()
                .map(|feature| {
                    (
                        feature.rings.clone(),
                        feature.color,
                        feature.ward_name.clone(),
                        format!("{:.1}%", feature.exposure_rate * 100.0),
                    )
                })
                .collect::<Vec<_>>(),
        )?,
    });

    let evacuation = run_nagoya_evacuation_access(
        wards,
        genegis_catalog::nagoya_walk_network_path(),
        genegis_catalog::nagoya_flood_zones_path(),
        genegis_catalog::nagoya_shelters_path(),
    )?;
    let legend = legend_of(&evacuation.style);
    frames.push(ShowcaseFrame {
        name: "evacuation",
        png: render_wards_png(
            "UC-1 避難アクセス — 洪水ペナルティ付き遅延",
            &legend,
            &evacuation
                .features
                .iter()
                .map(|feature| {
                    (
                        feature.rings.clone(),
                        feature.color,
                        feature.ward_name.clone(),
                        format!("+{:.1} min", feature.delay_minutes),
                    )
                })
                .collect::<Vec<_>>(),
        )?,
    });

    let accessibility = run_nagoya_accessibility(
        wards,
        genegis_catalog::nagoya_walk_network_path(),
        genegis_catalog::nagoya_pois_path(),
    )?;
    let legend = legend_of(&accessibility.style);
    frames.push(ShowcaseFrame {
        name: "xmin-city",
        png: render_wards_png(
            "UC-4 15分都市 — 到達POIシェア",
            &legend,
            &accessibility
                .features
                .iter()
                .map(|feature| {
                    (
                        feature.rings.clone(),
                        feature.color,
                        feature.ward_name.clone(),
                        format!("{:.0}%", feature.accessibility_score * 100.0),
                    )
                })
                .collect::<Vec<_>>(),
        )?,
    });

    let ndvi = run_nagoya_ndvi_timeseries(
        &genegis_catalog::resolve_catalog_url("examples/stac/ndvi-timeseries-collection.json"),
        wards,
    )?;
    let legend = vec![
        ("gain ≥ +0.02".into(), "#2e7d32".into()),
        ("stable".into(), "#9e9e9e".into()),
        ("loss ≤ −0.02".into(), "#c62828".into()),
    ];
    frames.push(ShowcaseFrame {
        name: "ndvi",
        png: render_wards_png(
            "UC-3 NDVI時系列 — エポック間差分",
            &legend,
            &ndvi
                .features
                .iter()
                .map(|feature| {
                    let fill = match feature.change_class.as_str() {
                        "gain" => hex_color("#2e7d32"),
                        "loss" => hex_color("#c62828"),
                        _ => hex_color("#9e9e9e"),
                    };
                    (
                        feature.rings.clone(),
                        fill,
                        feature.ward_name.clone(),
                        format!("{:+.0}×10⁻³", feature.delta_ndvi * 1000.0),
                    )
                })
                .collect::<Vec<_>>(),
        )?,
    });

    frames.extend(render_change_frames()?);

    Ok(frames)
}

fn legend_of(style: &genegis_style::ChoroplethStyle) -> Vec<(String, String)> {
    style
        .legend
        .iter()
        .map(|item| (item.label.clone(), item.color.to_hex()))
        .collect()
}

type WardShape = (Vec<PolygonRing>, ColorRgba, String, String);

fn render_wards_png(
    title: &str,
    legend: &[(String, String)],
    wards: &[WardShape],
) -> Result<Vec<u8>, AnalysisError> {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for (rings, ..) in wards {
        for ring in rings {
            for &(x, y) in ring.exterior() {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    let mut paths = String::new();
    for (rings, color, name, value) in wards {
        for ring in rings {
            let d = ring_to_svg_path(ring.exterior(), min_x, min_y, max_x, max_y);
            paths.push_str(&format!(
                r##"<path d="{d}" fill="{}" fill-rule="evenodd" stroke="#1a1a1a" stroke-width="0.5"><title>{name}: {value}</title></path>"##,
                color.to_hex(),
            ));
            for hole in ring.holes() {
                let dh = ring_to_svg_path(hole, min_x, min_y, max_x, max_y);
                paths.push_str(&format!(
                    r##"<path d="{dh}" fill="#ffffff" fill-rule="evenodd" stroke="#1a1a1a" stroke-width="0.4"/>"##
                ));
            }
        }
    }
    let svg = wrap_svg(title, &paths, legend);
    rasterize_svg(&svg)
}

/// Point-cloud scatter frames for the UC-5 epochs.
fn render_change_frames() -> Result<Vec<ShowcaseFrame>, AnalysisError> {
    let mut frames = Vec::new();
    for (path_name, name, title) in [
        (
            genegis_catalog::nagoya_pointcloud_epoch_a_path(),
            "uc5-epoch-a",
            "UC-5 点群 エポックA (2024)",
        ),
        (
            genegis_catalog::nagoya_pointcloud_epoch_b_path(),
            "uc5-epoch-b",
            "UC-5 点群 エポックB (2025)",
        ),
    ] {
        let cloud = genegis_pointcloud::read_point_cloud_path(path_name)
            .map_err(|error| AnalysisError::Message(error.to_string()))?;
        frames.push(ShowcaseFrame {
            name,
            png: render_epoch_png(&cloud, title)?,
        });
    }
    Ok(frames)
}

fn render_epoch_png(
    cloud: &genegis_pointcloud::PointCloud,
    title: &str,
) -> Result<Vec<u8>, AnalysisError> {
    let Some(bounds) = cloud.bounds() else {
        return Err(AnalysisError::Message("empty point cloud".into()));
    };
    // Subsample to ~12k dots for a crisp, small SVG.
    let stride = (cloud.point_count() / 12_000).max(1);
    let scale_x = (FRAME_W - PAD * 2.0) / (bounds[3] - bounds[0]).max(1e-9);
    let scale_y = (FRAME_H - PAD * 2.0) / (bounds[4] - bounds[1]).max(1e-9);
    let uniform = scale_x.min(scale_y);
    let offset_x = PAD + ((FRAME_W - PAD * 2.0) - (bounds[3] - bounds[0]) * uniform) / 2.0;
    let offset_y = PAD + ((FRAME_H - PAD * 2.0) - (bounds[4] - bounds[1]) * uniform) / 2.0;
    let mut dots = String::new();
    for p in cloud.points.iter().step_by(stride) {
        let sx = offset_x + (p[0] - bounds[0]) * uniform;
        let sy = FRAME_H - offset_y - (p[1] - bounds[1]) * uniform;
        let fill = if p[2] > 8.0 {
            "#d84315"
        } else if p[2] > 3.0 {
            "#ff9800"
        } else {
            "#90a4ae"
        };
        dots.push_str(&format!(
            r#"<circle cx="{sx:.1}" cy="{sy:.1}" r="0.9" fill="{fill}"/>"#
        ));
    }
    let legend = vec![
        ("ground".to_string(), "#90a4ae".to_string()),
        ("vegetation 3–8 m".to_string(), "#ff9800".to_string()),
        ("building > 8 m".to_string(), "#d84315".to_string()),
    ];
    rasterize_svg(&wrap_svg(title, &dots, &legend))
}

/// Parse `#rrggbb` into a [`ColorRgba`].
fn hex_color(value: &str) -> ColorRgba {
    let hex = value.trim_start_matches('#');
    let channel = |range: std::ops::Range<usize>| {
        u8::from_str_radix(&hex[range], 16).unwrap_or(128) as f32 / 255.0
    };
    ColorRgba::new(channel(0..2), channel(2..4), channel(4..6), 1.0)
}

fn ring_to_svg_path(ring: &[(f64, f64)], min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> String {
    let dx = (max_x - min_x).max(1e-9);
    let dy = (max_y - min_y).max(1e-9);
    let inner_w = FRAME_W - PAD * 2.0;
    let inner_h = FRAME_H - PAD * 2.0;
    // Preserve aspect ratio within the canvas.
    let scale = (inner_w / dx).min(inner_h / dy);
    let offset_x = PAD + (inner_w - dx * scale) / 2.0;
    let offset_y = PAD + (inner_h - dy * scale) / 2.0;
    let mut parts = Vec::new();
    for (i, (x, y)) in ring.iter().enumerate() {
        let sx = offset_x + (x - min_x) * scale;
        let sy = offset_y + (max_y - y) * scale;
        let cmd = if i == 0 { "M" } else { "L" };
        parts.push(format!("{cmd} {sx:.2} {sy:.2}"));
    }
    parts.push("Z".into());
    parts.join(" ")
}

/// Frame subtitle reflects which data tier produced the frames.
fn subtitle() -> &'static str {
    if std::env::var("GENEGIS_WALK_NETWORK_PATH").is_ok()
        || std::env::var("GENEGIS_FLOOD_ZONES_PATH").is_ok()
    {
        "GeneGIS · real open data (国交省 A31a · OSM · 名古屋市) · RFC 0005"
    } else {
        "GeneGIS · verified offline fixture · RFC 0005"
    }
}

fn wrap_svg(title: &str, body: &str, legend: &[(String, String)]) -> String {
    let mut legend_items = String::new();
    for (i, (label, color)) in legend.iter().enumerate() {
        let x = 620.0 + (i % 2) as f64 * 170.0;
        let y = 505.0 + (i / 2) as f64 * 26.0;
        legend_items.push_str(&format!(
            r##"<rect x="{x}" y="{y}" width="14" height="14" rx="3" fill="{color}"/><text x="{}" y="{:.1}" font-family="Noto Serif CJK JP" font-size="13" fill="#37474f">{}</text>"##,
            x + 20.0,
            y + 11.5,
            escape_xml(label),
        ));
    }
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{FRAME_W}" height="{FRAME_H}" viewBox="0 0 {FRAME_W} {FRAME_H}">
<rect width="100%" height="100%" fill="#fafafa"/>
<text x="34" y="44" font-family="Noto Serif CJK JP" font-size="24" font-weight="bold" fill="#263238">{}</text>
<text x="34" y="70" font-family="Noto Serif CJK JP" font-size="13" fill="#607d8b">{}</text>
<g>{body}</g>
<g>{legend_items}</g>
</svg>"##,
        escape_xml(title),
        escape_xml(subtitle()),
    )
}

pub(crate) fn rasterize_svg(svg: &str) -> Result<Vec<u8>, AnalysisError> {
    let mut options = resvg::usvg::Options::<'static>::default();
    let mut fonts = resvg::usvg::fontdb::Database::new();
    fonts.load_system_fonts();
    fonts.set_serif_family("Noto Serif CJK JP");
    fonts.set_sans_serif_family("Noto Serif CJK JP");
    options.fontdb = std::sync::Arc::new(fonts);
    options.font_family = "Noto Serif CJK JP".into();
    let tree = resvg::usvg::Tree::from_str(svg, &options)
        .map_err(|err| AnalysisError::Message(format!("svg parse: {err}")))?;
    let size = tree.size().to_int_size();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size.width(), size.height())
        .ok_or_else(|| AnalysisError::Message("raster alloc failed".into()))?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::default(),
        &mut pixmap.as_mut(),
    );
    pixmap
        .encode_png()
        .map_err(|err| AnalysisError::Message(format!("png encode: {err}")))
}

pub(crate) fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
