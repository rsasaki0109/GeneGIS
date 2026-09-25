//! User data import: format detection, decoding, CRS resolution, and schema
//! inference. Imports never guess silently — every assumption is recorded in
//! the [`ImportReport`], and an unknowable CRS fails closed with
//! [`ToolkitError::CrsRequired`](crate::ToolkitError::CrsRequired).

mod csv;
mod shapefile;

use serde::{Deserialize, Serialize};

use crate::error::{Result, ToolkitError};
use crate::layer::{sha256_bytes, Layer};
use crate::{geojson_io, geoparquet_io, gpkg_io};

/// Supported import formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportFormat {
    /// GeoJSON / JSON FeatureCollection.
    Geojson,
    /// Delimited text (CSV/TSV) with coordinate or WKT columns.
    Csv,
    /// Zipped ESRI Shapefile (`.shp` + `.dbf` + optional `.prj`/`.cpg`).
    Shapefile,
    /// OGC GeoPackage.
    Geopackage,
    /// GeoParquet (WKB encoding).
    Geoparquet,
}

impl ImportFormat {
    /// Stable format name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Geojson => "geojson",
            Self::Csv => "csv",
            Self::Shapefile => "shapefile",
            Self::Geopackage => "geopackage",
            Self::Geoparquet => "geoparquet",
        }
    }
}

/// Caller-supplied import hints. Every field is optional.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ImportOptions {
    /// Layer display name (defaults to the file stem).
    pub name: Option<String>,
    /// Force a format instead of detecting it.
    pub format: Option<ImportFormat>,
    /// Source CRS, overriding whatever the file declares or implies.
    pub crs: Option<String>,
    /// CSV: longitude / easting column.
    pub x_field: Option<String>,
    /// CSV: latitude / northing column.
    pub y_field: Option<String>,
    /// CSV: WKT geometry column.
    pub wkt_field: Option<String>,
    /// Text encoding (`utf-8`, `shift_jis`, …) for CSV and DBF.
    pub encoding: Option<String>,
    /// GeoPackage: feature table to read.
    pub table: Option<String>,
    /// License of the data.
    pub license: Option<String>,
    /// Attribution that must accompany outputs.
    pub attribution: Option<String>,
}

/// Row-level problem that did not abort the import.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkippedRecord {
    /// 1-based row/record number in the source.
    pub record: usize,
    /// Why it was skipped.
    pub reason: String,
}

/// What happened during an import.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportReport {
    /// Detected format.
    pub format: String,
    /// `sha256:` digest of the uploaded bytes.
    pub source_sha256: String,
    /// Byte length of the upload.
    pub source_bytes: usize,
    /// Text encoding used, when relevant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    /// Records that could not be converted, with reasons.
    #[serde(default)]
    pub skipped: Vec<SkippedRecord>,
    /// Assumptions and normalisation notes.
    #[serde(default)]
    pub notes: Vec<String>,
}

/// Import `bytes` named `filename`.
pub fn import_bytes(
    filename: &str,
    bytes: &[u8],
    options: &ImportOptions,
) -> Result<(Layer, ImportReport)> {
    let format = match options.format {
        Some(format) => format,
        None => detect_format(filename, bytes)?,
    };
    let mut report = ImportReport {
        format: format.as_str().into(),
        source_sha256: sha256_bytes(bytes),
        source_bytes: bytes.len(),
        ..Default::default()
    };
    let mut layer = match format {
        ImportFormat::Geojson => {
            let text = std::str::from_utf8(strip_bom(bytes))
                .map_err(|_| ToolkitError::import("geojson", "GeoJSON must be UTF-8 (RFC 8259)"))?;
            report.encoding = Some("utf-8".into());
            geojson_io::read(text, options, &mut report)?
        }
        ImportFormat::Csv => csv::read(bytes, options, &mut report)?,
        ImportFormat::Shapefile => shapefile::read_zip(bytes, options, &mut report)?,
        ImportFormat::Geopackage => gpkg_io::read(bytes, options, &mut report)?,
        ImportFormat::Geoparquet => geoparquet_io::read(bytes, options, &mut report)?,
    };
    if layer.features.is_empty() {
        return Err(ToolkitError::import(
            format.as_str(),
            if report.skipped.is_empty() {
                "the file contains no features".to_string()
            } else {
                format!(
                    "no usable features; first problem: {}",
                    report.skipped[0].reason
                )
            },
        ));
    }
    validate_coordinates(&layer)?;
    let invalid = layer.invalid_feature_ids();
    if !invalid.is_empty() {
        report.notes.push(format!(
            "{} features have invalid polygons (self-intersections or crossing rings): ids {:?}; \
             polygon operations reject them until a make_valid step repairs them",
            invalid.len(),
            &invalid[..invalid.len().min(20)]
        ));
    }
    if options.name.is_none() {
        let stem = filename
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(filename)
            .rsplit_once('.')
            .map(|(stem, _)| stem)
            .unwrap_or(filename);
        if !stem.is_empty()
            && (layer.name.is_empty() || layer.name == format.as_str() || layer.name == "unnamed")
        {
            layer.name = stem.to_string();
        }
    }
    layer.provenance.source_uri = format!("upload://{}", sanitize_filename(filename));
    layer.provenance.source_sha256 = Some(report.source_sha256.clone());
    layer.provenance.format = format.as_str().into();
    layer.provenance.license = options.license.clone();
    layer.provenance.attribution = options.attribution.clone();
    layer.provenance.notes = report.notes.clone();
    if !report.skipped.is_empty() {
        layer.provenance.notes.push(format!(
            "{} source records skipped (see import report)",
            report.skipped.len()
        ));
    }
    Ok((layer, report))
}

/// Detect the format from the file extension, then from magic bytes.
pub fn detect_format(filename: &str, bytes: &[u8]) -> Result<ImportFormat> {
    let lower = filename.to_ascii_lowercase();
    let by_extension = [
        (".geojson", ImportFormat::Geojson),
        (".json", ImportFormat::Geojson),
        (".csv", ImportFormat::Csv),
        (".tsv", ImportFormat::Csv),
        (".txt", ImportFormat::Csv),
        (".zip", ImportFormat::Shapefile),
        (".gpkg", ImportFormat::Geopackage),
        (".parquet", ImportFormat::Geoparquet),
        (".geoparquet", ImportFormat::Geoparquet),
    ];
    if let Some((_, format)) = by_extension.iter().find(|(ext, _)| lower.ends_with(ext)) {
        return Ok(*format);
    }
    if lower.ends_with(".shp") {
        return Err(ToolkitError::UnsupportedFormat(
            "a bare .shp has no attributes or CRS; zip the .shp, .shx, .dbf, .prj (and .cpg) files together".into(),
        ));
    }
    if bytes.starts_with(b"SQLite format 3\0") {
        return Ok(ImportFormat::Geopackage);
    }
    if bytes.starts_with(b"PAR1") {
        return Ok(ImportFormat::Geoparquet);
    }
    if bytes.starts_with(b"PK\x03\x04") {
        return Ok(ImportFormat::Shapefile);
    }
    let head = strip_bom(&bytes[..bytes.len().min(64)]);
    if head.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{') {
        return Ok(ImportFormat::Geojson);
    }
    Err(ToolkitError::UnsupportedFormat(format!(
        "{filename}: use GeoJSON, CSV/TSV, zipped Shapefile, GeoPackage, or GeoParquet"
    )))
}

pub(crate) fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes)
}

/// Decode text, honouring an explicit encoding; otherwise UTF-8 (with or
/// without BOM), falling back to Shift_JIS (Windows-31J) which is common for
/// Japanese government data.
pub(crate) fn decode_text(bytes: &[u8], explicit: Option<&str>) -> Result<(String, String)> {
    if let Some(label) = explicit {
        let encoding = encoding_rs::Encoding::for_label(label.trim().as_bytes())
            .ok_or_else(|| ToolkitError::import("text", format!("unknown encoding {label}")))?;
        let (text, _, had_errors) = encoding.decode(bytes);
        if had_errors {
            return Err(ToolkitError::import(
                "text",
                format!("bytes are not valid {}", encoding.name()),
            ));
        }
        return Ok((text.into_owned(), encoding.name().to_ascii_lowercase()));
    }
    if let Ok(text) = std::str::from_utf8(strip_bom(bytes)) {
        return Ok((text.to_string(), "utf-8".into()));
    }
    let (text, _, had_errors) = encoding_rs::SHIFT_JIS.decode(bytes);
    if had_errors {
        return Err(ToolkitError::import(
            "text",
            "text is neither UTF-8 nor Shift_JIS; specify the encoding explicitly",
        ));
    }
    Ok((text.into_owned(), "shift_jis".into()))
}

/// Whether every coordinate fits the longitude/latitude domain.
pub(crate) fn coordinates_look_geographic(layer: &Layer) -> bool {
    use geo::CoordsIter;
    layer
        .features
        .iter()
        .filter_map(|f| f.geometry.as_ref())
        .all(|g| {
            g.coords_iter()
                .all(|c| (-180.0..=180.0).contains(&c.x) && (-90.0..=90.0).contains(&c.y))
        })
}

fn validate_coordinates(layer: &Layer) -> Result<()> {
    use geo::CoordsIter;
    let crs = layer.crs_info()?;
    for feature in &layer.features {
        if let Some(geometry) = &feature.geometry {
            for c in geometry.coords_iter() {
                if !c.x.is_finite() || !c.y.is_finite() {
                    return Err(ToolkitError::InvalidCoordinate(format!(
                        "feature {} has a non-finite coordinate",
                        feature.id
                    )));
                }
                if crs.is_geographic()
                    && (!(-180.0..=180.0).contains(&c.x) || !(-90.0..=90.0).contains(&c.y))
                {
                    return Err(ToolkitError::InvalidCoordinate(format!(
                        "feature {} has ({}, {}) which is outside {}; the declared CRS is probably wrong",
                        feature.id, c.x, c.y, crs.id
                    )));
                }
            }
        }
    }
    Ok(())
}

fn sanitize_filename(filename: &str) -> String {
    filename
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(filename)
        .chars()
        .map(|c| if c.is_control() { '_' } else { c })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::CrsStatus;

    #[test]
    fn detects_formats_by_name_and_magic() {
        assert_eq!(detect_format("a.csv", b"x,y").unwrap(), ImportFormat::Csv);
        assert_eq!(
            detect_format("blob", b"PAR1....").unwrap(),
            ImportFormat::Geoparquet
        );
        assert_eq!(
            detect_format("blob", b"  {\"type\":1}").unwrap(),
            ImportFormat::Geojson
        );
        assert!(detect_format("roads.shp", b"").is_err());
        assert!(detect_format("image.png", b"\x89PNG").is_err());
    }

    #[test]
    fn geojson_import_records_provenance() {
        let text = br#"{"type":"FeatureCollection","features":[
            {"type":"Feature","properties":{"name":"A","pop":10},"geometry":{"type":"Point","coordinates":[136.9,35.1]}},
            {"type":"Feature","properties":{"name":"B","pop":20.5},"geometry":{"type":"LineString","coordinates":[[136.9,35.1],[137.0,35.2]]}}
        ]}"#;
        let (layer, report) =
            import_bytes("sites.geojson", text, &ImportOptions::default()).unwrap();
        assert_eq!(layer.name, "sites");
        assert_eq!(layer.crs, "EPSG:4326");
        assert_eq!(layer.crs_status, CrsStatus::FormatDefault);
        assert_eq!(layer.features.len(), 2);
        assert_eq!(
            layer.field("pop").unwrap().field_type,
            crate::FieldType::Float
        );
        assert_eq!(
            layer.provenance.source_sha256.as_deref(),
            Some(report.source_sha256.as_str())
        );
        assert_eq!(layer.provenance.source_uri, "upload://sites.geojson");
    }

    #[test]
    fn projected_geojson_without_crs_fails_closed() {
        let text = br#"{"type":"FeatureCollection","features":[
            {"type":"Feature","properties":{},"geometry":{"type":"Point","coordinates":[-26000.0,-92000.0]}}]}"#;
        let error = import_bytes("p.geojson", text, &ImportOptions::default()).unwrap_err();
        assert!(matches!(error, ToolkitError::CrsRequired(_)), "{error}");
        let options = ImportOptions {
            crs: Some("EPSG:6675".into()),
            ..Default::default()
        };
        let (layer, _) = import_bytes("p.geojson", text, &options).unwrap();
        assert_eq!(layer.crs, "EPSG:6675");
        assert_eq!(layer.crs_status, CrsStatus::UserSupplied);
    }

    #[test]
    fn legacy_crs_member_is_honoured() {
        let text = br#"{"type":"FeatureCollection","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::6675"}},"features":[
            {"type":"Feature","properties":{},"geometry":{"type":"Point","coordinates":[-26000.0,-92000.0]}}]}"#;
        let (layer, _) = import_bytes("p.geojson", text, &ImportOptions::default()).unwrap();
        assert_eq!(layer.crs, "EPSG:6675");
        assert_eq!(layer.crs_status, CrsStatus::Declared);
    }

    #[test]
    fn decodes_shift_jis_fallback() {
        let (bytes, _, _) = encoding_rs::SHIFT_JIS.encode("名古屋市");
        let (text, encoding) = decode_text(&bytes, None).unwrap();
        assert_eq!(text, "名古屋市");
        assert_eq!(encoding, "shift_jis");
    }
}
