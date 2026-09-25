//! OGC GeoPackage (1.4) feature-table reading and writing.

use std::collections::BTreeMap;
use std::path::PathBuf;

use geo::BoundingRect;
use geo_types::Geometry;
use rusqlite::{types::ValueRef, Connection};
use serde_json::Value;

use crate::error::{Result, ToolkitError};
use crate::import::{ImportOptions, ImportReport, SkippedRecord};
use crate::layer::{CrsStatus, Feature, FieldType, Layer};
use crate::{proj, wkb};

fn rerr(reason: impl std::fmt::Display) -> ToolkitError {
    ToolkitError::import("geopackage", reason.to_string())
}

fn werr(reason: impl std::fmt::Display) -> ToolkitError {
    ToolkitError::export("geopackage", reason.to_string())
}

/// Temporary file removed on drop.
struct TempFile(PathBuf);

impl TempFile {
    fn new(ext: &str) -> Self {
        Self(std::env::temp_dir().join(format!("genegis-{}.{ext}", uuid::Uuid::new_v4())))
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// (table, geometry column, srs_id, organization, organization_coordsys_id)
type GeometryColumn = (String, String, i64, Option<String>, Option<i64>);

pub(crate) fn read(
    bytes: &[u8],
    options: &ImportOptions,
    report: &mut ImportReport,
) -> Result<Layer> {
    let temp = TempFile::new("gpkg");
    std::fs::write(&temp.0, bytes)?;
    let conn = Connection::open_with_flags(&temp.0, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(rerr)?;
    let mut tables: Vec<GeometryColumn> = conn
        .prepare(
            "SELECT g.table_name, g.column_name, g.srs_id, s.organization, s.organization_coordsys_id
             FROM gpkg_geometry_columns g
             LEFT JOIN gpkg_spatial_ref_sys s ON s.srs_id = g.srs_id
             ORDER BY g.table_name",
        )
        .and_then(|mut stmt| {
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)))?
                .collect()
        })
        .map_err(|e| rerr(format!("not a GeoPackage feature container: {e}")))?;
    if tables.is_empty() {
        return Err(rerr("the GeoPackage has no feature tables"));
    }
    let chosen = match &options.table {
        Some(name) => tables
            .iter()
            .position(|t| &t.0 == name)
            .ok_or_else(|| rerr(format!("no feature table named {name}")))?,
        None => {
            if tables.len() > 1 {
                report.notes.push(format!(
                    "GeoPackage has {} feature tables ({}); imported {} (choose another with table)",
                    tables.len(),
                    tables.iter().map(|t| t.0.as_str()).collect::<Vec<_>>().join(", "),
                    tables[0].0
                ));
            }
            0
        }
    };
    let (table, geom_column, srs_id, organization, org_id) = tables.swap_remove(chosen);

    let mut layer = Layer::new(
        options.name.clone().unwrap_or_else(|| table.clone()),
        "EPSG:4326",
        CrsStatus::Declared,
    );
    if let Some(user) = &options.crs {
        layer.crs = proj::lookup(user)?.id;
        layer.crs_status = CrsStatus::UserSupplied;
    } else {
        let epsg = match (
            organization
                .as_deref()
                .map(str::to_ascii_uppercase)
                .as_deref(),
            org_id,
        ) {
            (Some("EPSG"), Some(code)) => code as u32,
            _ if srs_id == 4326 => 4326,
            _ => {
                return Err(ToolkitError::CrsRequired(format!(
                    "GeoPackage SRS {srs_id} is not an EPSG definition; choose the CRS explicitly"
                )))
            }
        };
        layer.crs = proj::lookup_epsg(epsg)?.id;
        report
            .notes
            .push(format!("CRS {} read from gpkg_spatial_ref_sys", layer.crs));
    }

    let quoted = format!("\"{}\"", table.replace('"', "\"\""));
    let mut stmt = conn
        .prepare(&format!("SELECT * FROM {quoted}"))
        .map_err(rerr)?;
    let names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let pk = conn
        .prepare(&format!("PRAGMA table_info({quoted})"))
        .and_then(|mut s| {
            s.query_map([], |row| {
                Ok((row.get::<_, String>(1)?, row.get::<_, i64>(5)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
        })
        .map_err(rerr)?
        .into_iter()
        .find(|(_, pk)| *pk > 0)
        .map(|(name, _)| name);
    let mut rows = stmt.query([]).map_err(rerr)?;
    let mut record = 0;
    while let Some(row) = rows.next().map_err(rerr)? {
        record += 1;
        let mut properties = BTreeMap::new();
        let mut geometry = None;
        let mut failed = None;
        for (i, name) in names.iter().enumerate() {
            let value = row.get_ref(i).map_err(rerr)?;
            if name == &geom_column {
                if let ValueRef::Blob(blob) = value {
                    match decode_blob(blob) {
                        Ok(g) => geometry = g,
                        Err(e) => failed = Some(e.to_string()),
                    }
                }
                continue;
            }
            if Some(name) == pk.as_ref() {
                continue;
            }
            properties.insert(
                name.clone(),
                match value {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(i) => Value::from(i),
                    ValueRef::Real(f) => serde_json::Number::from_f64(f)
                        .map(Value::Number)
                        .unwrap_or(Value::Null),
                    ValueRef::Text(t) => Value::from(String::from_utf8_lossy(t).into_owned()),
                    ValueRef::Blob(_) => Value::Null,
                },
            );
        }
        if let Some(reason) = failed {
            report.skipped.push(SkippedRecord { record, reason });
            continue;
        }
        layer.features.push(Feature {
            id: layer.features.len() as u64,
            geometry,
            properties,
        });
    }
    layer.refresh_schema();
    Ok(layer)
}

/// Decode a GeoPackage binary geometry blob.
fn decode_blob(blob: &[u8]) -> Result<Option<Geometry<f64>>> {
    if blob.len() < 8 || &blob[0..2] != b"GP" {
        return Err(rerr("geometry blob lacks the GP header"));
    }
    let flags = blob[3];
    if flags & 0b0010_0000 != 0 {
        return Err(rerr("extended GeoPackage geometries are not supported"));
    }
    let empty = flags & 0b0001_0000 != 0;
    let envelope = match (flags >> 1) & 0b111 {
        0 => 0,
        1 => 32,
        2 | 3 => 48,
        4 => 64,
        other => return Err(rerr(format!("invalid envelope indicator {other}"))),
    };
    let start = 8 + envelope;
    if empty {
        return Ok(None);
    }
    let wkb_bytes = blob
        .get(start..)
        .ok_or_else(|| rerr("truncated geometry blob"))?;
    wkb::read(wkb_bytes).map(Some)
}

fn encode_blob(geometry: &Geometry<f64>, srs_id: i32) -> Vec<u8> {
    let mut out = vec![b'G', b'P', 0];
    let rect = geometry.bounding_rect();
    out.push(if rect.is_some() {
        0b0000_0011
    } else {
        0b0001_0001
    });
    out.extend_from_slice(&srs_id.to_le_bytes());
    if let Some(rect) = rect {
        for v in [rect.min().x, rect.max().x, rect.min().y, rect.max().y] {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out.extend_from_slice(&wkb::write(geometry));
    out
}

/// Write a single feature table GeoPackage and return its bytes.
pub fn write(layer: &Layer) -> Result<Vec<u8>> {
    let crs = layer.crs_info()?;
    let temp = TempFile::new("gpkg");
    {
        let conn = Connection::open(&temp.0).map_err(werr)?;
        let table = sanitize_identifier(&layer.name);
        let srs_id = crs.epsg as i32;
        let geometry_type = "GEOMETRY";
        conn.execute_batch(
            "PRAGMA application_id = 1196444487;
             PRAGMA user_version = 10400;
             CREATE TABLE gpkg_spatial_ref_sys (srs_name TEXT NOT NULL, srs_id INTEGER PRIMARY KEY, organization TEXT NOT NULL, organization_coordsys_id INTEGER NOT NULL, definition TEXT NOT NULL, description TEXT);
             CREATE TABLE gpkg_contents (table_name TEXT NOT NULL PRIMARY KEY, data_type TEXT NOT NULL, identifier TEXT UNIQUE, description TEXT DEFAULT '', last_change DATETIME NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')), min_x DOUBLE, min_y DOUBLE, max_x DOUBLE, max_y DOUBLE, srs_id INTEGER, CONSTRAINT fk_gc_r_srs_id FOREIGN KEY (srs_id) REFERENCES gpkg_spatial_ref_sys(srs_id));
             CREATE TABLE gpkg_geometry_columns (table_name TEXT NOT NULL, column_name TEXT NOT NULL, geometry_type_name TEXT NOT NULL, srs_id INTEGER NOT NULL, z TINYINT NOT NULL, m TINYINT NOT NULL, CONSTRAINT pk_geom_cols PRIMARY KEY (table_name, column_name));
             INSERT INTO gpkg_spatial_ref_sys VALUES ('Undefined cartesian SRS', -1, 'NONE', -1, 'undefined', NULL);
             INSERT INTO gpkg_spatial_ref_sys VALUES ('Undefined geographic SRS', 0, 'NONE', 0, 'undefined', NULL);",
        )
        .map_err(werr)?;
        if srs_id != 4326 {
            conn.execute(
                "INSERT INTO gpkg_spatial_ref_sys VALUES ('WGS 84', 4326, 'EPSG', 4326, ?1, NULL)",
                [proj::to_wkt(&proj::lookup_epsg(4326)?)],
            )
            .map_err(werr)?;
        }
        conn.execute(
            "INSERT INTO gpkg_spatial_ref_sys VALUES (?1, ?2, 'EPSG', ?2, ?3, NULL)",
            rusqlite::params![crs.name, srs_id, proj::to_wkt(&crs)],
        )
        .map_err(werr)?;

        let mut columns = vec![
            "fid INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL".to_string(),
            "geom GEOMETRY".to_string(),
        ];
        let mut field_names = Vec::new();
        for field in &layer.fields {
            let name = sanitize_identifier(&field.name);
            if name.eq_ignore_ascii_case("fid") || name.eq_ignore_ascii_case("geom") {
                continue;
            }
            let sql_type = match field.field_type {
                FieldType::Integer => "INTEGER",
                FieldType::Float => "REAL",
                FieldType::Boolean => "BOOLEAN",
                FieldType::Text | FieldType::Json => "TEXT",
            };
            columns.push(format!("\"{name}\" {sql_type}"));
            field_names.push((field.name.clone(), name, field.field_type));
        }
        conn.execute(
            &format!("CREATE TABLE \"{table}\" ({})", columns.join(", ")),
            [],
        )
        .map_err(werr)?;
        let bbox = layer.bbox().unwrap_or([0.0; 4]);
        let description = format!(
            "GeneGIS export; source {}; digest {}{}",
            layer.provenance.source_uri,
            layer.digest(),
            layer
                .provenance
                .attribution
                .as_ref()
                .map(|a| format!("; attribution: {a}"))
                .unwrap_or_default()
        );
        conn.execute(
            "INSERT INTO gpkg_contents (table_name, data_type, identifier, description, min_x, min_y, max_x, max_y, srs_id) VALUES (?1, 'features', ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![table, description, bbox[0], bbox[1], bbox[2], bbox[3], srs_id],
        )
        .map_err(werr)?;
        conn.execute(
            "INSERT INTO gpkg_geometry_columns VALUES (?1, 'geom', ?2, ?3, 0, 0)",
            rusqlite::params![table, geometry_type, srs_id],
        )
        .map_err(werr)?;

        let placeholders: Vec<String> = (0..field_names.len() + 1)
            .map(|i| format!("?{}", i + 1))
            .collect();
        let column_list: Vec<String> = std::iter::once("geom".to_string())
            .chain(field_names.iter().map(|(_, n, _)| format!("\"{n}\"")))
            .collect();
        let sql = format!(
            "INSERT INTO \"{table}\" ({}) VALUES ({})",
            column_list.join(", "),
            placeholders.join(", ")
        );
        conn.execute_batch("BEGIN").map_err(werr)?;
        {
            let mut stmt = conn.prepare(&sql).map_err(werr)?;
            for feature in &layer.features {
                let mut values: Vec<rusqlite::types::Value> =
                    Vec::with_capacity(field_names.len() + 1);
                values.push(match &feature.geometry {
                    Some(g) => rusqlite::types::Value::Blob(encode_blob(g, srs_id)),
                    None => rusqlite::types::Value::Null,
                });
                for (original, _, field_type) in &field_names {
                    values.push(to_sql_value(feature.properties.get(original), *field_type));
                }
                stmt.execute(rusqlite::params_from_iter(values))
                    .map_err(werr)?;
            }
        }
        conn.execute_batch("COMMIT").map_err(werr)?;
    }
    Ok(std::fs::read(&temp.0)?)
}

fn to_sql_value(value: Option<&Value>, field_type: FieldType) -> rusqlite::types::Value {
    use rusqlite::types::Value as Sql;
    match value {
        None | Some(Value::Null) => Sql::Null,
        Some(Value::Bool(b)) => Sql::Integer(i64::from(*b)),
        Some(Value::Number(n)) => match (field_type, n.as_i64()) {
            (FieldType::Integer, Some(i)) => Sql::Integer(i),
            _ => n.as_f64().map(Sql::Real).unwrap_or(Sql::Null),
        },
        Some(Value::String(s)) => Sql::Text(s.clone()),
        Some(other) => Sql::Text(other.to_string()),
    }
}

fn sanitize_identifier(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !c.is_control() && *c != '"')
        .collect();
    if cleaned.trim().is_empty() {
        "layer".into()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::import_bytes;
    use geo_types::point;

    #[test]
    fn round_trips_through_geopackage() {
        let mut layer = Layer::new("地点", "EPSG:6675", CrsStatus::Declared);
        layer.features.push(Feature {
            id: 0,
            geometry: Some(Geometry::Point(point!(x: -26000.0, y: -92000.0))),
            properties: BTreeMap::from([
                ("名前".to_string(), Value::from("名古屋駅")),
                ("n".to_string(), Value::from(3)),
                ("v".to_string(), Value::from(1.5)),
            ]),
        });
        layer.refresh_schema();
        let bytes = write(&layer).unwrap();
        let (back, report) = import_bytes("out.gpkg", &bytes, &ImportOptions::default()).unwrap();
        assert_eq!(back.crs, "EPSG:6675");
        assert_eq!(back.features.len(), 1);
        assert_eq!(back.features[0].geometry, layer.features[0].geometry);
        assert_eq!(back.features[0].properties["名前"], "名古屋駅");
        assert_eq!(back.features[0].properties["n"], 3);
        assert!(report
            .notes
            .iter()
            .any(|n| n.contains("gpkg_spatial_ref_sys")));
    }
}
