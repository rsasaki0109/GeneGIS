//! Export layers to open formats and print-ready PDF maps.

use std::io::Write;

use geo::BoundingRect;
use geo_types::{Coord, Geometry, LineString, Polygon};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Result, ToolkitError};
use crate::layer::{sha256_bytes, Layer};
use crate::proj::{self, CrsInfo};
use crate::table::Classification;
use crate::{geojson_io, geoparquet_io, gpkg_io, wkt};

/// Export format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    /// RFC 7946 GeoJSON (EPSG:4326).
    Geojson,
    /// CSV with WKT geometry and a CRS column (UTF-8 with BOM for Excel).
    Csv,
    /// OGC GeoPackage.
    Geopackage,
    /// GeoParquet 1.1.
    Geoparquet,
    /// Print-ready PDF map (A4 landscape).
    Pdf,
}

/// Exported file.
#[derive(Debug, Clone)]
pub struct ExportFile {
    /// Suggested filename.
    pub filename: String,
    /// Media type.
    pub media_type: &'static str,
    /// Bytes.
    pub bytes: Vec<u8>,
    /// Receipt.
    pub receipt: ExportReceipt,
}

/// What was exported.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportReceipt {
    /// Format.
    pub format: ExportFormat,
    /// Layer digest.
    pub layer_digest: String,
    /// CRS written.
    pub crs: String,
    /// Features written.
    pub feature_count: usize,
    /// `sha256:` of the output bytes.
    pub output_sha256: String,
    /// Attribution carried into the file.
    pub attribution: Option<String>,
}

/// Map styling for PDF export.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MapOptions {
    /// Map title (defaults to the layer name).
    pub title: Option<String>,
    /// Subtitle / description.
    pub subtitle: Option<String>,
    /// Thematic classification to colour features.
    pub classification: Option<Classification>,
    /// Single fill colour when not classified.
    pub color: Option<String>,
}

fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.trim_matches('_').is_empty() {
        "layer".into()
    } else {
        cleaned
    }
}

/// Export a layer.
pub fn export(layer: &Layer, format: ExportFormat, map: &MapOptions) -> Result<ExportFile> {
    let stem = safe_name(&layer.name);
    let (bytes, filename, media_type, crs) = match format {
        ExportFormat::Geojson => (
            geojson_io::write(layer, false)?.into_bytes(),
            format!("{stem}.geojson"),
            "application/geo+json",
            "EPSG:4326".to_string(),
        ),
        ExportFormat::Csv => (
            write_csv(layer),
            format!("{stem}.csv"),
            "text/csv; charset=utf-8",
            layer.crs.clone(),
        ),
        ExportFormat::Geopackage => (
            gpkg_io::write(layer)?,
            format!("{stem}.gpkg"),
            "application/geopackage+sqlite3",
            layer.crs.clone(),
        ),
        ExportFormat::Geoparquet => (
            geoparquet_io::write(layer)?,
            format!("{stem}.parquet"),
            "application/vnd.apache.parquet",
            layer.crs.clone(),
        ),
        ExportFormat::Pdf => (
            write_pdf(layer, map)?,
            format!("{stem}.pdf"),
            "application/pdf",
            layer.crs.clone(),
        ),
    };
    let receipt = ExportReceipt {
        format,
        layer_digest: layer.digest(),
        crs,
        feature_count: layer.features.len(),
        output_sha256: sha256_bytes(&bytes),
        attribution: layer.provenance.attribution.clone(),
    };
    Ok(ExportFile {
        filename,
        media_type,
        bytes,
        receipt,
    })
}

/// Display payload for map clients: EPSG:4326 GeoJSON carrying only the
/// feature ID (attributes come from the table/pick APIs). Layers with more
/// than `vertex_budget` vertices are simplified (Douglas–Peucker) for
/// display only; the tolerance in degrees is returned so clients can say so.
pub fn display_geojson(layer: &Layer, vertex_budget: usize) -> Result<(String, Option<f64>)> {
    use geo::{CoordsIter, Simplify};
    let wgs84 = proj::lookup_epsg(4326)?;
    let mut display = layer.reprojected(&wgs84)?;
    let vertices: usize = display
        .features
        .iter()
        .filter_map(|f| f.geometry.as_ref())
        .map(|g| g.coords_count())
        .sum();
    let tolerance = match (vertices > vertex_budget, display.bbox()) {
        (true, Some(b)) => {
            let diagonal = ((b[2] - b[0]).powi(2) + (b[3] - b[1]).powi(2)).sqrt();
            Some(diagonal / 4000.0 * (vertices as f64 / vertex_budget as f64).sqrt())
        }
        _ => None,
    };
    for feature in &mut display.features {
        feature.properties =
            std::collections::BTreeMap::from([("__id".to_string(), Value::from(feature.id))]);
        if let (Some(epsilon), Some(geometry)) = (tolerance, feature.geometry.as_mut()) {
            let simplified = match &*geometry {
                Geometry::Polygon(p) => Geometry::Polygon(p.simplify(epsilon)),
                Geometry::MultiPolygon(m) => Geometry::MultiPolygon(m.simplify(epsilon)),
                Geometry::LineString(l) => Geometry::LineString(l.simplify(epsilon)),
                Geometry::MultiLineString(m) => Geometry::MultiLineString(m.simplify(epsilon)),
                other => other.clone(),
            };
            *geometry = simplified;
        }
    }
    display.fields.clear();
    Ok((geojson_io::write(&display, true)?, tolerance))
}

fn csv_cell(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

fn write_csv(layer: &Layer) -> Vec<u8> {
    let mut out = String::from("\u{feff}");
    let mut header: Vec<String> = layer.fields.iter().map(|f| csv_cell(&f.name)).collect();
    header.push("geometry_wkt".into());
    header.push("crs".into());
    out.push_str(&header.join(","));
    out.push_str("\r\n");
    for feature in &layer.features {
        let mut row: Vec<String> = layer
            .fields
            .iter()
            .map(|field| match feature.properties.get(&field.name) {
                None | Some(Value::Null) => String::new(),
                Some(Value::String(s)) => csv_cell(s),
                Some(other) => csv_cell(&other.to_string()),
            })
            .collect();
        row.push(csv_cell(
            &feature
                .geometry
                .as_ref()
                .map(wkt::write)
                .unwrap_or_default(),
        ));
        row.push(layer.crs.clone());
        out.push_str(&row.join(","));
        out.push_str("\r\n");
    }
    out.into_bytes()
}

// ---------------------------------------------------------------------------
// PDF
// ---------------------------------------------------------------------------

const PAGE_W: f64 = 842.0;
const PAGE_H: f64 = 595.0;
const MARGIN: f64 = 28.0;

fn hex_color(text: &str) -> (f64, f64, f64) {
    let hex = text.trim_start_matches('#');
    let channel = |i: usize| {
        u8::from_str_radix(hex.get(i..i + 2).unwrap_or("80"), 16).unwrap_or(128) as f64 / 255.0
    };
    (channel(0), channel(2), channel(4))
}

/// UTF-16BE hex string for the Adobe-Japan1 UniJIS-UTF16-H CMap.
fn pdf_text(text: &str) -> String {
    let mut hex = String::from("<");
    for unit in text.encode_utf16() {
        hex.push_str(&format!("{unit:04X}"));
    }
    hex.push('>');
    hex
}

/// WinAnsiEncoding byte for characters Helvetica can draw.
fn winansi(c: char) -> Option<u8> {
    match c as u32 {
        code @ (0x20..=0x7E | 0xA0..=0xFF) => Some(code as u8),
        0x2012 | 0x2013 => Some(0x96),
        0x2014 => Some(0x97),
        0x2018 => Some(0x91),
        0x2019 => Some(0x92),
        0x201C => Some(0x93),
        0x201D => Some(0x94),
        0x2022 => Some(0x95),
        0x2026 => Some(0x85),
        _ => None,
    }
}

/// Text operators that draw Latin runs in Helvetica (`/F2`, proportional,
/// metrics built into every viewer) and everything else in the Japanese CID
/// font (`/F1`). The text position advances across font switches.
fn text_runs(text: &str, size: f64) -> String {
    let mut out = String::new();
    let mut latin = String::new();
    let mut cjk = String::new();
    for c in text.chars() {
        match winansi(c) {
            Some(byte) => {
                if !cjk.is_empty() {
                    out.push_str(&format!(
                        "/F1 {size:.1} Tf {} Tj ",
                        pdf_text(&std::mem::take(&mut cjk))
                    ));
                }
                match byte {
                    b'(' | b')' | b'\\' => {
                        latin.push('\\');
                        latin.push(byte as char);
                    }
                    0x20..=0x7E => latin.push(byte as char),
                    _ => latin.push_str(&format!("\\{byte:03o}")),
                }
            }
            None => {
                if !latin.is_empty() {
                    out.push_str(&format!(
                        "/F2 {size:.1} Tf ({}) Tj ",
                        std::mem::take(&mut latin)
                    ));
                }
                cjk.push(c);
            }
        }
    }
    if !latin.is_empty() {
        out.push_str(&format!("/F2 {size:.1} Tf ({latin}) Tj "));
    }
    if !cjk.is_empty() {
        out.push_str(&format!("/F1 {size:.1} Tf {} Tj ", pdf_text(&cjk)));
    }
    out
}

/// Approximate text width in points (CJK full width, Latin ~0.55 em).
fn text_width(text: &str, size: f64) -> f64 {
    text.chars()
        .map(|c| if winansi(c).is_some() { 0.55 } else { 1.0 })
        .sum::<f64>()
        * size
}

fn fit_text(text: &str, size: f64, max_width: f64) -> String {
    if text_width(text, size) <= max_width {
        return text.to_string();
    }
    let mut out = String::new();
    for c in text.chars() {
        if text_width(&format!("{out}{c}…"), size) > max_width {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

struct Canvas {
    ops: String,
}

impl Canvas {
    fn text(&mut self, x: f64, y: f64, size: f64, text: &str) {
        self.ops.push_str(&format!(
            "BT {x:.2} {y:.2} Td {}ET\n",
            text_runs(text, size)
        ));
    }

    fn fill_rgb(&mut self, color: (f64, f64, f64)) {
        self.ops.push_str(&format!(
            "{:.3} {:.3} {:.3} rg\n",
            color.0, color.1, color.2
        ));
    }

    fn stroke_rgb(&mut self, color: (f64, f64, f64)) {
        self.ops.push_str(&format!(
            "{:.3} {:.3} {:.3} RG\n",
            color.0, color.1, color.2
        ));
    }

    fn rect(&mut self, x: f64, y: f64, w: f64, h: f64, op: &str) {
        self.ops
            .push_str(&format!("{x:.2} {y:.2} {w:.2} {h:.2} re {op}\n"));
    }

    fn line_width(&mut self, w: f64) {
        self.ops.push_str(&format!("{w:.2} w\n"));
    }
}

struct Viewport {
    display: CrsInfo,
    source: CrsInfo,
    min_x: f64,
    min_y: f64,
    scale: f64,
    origin_x: f64,
    origin_y: f64,
}

impl Viewport {
    fn map(&self, c: Coord<f64>) -> Option<(f64, f64)> {
        let (x, y) = proj::transform_coord(&self.source, &self.display, c.x, c.y).ok()?;
        Some((
            self.origin_x + (x - self.min_x) * self.scale,
            self.origin_y + (y - self.min_y) * self.scale,
        ))
    }

    fn path(&self, line: &LineString<f64>, close: bool) -> String {
        let mut out = String::new();
        for (i, c) in line.0.iter().enumerate() {
            if let Some((x, y)) = self.map(*c) {
                out.push_str(&format!(
                    "{x:.2} {y:.2} {}\n",
                    if i == 0 { "m" } else { "l" }
                ));
            }
        }
        if close {
            out.push_str("h\n");
        }
        out
    }

    fn polygon(&self, polygon: &Polygon<f64>) -> String {
        let mut out = self.path(polygon.exterior(), true);
        for hole in polygon.interiors() {
            out.push_str(&self.path(hole, true));
        }
        out
    }
}

fn circle(x: f64, y: f64, r: f64) -> String {
    let k = 0.552_284_75 * r;
    format!(
        "{:.2} {:.2} m {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c h\n",
        x + r, y,
        x + r, y + k, x + k, y + r, x, y + r,
        x - k, y + r, x - r, y + k, x - r, y,
        x - r, y - k, x - k, y - r, x, y - r,
        x + k, y - r, x + r, y - k, x + r, y
    )
}

fn draw_geometry(
    canvas: &mut Canvas,
    viewport: &Viewport,
    geometry: &Geometry<f64>,
    fill: (f64, f64, f64),
) {
    match geometry {
        Geometry::Polygon(p) => {
            canvas.fill_rgb(fill);
            canvas.ops.push_str(&viewport.polygon(p));
            canvas.ops.push_str("B*\n");
        }
        Geometry::MultiPolygon(m) => {
            canvas.fill_rgb(fill);
            for p in &m.0 {
                canvas.ops.push_str(&viewport.polygon(p));
            }
            canvas.ops.push_str("B*\n");
        }
        Geometry::Rect(r) => {
            draw_geometry(canvas, viewport, &Geometry::Polygon(r.to_polygon()), fill)
        }
        Geometry::Triangle(t) => {
            draw_geometry(canvas, viewport, &Geometry::Polygon(t.to_polygon()), fill)
        }
        Geometry::LineString(l) => {
            canvas.stroke_rgb(fill);
            canvas.line_width(1.6);
            canvas.ops.push_str(&viewport.path(l, false));
            canvas.ops.push_str("S\n0.2 0.2 0.2 RG 0.4 w\n");
        }
        Geometry::MultiLineString(m) => {
            for l in &m.0 {
                draw_geometry(canvas, viewport, &Geometry::LineString(l.clone()), fill);
            }
        }
        Geometry::Line(l) => draw_geometry(
            canvas,
            viewport,
            &Geometry::LineString(LineString(vec![l.start, l.end])),
            fill,
        ),
        Geometry::Point(p) => {
            if let Some((x, y)) = viewport.map(p.0) {
                canvas.fill_rgb(fill);
                canvas.ops.push_str(&circle(x, y, 3.2));
                canvas.ops.push_str("B\n");
            }
        }
        Geometry::MultiPoint(m) => {
            for p in &m.0 {
                draw_geometry(canvas, viewport, &Geometry::Point(*p), fill);
            }
        }
        Geometry::GeometryCollection(c) => {
            for g in &c.0 {
                draw_geometry(canvas, viewport, g, fill);
            }
        }
    }
}

fn nice_length(max_metres: f64) -> f64 {
    let exponent = max_metres.log10().floor();
    let base = 10f64.powf(exponent);
    for m in [5.0, 2.0, 1.0] {
        if m * base <= max_metres {
            return m * base;
        }
    }
    base
}

fn format_distance(metres: f64) -> String {
    if metres >= 1000.0 {
        let km = metres / 1000.0;
        if km.fract() == 0.0 {
            format!("{km:.0} km")
        } else {
            format!("{km} km")
        }
    } else {
        format!("{metres:.0} m")
    }
}

fn write_pdf(layer: &Layer, options: &MapOptions) -> Result<Vec<u8>> {
    let source = layer.crs_info()?;
    let bbox = layer.bbox_wgs84();
    // Display in a conformal metric CRS so shapes and the scale bar are true.
    let display = match (&source.projection, bbox) {
        (proj::Projection::TransverseMercator { .. }, _) => source.clone(),
        (_, Some(b)) => proj::metric_crs_for((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0),
        (_, None) => source.clone(),
    };
    let mut canvas = Canvas { ops: String::new() };
    let title = options.title.clone().unwrap_or_else(|| layer.name.clone());

    // Frame geometry.
    let legend_w = 190.0;
    let footer_h = 58.0;
    let title_h = options.subtitle.as_ref().map_or(34.0, |_| 48.0);
    let frame_x = MARGIN;
    let frame_y = MARGIN + footer_h;
    let frame_w = PAGE_W - 2.0 * MARGIN - legend_w - 12.0;
    let frame_h = PAGE_H - 2.0 * MARGIN - footer_h - title_h;

    // Title.
    canvas.fill_rgb((0.1, 0.1, 0.12));
    canvas.text(
        MARGIN,
        PAGE_H - MARGIN - 18.0,
        18.0,
        &fit_text(&title, 18.0, PAGE_W - 2.0 * MARGIN),
    );
    if let Some(subtitle) = &options.subtitle {
        canvas.fill_rgb((0.35, 0.35, 0.4));
        canvas.text(
            MARGIN,
            PAGE_H - MARGIN - 36.0,
            10.0,
            &fit_text(subtitle, 10.0, PAGE_W - 2.0 * MARGIN),
        );
    }

    // Map frame.
    canvas.fill_rgb((0.97, 0.97, 0.96));
    canvas.stroke_rgb((0.6, 0.6, 0.6));
    canvas.line_width(0.6);
    canvas.rect(frame_x, frame_y, frame_w, frame_h, "B");

    let mut display_extent: Option<[f64; 4]> = None;
    for feature in &layer.features {
        let Some(geometry) = &feature.geometry else {
            continue;
        };
        let Ok(projected) = proj::transform_geometry(&source, &display, geometry) else {
            continue;
        };
        if let Some(r) = projected.bounding_rect() {
            display_extent = Some(match display_extent {
                None => [r.min().x, r.min().y, r.max().x, r.max().y],
                Some(e) => [
                    e[0].min(r.min().x),
                    e[1].min(r.min().y),
                    e[2].max(r.max().x),
                    e[3].max(r.max().y),
                ],
            });
        }
    }
    let mut scale_note = String::new();
    if let Some(mut e) = display_extent {
        let pad_x = ((e[2] - e[0]) * 0.05).max(50.0);
        let pad_y = ((e[3] - e[1]) * 0.05).max(50.0);
        e = [e[0] - pad_x, e[1] - pad_y, e[2] + pad_x, e[3] + pad_y];
        let scale = (frame_w / (e[2] - e[0])).min(frame_h / (e[3] - e[1]));
        let used_w = (e[2] - e[0]) * scale;
        let used_h = (e[3] - e[1]) * scale;
        let viewport = Viewport {
            display: display.clone(),
            source: source.clone(),
            min_x: e[0],
            min_y: e[1],
            scale,
            origin_x: frame_x + (frame_w - used_w) / 2.0,
            origin_y: frame_y + (frame_h - used_h) / 2.0,
        };
        canvas.ops.push_str("q\n");
        canvas.ops.push_str(&format!(
            "{frame_x:.2} {frame_y:.2} {frame_w:.2} {frame_h:.2} re W n\n"
        ));
        canvas.stroke_rgb((0.25, 0.25, 0.28));
        canvas.line_width(0.4);
        let default_fill = hex_color(options.color.as_deref().unwrap_or("#4f8fd6"));
        for feature in &layer.features {
            let Some(geometry) = &feature.geometry else {
                continue;
            };
            let fill = options
                .classification
                .as_ref()
                .and_then(|c| {
                    c.assignments
                        .get(&feature.id)
                        .copied()
                        .flatten()
                        .map(|i| hex_color(&c.classes[i].color))
                })
                .unwrap_or(if options.classification.is_some() {
                    (0.85, 0.85, 0.85)
                } else {
                    default_fill
                });
            draw_geometry(&mut canvas, &viewport, geometry, fill);
        }
        canvas.ops.push_str("Q\n");

        // Scale bar (true in the conformal display CRS near its centre).
        let bar_m = nice_length((frame_w * 0.25) / scale);
        let bar_pt = bar_m * scale;
        let (sx, sy) = (frame_x + 12.0, frame_y + 12.0);
        canvas.fill_rgb((1.0, 1.0, 1.0));
        canvas.stroke_rgb((0.2, 0.2, 0.2));
        canvas.line_width(0.6);
        canvas.rect(sx - 4.0, sy - 4.0, bar_pt + 60.0, 20.0, "B");
        canvas.fill_rgb((0.15, 0.15, 0.15));
        canvas.rect(sx, sy, bar_pt / 2.0, 4.0, "f");
        canvas.fill_rgb((1.0, 1.0, 1.0));
        canvas.rect(sx + bar_pt / 2.0, sy, bar_pt / 2.0, 4.0, "B");
        canvas.fill_rgb((0.15, 0.15, 0.15));
        canvas.text(sx + bar_pt + 6.0, sy, 8.0, &format_distance(bar_m));
        canvas.text(sx, sy + 6.0, 6.5, "0");
        scale_note = format!("縮尺バー・形状は {} 上で描画", display.id);

        // North arrow (grid north of the conformal display CRS).
        let (nx, ny) = (frame_x + frame_w - 22.0, frame_y + frame_h - 38.0);
        canvas.fill_rgb((0.15, 0.15, 0.15));
        canvas.ops.push_str(&format!(
            "{nx:.2} {:.2} m {:.2} {ny:.2} l {nx:.2} {:.2} l {:.2} {ny:.2} l h f\n",
            ny + 24.0,
            nx + 7.0,
            ny + 6.0,
            nx - 7.0
        ));
        canvas.text(nx - 3.5, ny + 27.0, 9.0, "N");
    } else {
        canvas.fill_rgb((0.4, 0.4, 0.4));
        canvas.text(
            frame_x + 20.0,
            frame_y + frame_h / 2.0,
            12.0,
            "このレイヤには地図に描ける図形がありません",
        );
    }

    // Legend.
    let lx = PAGE_W - MARGIN - legend_w;
    let mut ly = PAGE_H - MARGIN - title_h - 4.0;
    canvas.fill_rgb((0.1, 0.1, 0.12));
    canvas.text(lx, ly - 12.0, 11.0, "凡例");
    ly -= 30.0;
    if let Some(classification) = &options.classification {
        let heading = match &classification.unit {
            Some(unit) => format!("{} ({unit})", classification.field),
            None => classification.field.clone(),
        };
        canvas.fill_rgb((0.3, 0.3, 0.35));
        canvas.text(lx, ly, 8.5, &fit_text(&heading, 8.5, legend_w));
        ly -= 16.0;
        for class in &classification.classes {
            if ly < frame_y + 10.0 {
                break;
            }
            canvas.fill_rgb(hex_color(&class.color));
            canvas.stroke_rgb((0.3, 0.3, 0.3));
            canvas.line_width(0.4);
            canvas.rect(lx, ly - 2.0, 16.0, 10.0, "B");
            canvas.fill_rgb((0.15, 0.15, 0.15));
            canvas.text(
                lx + 22.0,
                ly,
                8.0,
                &fit_text(
                    &format!("{}  ({})", class.label, class.count),
                    8.0,
                    legend_w - 24.0,
                ),
            );
            ly -= 15.0;
        }
    } else {
        canvas.fill_rgb(hex_color(options.color.as_deref().unwrap_or("#4f8fd6")));
        canvas.stroke_rgb((0.3, 0.3, 0.3));
        canvas.rect(lx, ly - 2.0, 16.0, 10.0, "B");
        canvas.fill_rgb((0.15, 0.15, 0.15));
        canvas.text(
            lx + 22.0,
            ly,
            8.0,
            &fit_text(
                &format!("{} ({} 件)", layer.name, layer.features.len()),
                8.0,
                legend_w - 24.0,
            ),
        );
        ly -= 15.0;
    }
    ly -= 10.0;
    canvas.fill_rgb((0.35, 0.35, 0.4));
    for line in layer.provenance.notes.iter().take(6) {
        if ly < frame_y + 10.0 {
            break;
        }
        canvas.text(lx, ly, 6.5, &fit_text(line, 6.5, legend_w));
        ly -= 10.0;
    }

    // Footer: CRS, sources, digests.
    let crs_line = format!(
        "座標参照系: {} ({})  ·  {}",
        source.id, source.name, scale_note
    );
    let source_line = format!(
        "出典: {}{}",
        layer
            .provenance
            .attribution
            .clone()
            .unwrap_or_else(|| layer.provenance.source_uri.clone()),
        layer
            .provenance
            .license
            .as_ref()
            .map(|l| format!("  ·  ライセンス: {l}"))
            .unwrap_or_default()
    );
    let digest_line = format!(
        "データ: {}{}  ·  作成: GeneGIS {}",
        layer.digest(),
        layer
            .provenance
            .workflow_digest
            .as_ref()
            .map(|d| format!("  ·  ワークフロー: {d}"))
            .unwrap_or_default(),
        chrono::Utc::now().format("%Y-%m-%d")
    );
    canvas.fill_rgb((0.3, 0.3, 0.33));
    let width = PAGE_W - 2.0 * MARGIN;
    canvas.text(MARGIN, MARGIN + 38.0, 7.5, &fit_text(&crs_line, 7.5, width));
    canvas.text(
        MARGIN,
        MARGIN + 24.0,
        7.5,
        &fit_text(&source_line, 7.5, width),
    );
    canvas.text(
        MARGIN,
        MARGIN + 10.0,
        6.0,
        &fit_text(&digest_line, 6.0, width),
    );

    assemble_pdf(&canvas.ops, &title)
}

fn assemble_pdf(content: &str, title: &str) -> Result<Vec<u8>> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(content.as_bytes())
        .map_err(|e| ToolkitError::export("pdf", e.to_string()))?;
    let compressed = encoder
        .finish()
        .map_err(|e| ToolkitError::export("pdf", e.to_string()))?;

    let objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_W} {PAGE_H}] /Resources << /Font << /F1 5 0 R /F2 9 0 R >> >> /Contents 4 0 R >>").into_bytes(),
        {
            let mut stream = format!("<< /Length {} /Filter /FlateDecode >>\nstream\n", compressed.len()).into_bytes();
            stream.extend_from_slice(&compressed);
            stream.extend_from_slice(b"\nendstream");
            stream
        },
        // Non-embedded Adobe-Japan1 CID font: viewers substitute an installed
        // Japanese Gothic face. ASCII CIDs 1–95 are set to half width.
        b"<< /Type /Font /Subtype /Type0 /BaseFont /KozGoPr6N-Medium /Encoding /UniJIS-UTF16-H /DescendantFonts [6 0 R] >>".to_vec(),
        b"<< /Type /Font /Subtype /CIDFontType0 /BaseFont /KozGoPr6N-Medium /CIDSystemInfo << /Registry (Adobe) /Ordering (Japan1) /Supplement 6 >> /FontDescriptor 7 0 R /DW 1000 /W [1 95 500] >>".to_vec(),
        b"<< /Type /FontDescriptor /FontName /KozGoPr6N-Medium /Flags 4 /FontBBox [-149 -374 1265 1127] /ItalicAngle 0 /Ascent 880 /Descent -120 /CapHeight 763 /StemV 80 >>".to_vec(),
        // Document-information strings need a UTF-16BE BOM (content-stream
        // CID strings must not have one).
        format!("<< /Title <FEFF{} /Producer (GeneGIS) /Creator (GeneGIS toolkit) >>", &pdf_text(title)[1..]).into_bytes(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
    ];
    let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R /Info 8 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::{import_bytes, ImportOptions};
    use crate::table::{classify, ClassMethod, ClassifyRequest};

    fn wards() -> Layer {
        let bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/nagoya-population-density/data/nagoya-wards.geojson"
        ))
        .unwrap();
        let (mut layer, _) =
            import_bytes("nagoya-wards.geojson", &bytes, &ImportOptions::default()).unwrap();
        layer.provenance.attribution = Some("国土数値情報 N03 / 名古屋市".into());
        layer
    }

    #[test]
    fn exports_every_format_with_receipts() {
        let layer = wards();
        for format in [
            ExportFormat::Geojson,
            ExportFormat::Csv,
            ExportFormat::Geopackage,
            ExportFormat::Geoparquet,
            ExportFormat::Pdf,
        ] {
            let file = export(&layer, format, &MapOptions::default()).unwrap();
            assert!(!file.bytes.is_empty(), "{format:?}");
            assert_eq!(file.receipt.feature_count, 16);
            assert_eq!(file.receipt.layer_digest, layer.digest());
        }
    }

    #[test]
    fn display_payload_is_lean_and_simplified_when_large() {
        let layer = wards();
        let (full, tolerance) = display_geojson(&layer, usize::MAX).unwrap();
        assert!(tolerance.is_none());
        assert!(
            !full.contains("population_source"),
            "attributes must not be shipped for display"
        );
        let (small, tolerance) = display_geojson(&layer, 2_000).unwrap();
        assert!(tolerance.is_some());
        assert!(
            small.len() < full.len() / 2,
            "{} vs {}",
            small.len(),
            full.len()
        );
    }

    #[test]
    fn csv_is_excel_friendly_and_reimportable() {
        let layer = wards();
        let file = export(&layer, ExportFormat::Csv, &MapOptions::default()).unwrap();
        assert!(file.bytes.starts_with("\u{feff}".as_bytes()));
        let (back, _) =
            import_bytes(&file.filename, &file.bytes, &ImportOptions::default()).unwrap();
        assert_eq!(back.features.len(), 16);
        assert_eq!(back.crs_status, crate::CrsStatus::Declared);
        assert!(back.field("crs").is_none());
        // CSV carries no types: a text code without a leading zero comes back
        // numeric. GeoPackage/GeoParquet preserve types exactly.
        assert_eq!(
            back.features[0].properties["ward_code"]
                .to_string()
                .trim_matches('"'),
            "23101"
        );
    }

    #[test]
    fn pdf_is_well_formed_with_legend_and_provenance() {
        let layer = wards();
        let classification = classify(
            &layer,
            &ClassifyRequest {
                field: "population".into(),
                method: ClassMethod::Quantile,
                classes: 5,
            },
        )
        .unwrap();
        let options = MapOptions {
            title: Some("名古屋市の区別人口".into()),
            subtitle: Some("令和2年国勢調査".into()),
            classification: Some(classification),
            color: None,
        };
        let file = export(&layer, ExportFormat::Pdf, &options).unwrap();
        let bytes = &file.bytes;
        assert!(bytes.starts_with(b"%PDF-1.7"));
        assert!(bytes.ends_with(b"%%EOF\n"));
        let text = String::from_utf8_lossy(bytes);
        assert!(text.contains("/UniJIS-UTF16-H"));
        assert!(text.contains("/Helvetica"));
        assert_eq!(
            text_runs("EPSG:4326 (WGS 84) · 地図 km²", 8.0),
            "/F2 8.0 Tf (EPSG:4326 \\(WGS 84\\) \\267 ) Tj /F1 8.0 Tf <573056F3> Tj /F2 8.0 Tf ( km\\262) Tj "
        );
        let startxref: usize = text
            .rsplit("startxref\n")
            .next()
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(&bytes[startxref..startxref + 4], b"xref");
        // The document title is UTF-16BE with a BOM in the Info dictionary.
        assert!(text.contains(&format!("<FEFF{}", &pdf_text("名古屋市の区別人口")[1..])));
    }
}
