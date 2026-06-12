use crate::result::AnalysisResult;
use thiserror::Error;

const MAP_WIDTH: f64 = 960.0;
const MAP_HEIGHT: f64 = 660.0;
const MAP_PAD: f64 = 40.0;
const PNG_WIDTH: f64 = 1280.0;
const PNG_HEIGHT: f64 = 720.0;

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("failed to allocate raster buffer")]
    RasterAlloc,
    #[error("SVG parse failed: {0}")]
    Svg(String),
    #[error("PNG encode failed: {0}")]
    Png(String),
}

pub fn export_html_map(result: &AnalysisResult, title: &str) -> String {
    let paths = build_map_paths(result, MAP_WIDTH, MAP_HEIGHT, MAP_PAD);

    let mut legend = String::new();
    for item in &result.style.legend {
        legend.push_str(&format!(
            r#"<div class="legend-item"><span class="swatch" style="background:{}"></span><span>{}</span></div>"#,
            item.color.to_hex(),
            escape_xml(&item.label)
        ));
    }

    let mut checks = String::new();
    for check in &result.verification.checks {
        let mark = if check.passed { "✓" } else { "✗" };
        checks.push_str(&format!(
            "<li><strong>{mark}</strong> {} — {}</li>",
            escape_xml(&check.name),
            escape_xml(&check.detail)
        ));
    }

    let mut cites = String::new();
    for c in &result.citations {
        cites.push_str(&format!(
            "<li><a href=\"{}\">{}</a> ({})</li>",
            escape_xml(&c.url),
            escape_xml(&c.title),
            escape_xml(&c.license)
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
    <p>GeneGIS MVP — workflow verified choropleth</p>
  </header>
  <main>
    <section class="panel">
      <svg viewBox="0 0 {MAP_WIDTH} {MAP_HEIGHT}" xmlns="http://www.w3.org/2000/svg">{paths}</svg>
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

pub fn export_map_svg(result: &AnalysisResult, title: &str) -> String {
    let paths = build_map_paths(result, MAP_WIDTH, MAP_HEIGHT, MAP_PAD);

    let mut legend = String::new();
    for (i, item) in result.style.legend.iter().enumerate() {
        let y = 88.0 + i as f64 * 28.0;
        legend.push_str(&format!(
            r##"<rect x="990" y="{y}" width="16" height="16" fill="{color}" stroke="#444444" stroke-width="0.5"/>
<text x="1014" y="{text_y}" font-family="sans-serif" font-size="12" fill="#e8eef5">{label}</text>"##,
            color = item.color.to_hex(),
            text_y = y + 12.0,
            label = escape_xml(&item.label),
        ));
    }

    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{PNG_WIDTH}" height="{PNG_HEIGHT}" viewBox="0 0 {PNG_WIDTH} {PNG_HEIGHT}">
  <rect width="100%" height="100%" fill="#0f1720"/>
  <text x="24" y="36" font-family="sans-serif" font-size="22" fill="#e8eef5">{title}</text>
  <text x="24" y="58" font-family="sans-serif" font-size="12" fill="#8aa0b5">GeneGIS MVP — workflow verified choropleth</text>
  <rect x="0" y="72" width="{MAP_WIDTH}" height="{MAP_HEIGHT}" fill="#0b121a"/>
  <svg x="0" y="72" width="{MAP_WIDTH}" height="{MAP_HEIGHT}" viewBox="0 0 {MAP_WIDTH} {MAP_HEIGHT}">
    {paths}
  </svg>
  <text x="990" y="72" font-family="sans-serif" font-size="14" fill="#8aa0b5">LEGEND</text>
  {legend}
</svg>"##,
        title = escape_xml(title),
    )
}

pub fn export_png_map(result: &AnalysisResult, title: &str) -> Result<Vec<u8>, ExportError> {
    let svg = export_map_svg(result, title);
    let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default())
        .map_err(|err| ExportError::Svg(err.to_string()))?;

    let size = tree.size().to_int_size();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size.width(), size.height())
        .ok_or(ExportError::RasterAlloc)?;

    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::default(),
        &mut pixmap.as_mut(),
    );

    pixmap
        .encode_png()
        .map_err(|err| ExportError::Png(err.to_string()))
}

fn build_map_paths(result: &AnalysisResult, width: f64, height: f64, pad: f64) -> String {
    let (min_x, min_y, max_x, max_y) = bbox_from_features(result);
    let mut paths = String::new();
    for feature in &result.features {
        for ring in &feature.rings {
            let d = ring_to_svg_path(
                ring.exterior(),
                min_x,
                min_y,
                max_x,
                max_y,
                width,
                height,
                pad,
            );
            paths.push_str(&format!(
                r##"<path d="{d}" fill="{fill}" stroke="#1a1a1a" stroke-width="0.5" data-ward="{ward}" data-density="{density:.1}"><title>{ward}: {density:.0} persons/km²</title></path>"##,
                fill = feature.color.to_hex(),
                ward = escape_xml(&feature.ward_name),
                density = feature.density_per_km2,
            ));
        }
    }
    paths
}

fn bbox_from_features(result: &AnalysisResult) -> (f64, f64, f64, f64) {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for feature in &result.features {
        for ring in &feature.rings {
            for (x, y) in ring.exterior() {
                min_x = min_x.min(*x);
                min_y = min_y.min(*y);
                max_x = max_x.max(*x);
                max_y = max_y.max(*y);
            }
        }
    }
    (min_x, min_y, max_x, max_y)
}

fn ring_to_svg_path(
    ring: &[(f64, f64)],
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
    width: f64,
    height: f64,
    pad: f64,
) -> String {
    let dx = (max_x - min_x).max(1e-9);
    let dy = (max_y - min_y).max(1e-9);
    let inner_w = width - pad * 2.0;
    let inner_h = height - pad * 2.0;

    let mut parts = Vec::new();
    for (i, (x, y)) in ring.iter().enumerate() {
        let sx = pad + (x - min_x) / dx * inner_w;
        let sy = pad + (max_y - y) / dy * inner_h;
        let cmd = if i == 0 { "M" } else { "L" };
        parts.push(format!("{cmd} {sx:.2} {sy:.2}"));
    }
    parts.push("Z".into());
    parts.join(" ")
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
    use crate::nagoya::{default_nagoya_data_path, run_nagoya_population_density};

    #[test]
    fn png_export_is_valid_png() {
        let result = run_nagoya_population_density(default_nagoya_data_path()).expect("analysis");
        let png = export_png_map(&result, "名古屋市 人口密度").expect("png");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(png.len() > 10_000);
    }
}
