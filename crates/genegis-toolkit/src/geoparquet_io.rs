//! GeoParquet 1.1 (WKB encoding) reading and writing.

use std::collections::BTreeMap;
use std::sync::Arc;

use arrow_array::{
    Array, ArrayRef, BinaryArray, BooleanArray, Float32Array, Float64Array, Int16Array, Int32Array,
    Int64Array, Int8Array, LargeBinaryArray, LargeStringArray, RecordBatch, StringArray,
    UInt16Array, UInt32Array, UInt64Array, UInt8Array,
};
use arrow_schema::{DataType, Field as ArrowField, Schema};
use bytes::Bytes;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::ArrowWriter;
use parquet::file::metadata::KeyValue;
use parquet::file::properties::WriterProperties;
use serde_json::Value;

use crate::error::{Result, ToolkitError};
use crate::import::{ImportOptions, ImportReport, SkippedRecord};
use crate::layer::{CrsStatus, Feature, FieldType, Layer};
use crate::{proj, wkb};

fn rerr(reason: impl std::fmt::Display) -> ToolkitError {
    ToolkitError::import("geoparquet", reason.to_string())
}

fn werr(reason: impl std::fmt::Display) -> ToolkitError {
    ToolkitError::export("geoparquet", reason.to_string())
}

pub(crate) fn read(
    bytes: &[u8],
    options: &ImportOptions,
    report: &mut ImportReport,
) -> Result<Layer> {
    let builder =
        ParquetRecordBatchReaderBuilder::try_new(Bytes::copy_from_slice(bytes)).map_err(rerr)?;
    let geo: Value = builder
        .metadata()
        .file_metadata()
        .key_value_metadata()
        .and_then(|kv| kv.iter().find(|entry| entry.key == "geo"))
        .and_then(|entry| entry.value.as_deref())
        .ok_or_else(|| rerr("parquet file has no GeoParquet `geo` metadata"))
        .and_then(|text| serde_json::from_str(text).map_err(rerr))?;
    let primary = geo
        .get("primary_column")
        .and_then(Value::as_str)
        .ok_or_else(|| rerr("geo metadata has no primary_column"))?
        .to_string();
    let column_meta = geo
        .pointer(&format!(
            "/columns/{}",
            primary.replace('~', "~0").replace('/', "~1")
        ))
        .ok_or_else(|| rerr("geo metadata does not describe the primary column"))?;
    let encoding = column_meta
        .get("encoding")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !encoding.eq_ignore_ascii_case("WKB") {
        return Err(rerr(format!(
            "geometry encoding {encoding} is not supported; re-export with WKB encoding"
        )));
    }

    let mut layer = Layer::new(
        options.name.clone().unwrap_or_default(),
        "EPSG:4326",
        CrsStatus::FormatDefault,
    );
    if let Some(user) = &options.crs {
        layer.crs = proj::lookup(user)?.id;
        layer.crs_status = CrsStatus::UserSupplied;
    } else {
        match column_meta.get("crs") {
            None => report
                .notes
                .push("GeoParquet column has no crs; OGC:CRS84 (EPSG:4326 axis order lon/lat) per specification".into()),
            Some(Value::Null) => {
                return Err(ToolkitError::CrsRequired(
                    "GeoParquet declares an undefined CRS (crs: null); choose the CRS explicitly".into(),
                ))
            }
            Some(projjson) => {
                let epsg = projjson_epsg(projjson).ok_or_else(|| {
                    ToolkitError::CrsRequired("GeoParquet PROJJSON has no EPSG id; choose the CRS explicitly".into())
                })?;
                layer.crs = proj::lookup_epsg(epsg)?.id;
                layer.crs_status = CrsStatus::Declared;
                report.notes.push(format!("CRS {} read from GeoParquet PROJJSON", layer.crs));
            }
        }
    }

    let reader = builder.build().map_err(rerr)?;
    let mut record = 0usize;
    let mut skipped_columns = BTreeMap::new();
    for batch in reader {
        let batch = batch.map_err(rerr)?;
        let schema = batch.schema();
        let geometry_index = schema
            .index_of(&primary)
            .map_err(|_| rerr(format!("primary column {primary} is missing")))?;
        for row in 0..batch.num_rows() {
            record += 1;
            let geometry_column = batch.column(geometry_index);
            let geometry = if geometry_column.is_null(row) {
                None
            } else {
                let raw = binary_value(geometry_column, row)
                    .ok_or_else(|| rerr("geometry column is not binary"))?;
                match wkb::read(raw) {
                    Ok(g) => Some(g),
                    Err(e) => {
                        report.skipped.push(SkippedRecord {
                            record,
                            reason: e.to_string(),
                        });
                        continue;
                    }
                }
            };
            let mut properties = BTreeMap::new();
            for (index, field) in schema.fields().iter().enumerate() {
                if index == geometry_index {
                    continue;
                }
                match json_value(batch.column(index), row) {
                    Some(value) => {
                        properties.insert(field.name().clone(), value);
                    }
                    None => {
                        skipped_columns.insert(field.name().clone(), field.data_type().to_string());
                    }
                }
            }
            layer.features.push(Feature {
                id: layer.features.len() as u64,
                geometry,
                properties,
            });
        }
    }
    for (name, data_type) in skipped_columns {
        report.notes.push(format!(
            "column {name} ({data_type}) is not supported and was skipped"
        ));
    }
    layer.refresh_schema();
    Ok(layer)
}

fn projjson_epsg(value: &Value) -> Option<u32> {
    let id = value.get("id")?;
    let authority = id.get("authority")?.as_str()?;
    if !authority.eq_ignore_ascii_case("EPSG") {
        return if authority.eq_ignore_ascii_case("OGC") && id.get("code")?.as_str() == Some("CRS84")
        {
            Some(4326)
        } else {
            None
        };
    }
    match id.get("code")? {
        Value::Number(n) => n.as_u64().map(|n| n as u32),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn binary_value(array: &ArrayRef, row: usize) -> Option<&[u8]> {
    if let Some(a) = array.as_any().downcast_ref::<BinaryArray>() {
        return Some(a.value(row));
    }
    array
        .as_any()
        .downcast_ref::<LargeBinaryArray>()
        .map(|a| a.value(row))
}

fn json_value(array: &ArrayRef, row: usize) -> Option<Value> {
    if array.is_null(row) {
        return Some(Value::Null);
    }
    macro_rules! int {
        ($t:ty) => {
            if let Some(a) = array.as_any().downcast_ref::<$t>() {
                return Some(Value::from(a.value(row) as i64));
            }
        };
    }
    int!(Int8Array);
    int!(Int16Array);
    int!(Int32Array);
    int!(Int64Array);
    int!(UInt8Array);
    int!(UInt16Array);
    int!(UInt32Array);
    if let Some(a) = array.as_any().downcast_ref::<UInt64Array>() {
        return Some(Value::from(a.value(row)));
    }
    if let Some(a) = array.as_any().downcast_ref::<Float32Array>() {
        return Some(
            serde_json::Number::from_f64(a.value(row) as f64)
                .map(Value::Number)
                .unwrap_or(Value::Null),
        );
    }
    if let Some(a) = array.as_any().downcast_ref::<Float64Array>() {
        return Some(
            serde_json::Number::from_f64(a.value(row))
                .map(Value::Number)
                .unwrap_or(Value::Null),
        );
    }
    if let Some(a) = array.as_any().downcast_ref::<BooleanArray>() {
        return Some(Value::Bool(a.value(row)));
    }
    if let Some(a) = array.as_any().downcast_ref::<StringArray>() {
        return Some(Value::from(a.value(row)));
    }
    if let Some(a) = array.as_any().downcast_ref::<LargeStringArray>() {
        return Some(Value::from(a.value(row)));
    }
    None
}

/// Write a layer as GeoParquet 1.1 with WKB geometry.
pub fn write(layer: &Layer) -> Result<Vec<u8>> {
    let crs = layer.crs_info()?;
    let mut fields = Vec::new();
    let mut columns: Vec<ArrayRef> = Vec::new();
    for field in &layer.fields {
        let values = layer.features.iter().map(|f| f.properties.get(&field.name));
        let (data_type, array): (DataType, ArrayRef) = match field.field_type {
            FieldType::Integer => (
                DataType::Int64,
                Arc::new(
                    values
                        .map(|v| v.and_then(Value::as_i64))
                        .collect::<Int64Array>(),
                ),
            ),
            FieldType::Float => (
                DataType::Float64,
                Arc::new(
                    values
                        .map(|v| v.and_then(Value::as_f64))
                        .collect::<Float64Array>(),
                ),
            ),
            FieldType::Boolean => (
                DataType::Boolean,
                Arc::new(
                    values
                        .map(|v| v.and_then(Value::as_bool))
                        .collect::<BooleanArray>(),
                ),
            ),
            FieldType::Text | FieldType::Json => (
                DataType::Utf8,
                Arc::new(
                    values
                        .map(|v| match v {
                            None | Some(Value::Null) => None,
                            Some(Value::String(s)) => Some(s.clone()),
                            Some(other) => Some(other.to_string()),
                        })
                        .collect::<StringArray>(),
                ),
            ),
        };
        let mut metadata = std::collections::HashMap::new();
        if let Some(unit) = &field.unit {
            metadata.insert("unit".to_string(), unit.clone());
        }
        fields.push(ArrowField::new(field.name.clone(), data_type, true).with_metadata(metadata));
        columns.push(array);
    }
    let geometry_name = if layer.field("geometry").is_some() {
        "__geometry"
    } else {
        "geometry"
    };
    fields.push(ArrowField::new(geometry_name, DataType::Binary, true));
    let wkbs: Vec<Option<Vec<u8>>> = layer
        .features
        .iter()
        .map(|f| f.geometry.as_ref().map(wkb::write))
        .collect();
    columns.push(Arc::new(BinaryArray::from_iter(
        wkbs.iter().map(|w| w.as_deref()),
    )));
    let schema = Arc::new(Schema::new(fields));
    let batch = RecordBatch::try_new(schema.clone(), columns).map_err(werr)?;

    let mut kinds: Vec<&str> = layer
        .features
        .iter()
        .filter_map(|f| f.geometry.as_ref())
        .map(|g| match g {
            geo_types::Geometry::Point(_) => "Point",
            geo_types::Geometry::MultiPoint(_) => "MultiPoint",
            geo_types::Geometry::LineString(_) | geo_types::Geometry::Line(_) => "LineString",
            geo_types::Geometry::MultiLineString(_) => "MultiLineString",
            geo_types::Geometry::Polygon(_)
            | geo_types::Geometry::Rect(_)
            | geo_types::Geometry::Triangle(_) => "Polygon",
            geo_types::Geometry::MultiPolygon(_) => "MultiPolygon",
            geo_types::Geometry::GeometryCollection(_) => "GeometryCollection",
        })
        .collect();
    kinds.sort_unstable();
    kinds.dedup();
    let mut column = serde_json::json!({
        "encoding": "WKB",
        "geometry_types": kinds,
    });
    if let Some(bbox) = layer.bbox() {
        column["bbox"] = serde_json::json!(bbox);
    }
    if crs.epsg != 4326 {
        column["crs"] = serde_json::json!({
            "$schema": "https://proj.org/schemas/v0.7/projjson.schema.json",
            "type": if crs.is_geographic() { "GeographicCRS" } else { "ProjectedCRS" },
            "name": crs.name,
            "id": {"authority": "EPSG", "code": crs.epsg}
        });
    }
    let geo = serde_json::json!({
        "version": "1.1.0",
        "primary_column": geometry_name,
        "columns": { geometry_name: column },
    });
    let provenance = serde_json::json!({
        "source_uri": layer.provenance.source_uri,
        "source_sha256": layer.provenance.source_sha256,
        "license": layer.provenance.license,
        "attribution": layer.provenance.attribution,
        "layer_digest": layer.digest(),
        "parents": layer.provenance.parents,
        "operation": layer.provenance.operation,
        "workflow_digest": layer.provenance.workflow_digest,
    });
    let props = WriterProperties::builder()
        .set_key_value_metadata(Some(vec![
            KeyValue::new("geo".to_string(), geo.to_string()),
            KeyValue::new("genegis:provenance".to_string(), provenance.to_string()),
        ]))
        .build();
    let mut out = Vec::new();
    {
        let mut writer = ArrowWriter::try_new(&mut out, schema, Some(props)).map_err(werr)?;
        writer.write(&batch).map_err(werr)?;
        writer.close().map_err(werr)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::import_bytes;
    use geo_types::{point, Geometry};

    #[test]
    fn round_trips_projected_layer() {
        let mut layer = Layer::new("sites", "EPSG:6675", CrsStatus::Declared);
        for i in 0..3 {
            layer.features.push(Feature {
                id: i,
                geometry: Some(Geometry::Point(point!(x: -26000.0 + i as f64, y: -92000.0))),
                properties: BTreeMap::from([
                    ("name".to_string(), Value::from(format!("s{i}"))),
                    ("pop".to_string(), Value::from(i as i64 * 10)),
                ]),
            });
        }
        layer.refresh_schema();
        layer.set_field_unit("pop", "persons");
        let bytes = write(&layer).unwrap();
        let (back, _) = import_bytes("sites.parquet", &bytes, &ImportOptions::default()).unwrap();
        assert_eq!(back.crs, "EPSG:6675");
        assert_eq!(back.features.len(), 3);
        assert_eq!(back.features[2].properties["pop"], 20);
        assert_eq!(back.features[1].geometry, layer.features[1].geometry);
    }
}
