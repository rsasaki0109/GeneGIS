//! Delimited text import with coordinate-column or WKT detection.

use std::collections::BTreeMap;

use geo_types::{Geometry, Point};
use serde_json::Value;

use super::{coordinates_look_geographic, decode_text, ImportOptions, ImportReport, SkippedRecord};
use crate::error::{Result, ToolkitError};
use crate::layer::{CrsStatus, Feature, Layer};
use crate::{proj, wkt};

const X_NAMES: &[&str] = &[
    "lon",
    "lng",
    "long",
    "longitude",
    "x",
    "経度",
    "東経",
    "x座標",
    "easting",
    "point_x",
];
const Y_NAMES: &[&str] = &[
    "lat", "latitude", "y", "緯度", "北緯", "y座標", "northing", "point_y",
];
const WKT_NAMES: &[&str] = &[
    "wkt",
    "geometry_wkt",
    "geometry",
    "geom",
    "the_geom",
    "shape",
    "ジオメトリ",
];

pub(super) fn read(
    bytes: &[u8],
    options: &ImportOptions,
    report: &mut ImportReport,
) -> Result<Layer> {
    let (text, encoding) = decode_text(bytes, options.encoding.as_deref())?;
    report.encoding = Some(encoding);
    let delimiter = detect_delimiter(&text);
    let mut rows = parse_rows(&text, delimiter)?;
    if rows.is_empty() {
        return Err(ToolkitError::import("csv", "file is empty"));
    }
    let header: Vec<String> = rows
        .remove(0)
        .into_iter()
        .map(|h| h.trim().to_string())
        .collect();
    if header.iter().any(String::is_empty) {
        return Err(ToolkitError::import(
            "csv",
            "header contains an empty column name",
        ));
    }
    let find = |explicit: &Option<String>, names: &[&str]| -> Result<Option<usize>> {
        if let Some(name) = explicit {
            return header
                .iter()
                .position(|h| h == name)
                .map(Some)
                .ok_or_else(|| {
                    ToolkitError::import("csv", format!("column {name} does not exist"))
                });
        }
        Ok(header
            .iter()
            .position(|h| names.contains(&h.to_lowercase().as_str())))
    };
    let wkt_column = find(&options.wkt_field, WKT_NAMES)?;
    let x_column = find(&options.x_field, X_NAMES)?;
    let y_column = find(&options.y_field, Y_NAMES)?;
    let geometry_source = match (wkt_column, x_column, y_column) {
        (Some(w), _, _) if options.x_field.is_none() => GeometrySource::Wkt(w),
        (_, Some(x), Some(y)) => GeometrySource::Xy(x, y),
        (Some(w), _, _) => GeometrySource::Wkt(w),
        _ => {
            return Err(ToolkitError::import(
                "csv",
                format!(
                    "no geometry columns found in [{}]; name them lon/lat (経度/緯度), x/y, or wkt, or pass x_field/y_field",
                    header.join(", ")
                ),
            ))
        }
    };
    match geometry_source {
        GeometrySource::Wkt(w) => report
            .notes
            .push(format!("geometry read from WKT column {}", header[w])),
        GeometrySource::Xy(x, y) => report.notes.push(format!(
            "geometry built from columns {} (x) and {} (y)",
            header[x], header[y]
        )),
    }

    // A `crs` column with one consistent value (as written by GeneGIS CSV
    // export) declares the CRS.
    let crs_column = header.iter().position(|h| h.eq_ignore_ascii_case("crs"));
    let declared_crs = crs_column.and_then(|col| {
        let mut values = rows
            .iter()
            .filter_map(|row| row.get(col))
            .map(|v| v.trim())
            .filter(|v| !v.is_empty());
        let first = values.next()?.to_string();
        values.all(|v| v == first).then_some(first)
    });

    let column_types: Vec<ColumnType> = (0..header.len())
        .map(|col| {
            column_type(
                rows.iter()
                    .filter_map(|row| row.get(col).map(String::as_str)),
            )
        })
        .collect();

    let mut layer = Layer::new(
        options.name.clone().unwrap_or_default(),
        "EPSG:4326",
        CrsStatus::Inferred,
    );
    for (index, row) in rows.iter().enumerate() {
        let record = index + 2; // 1-based incl. header
        if row.iter().all(|cell| cell.trim().is_empty()) {
            continue;
        }
        if row.len() != header.len() {
            report.skipped.push(SkippedRecord {
                record,
                reason: format!("expected {} columns, found {}", header.len(), row.len()),
            });
            continue;
        }
        let geometry = match geometry_source {
            GeometrySource::Wkt(w) => match wkt::read(&row[w]) {
                Ok(g) => g,
                Err(e) => {
                    report.skipped.push(SkippedRecord {
                        record,
                        reason: e.to_string(),
                    });
                    continue;
                }
            },
            GeometrySource::Xy(x, y) => match (parse_number(&row[x]), parse_number(&row[y])) {
                (Some(x), Some(y)) => Geometry::Point(Point::new(x, y)),
                _ => {
                    report.skipped.push(SkippedRecord {
                        record,
                        reason: format!("coordinates ({}, {}) are not numbers", row[x], row[y]),
                    });
                    continue;
                }
            },
        };
        let mut properties = BTreeMap::new();
        for (col, cell) in row.iter().enumerate() {
            if matches!(geometry_source, GeometrySource::Wkt(w) if w == col) {
                continue;
            }
            if declared_crs.is_some() && crs_column == Some(col) {
                continue;
            }
            properties.insert(header[col].clone(), typed_value(cell, column_types[col]));
        }
        layer.features.push(Feature {
            id: layer.features.len() as u64,
            geometry: Some(geometry),
            properties,
        });
    }

    if let Some(user) = &options.crs {
        layer.crs = proj::lookup(user)?.id;
        layer.crs_status = CrsStatus::UserSupplied;
    } else if let Some(declared) = &declared_crs {
        layer.crs = proj::lookup(declared)?.id;
        layer.crs_status = CrsStatus::Declared;
        report
            .notes
            .push(format!("CRS {} read from the crs column", layer.crs));
    } else if layer.features.is_empty() {
        // Reported by the caller.
    } else if coordinates_look_geographic(&layer) {
        layer.crs_status = CrsStatus::Inferred;
        report.notes.push(
            "CSV has no CRS; EPSG:4326 was inferred from the longitude/latitude value ranges — confirm or re-import with an explicit CRS"
                .into(),
        );
    } else {
        return Err(ToolkitError::CrsRequired(
            "CSV coordinates are outside the longitude/latitude range; choose the source CRS \
             (for example EPSG:6675 for 平面直角座標系 VII 系)"
                .into(),
        ));
    }
    layer.refresh_schema();
    Ok(layer)
}

#[derive(Clone, Copy)]
enum GeometrySource {
    Wkt(usize),
    Xy(usize, usize),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ColumnType {
    Integer,
    Float,
    Boolean,
    Text,
}

fn column_type<'a>(values: impl Iterator<Item = &'a str>) -> ColumnType {
    let mut kind: Option<ColumnType> = None;
    for raw in values {
        let value = raw.trim();
        if value.is_empty() {
            continue;
        }
        let this = if has_significant_leading_zero(value) {
            // Codes such as 市区町村コード "01101" must stay text.
            ColumnType::Text
        } else if value.parse::<i64>().is_ok() {
            ColumnType::Integer
        } else if parse_number(value).is_some() {
            ColumnType::Float
        } else if matches!(value.to_ascii_lowercase().as_str(), "true" | "false") {
            ColumnType::Boolean
        } else {
            ColumnType::Text
        };
        kind = Some(match (kind, this) {
            (None, t) => t,
            (Some(a), b) if a == b => a,
            (Some(ColumnType::Integer), ColumnType::Float)
            | (Some(ColumnType::Float), ColumnType::Integer) => ColumnType::Float,
            _ => ColumnType::Text,
        });
        if kind == Some(ColumnType::Text) {
            break;
        }
    }
    kind.unwrap_or(ColumnType::Text)
}

fn has_significant_leading_zero(value: &str) -> bool {
    let digits = value.strip_prefix('-').unwrap_or(value);
    digits.len() > 1 && digits.starts_with('0') && !digits.starts_with("0.")
}

fn parse_number(value: &str) -> Option<f64> {
    let v = value.trim();
    if v.is_empty() || v.eq_ignore_ascii_case("nan") || v.to_ascii_lowercase().contains("inf") {
        return None;
    }
    v.parse::<f64>().ok().filter(|n| n.is_finite())
}

fn typed_value(cell: &str, column: ColumnType) -> Value {
    let v = cell.trim();
    if v.is_empty() {
        return Value::Null;
    }
    match column {
        ColumnType::Integer => v
            .parse::<i64>()
            .map(Value::from)
            .unwrap_or_else(|_| Value::from(cell)),
        ColumnType::Float => parse_number(v)
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or_else(|| Value::from(cell)),
        ColumnType::Boolean => Value::Bool(v.eq_ignore_ascii_case("true")),
        ColumnType::Text => Value::from(cell),
    }
}

fn detect_delimiter(text: &str) -> char {
    let first = text.lines().next().unwrap_or("");
    let candidates = [',', '\t', ';'];
    candidates
        .into_iter()
        .max_by_key(|c| first.matches(*c).count())
        .filter(|c| first.contains(*c))
        .unwrap_or(',')
}

/// RFC 4180 parser supporting quoted fields, escaped quotes, and embedded newlines.
fn parse_rows(text: &str, delimiter: char) -> Result<Vec<Vec<String>>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();
    let mut any = false;
    while let Some(c) = chars.next() {
        any = true;
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => in_quotes = true,
            c if c == delimiter => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            c => field.push(c),
        }
    }
    if in_quotes {
        return Err(ToolkitError::import("csv", "unterminated quoted field"));
    }
    if any && (!field.is_empty() || !row.is_empty()) {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::super::{import_bytes, ImportOptions};
    use crate::layer::{CrsStatus, FieldType};
    use crate::ToolkitError;

    #[test]
    fn imports_lon_lat_columns_and_keeps_codes_as_text() {
        let csv = "name,経度,緯度,code,pop\n名古屋駅,136.8815,35.1709,23100,10\n栄,136.9086,35.1681,01101,12.5\n";
        let (layer, report) =
            import_bytes("sites.csv", csv.as_bytes(), &ImportOptions::default()).unwrap();
        assert_eq!(layer.features.len(), 2);
        assert_eq!(layer.crs_status, CrsStatus::Inferred);
        assert!(layer.crs_status.needs_confirmation());
        assert_eq!(layer.field("code").unwrap().field_type, FieldType::Text);
        assert_eq!(layer.field("pop").unwrap().field_type, FieldType::Float);
        assert_eq!(layer.features[1].properties["code"], "01101");
        assert!(report.notes.iter().any(|n| n.contains("inferred")));
    }

    #[test]
    fn reports_bad_rows_instead_of_dropping_silently() {
        let csv = "lon,lat,v\n136.9,35.1,1\n,35.2,2\n\"137.0\",\"35.3\",\"a, b\"\n";
        let (layer, report) =
            import_bytes("x.csv", csv.as_bytes(), &ImportOptions::default()).unwrap();
        assert_eq!(layer.features.len(), 2);
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].record, 3);
        assert_eq!(layer.features[1].properties["v"], "a, b");
    }

    #[test]
    fn projected_csv_requires_crs() {
        let csv = "x,y\n-26000,-92000\n";
        let error = import_bytes("p.csv", csv.as_bytes(), &ImportOptions::default()).unwrap_err();
        assert!(matches!(error, ToolkitError::CrsRequired(_)));
        let options = ImportOptions {
            crs: Some("EPSG:6675".into()),
            ..Default::default()
        };
        let (layer, _) = import_bytes("p.csv", csv.as_bytes(), &options).unwrap();
        assert_eq!(layer.crs, "EPSG:6675");
    }

    #[test]
    fn reads_wkt_column_and_tab_delimiter() {
        let tsv = "id\twkt\n1\tPOLYGON ((0 0, 1 0, 1 1, 0 0))\n";
        let (layer, _) = import_bytes("p.tsv", tsv.as_bytes(), &ImportOptions::default()).unwrap();
        assert_eq!(layer.features.len(), 1);
        assert!(layer.field("wkt").is_none());
    }

    #[test]
    fn reads_shift_jis_csv() {
        let (bytes, _, _) = encoding_rs::SHIFT_JIS.encode("名称,lon,lat\n中区,136.9,35.16\n");
        let (layer, report) = import_bytes("sjis.csv", &bytes, &ImportOptions::default()).unwrap();
        assert_eq!(report.encoding.as_deref(), Some("shift_jis"));
        assert_eq!(layer.features[0].properties["名称"], "中区");
    }
}
