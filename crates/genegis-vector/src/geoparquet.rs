use std::fs::File;
use std::path::Path;

use arrow_array::{Array, BinaryArray, RecordBatch};
use arrow_schema::DataType;
use bytes::Bytes;
use genegis_geometry::BoundingBox;
use geoparquet::metadata::GeoParquetMetadata;
use geoparquet::reader::{GeoParquetReaderBuilder, GeoParquetRecordBatchReader};
use geo_traits::to_geo::ToGeoGeometry;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::file::reader::ChunkReader;
use serde_json::{Map, Value};
use wkb::reader::read_wkb;

use crate::dataset::{FeatureRecord, VectorDataset};
use crate::error::VectorError;
use crate::geometry::geo_geometry_to_rings;

/// Read a GeoParquet file from disk into the shared [`VectorDataset`] model.
pub fn read_geoparquet_path(path: &str) -> Result<VectorDataset, VectorError> {
    let file = File::open(path)?;
    let name = Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unnamed")
        .to_string();
    read_geoparquet_chunk_reader(file, Some(name))
}

/// Read GeoParquet from a local path or HTTP(S) URL.
pub fn read_geoparquet_uri(uri: &str) -> Result<VectorDataset, VectorError> {
    if genegis_storage::is_remote_uri(uri) {
        let bytes = genegis_storage::read_asset_bytes(uri)
            .map_err(|err| VectorError::GeoParquet(err.to_string()))?;
        read_geoparquet_bytes(&bytes)
    } else {
        read_geoparquet_path(uri)
    }
}

/// Read GeoParquet bytes (e.g. cloud object download) into [`VectorDataset`].
pub fn read_geoparquet_bytes(bytes: &[u8]) -> Result<VectorDataset, VectorError> {
    read_geoparquet_chunk_reader(Bytes::copy_from_slice(bytes), None)
}

fn read_geoparquet_chunk_reader<R: ChunkReader + 'static>(
    reader: R,
    name: Option<String>,
) -> Result<VectorDataset, VectorError> {
    let builder = ParquetRecordBatchReaderBuilder::try_new(reader)
        .map_err(|err| VectorError::GeoParquet(err.to_string()))?;

    let geo_metadata = match builder.geoparquet_metadata() {
        Some(Ok(metadata)) => metadata,
        Some(Err(err)) => return Err(VectorError::GeoParquet(err.to_string())),
        None => {
            return Err(VectorError::GeoParquet(
                "missing GeoParquet metadata".into(),
            ))
        }
    };

    let geoarrow_schema = builder
        .geoarrow_schema(&geo_metadata, false, Default::default())
        .map_err(|err| VectorError::GeoParquet(err.to_string()))?;

    let parquet_reader = builder
        .with_batch_size(1024)
        .build()
        .map_err(|err| VectorError::GeoParquet(err.to_string()))?;

    let mut reader = GeoParquetRecordBatchReader::try_new(parquet_reader, geoarrow_schema)
        .map_err(|err| VectorError::GeoParquet(err.to_string()))?;

    let geometry_column = primary_geometry_column(&geo_metadata)?;
    let crs = crs_from_metadata(&geo_metadata, geometry_column);
    let dataset_name = name.unwrap_or_else(|| "geoparquet".to_string());

    let mut features = Vec::new();
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    let mut next_id = 0usize;

    while let Some(batch) = reader
        .next()
        .transpose()
        .map_err(|err| VectorError::GeoParquet(err.to_string()))?
    {
        parse_batch(
            &batch,
            geometry_column,
            &mut features,
            &mut next_id,
            &mut min_x,
            &mut min_y,
            &mut max_x,
            &mut max_y,
        )?;
    }

    let bbox = if features.is_empty() {
        BoundingBox::new(0.0, 0.0, 0.0, 0.0)
    } else {
        BoundingBox::new(min_x, min_y, max_x, max_y)
    };

    Ok(VectorDataset {
        name: dataset_name,
        crs,
        features,
        bbox,
    })
}

fn parse_batch(
    batch: &RecordBatch,
    geometry_column: &str,
    features: &mut Vec<FeatureRecord>,
    next_id: &mut usize,
    min_x: &mut f64,
    min_y: &mut f64,
    max_x: &mut f64,
    max_y: &mut f64,
) -> Result<(), VectorError> {
    let geometry_idx = batch
        .schema()
        .fields()
        .iter()
        .position(|field| field.name() == geometry_column)
        .ok_or_else(|| {
            VectorError::GeoParquet(format!("geometry column `{geometry_column}` not found"))
        })?;

    let geometry_col = batch.column(geometry_idx);
    let geometry_array = geometry_col
        .as_any()
        .downcast_ref::<BinaryArray>()
        .ok_or_else(|| {
            VectorError::GeoParquet(format!(
                "geometry column `{geometry_column}` is not binary WKB"
            ))
        })?;

    for row in 0..batch.num_rows() {
        if geometry_array.is_null(row) {
            continue;
        }

        let wkb = geometry_array.value(row);
        let geometry = read_wkb(wkb)
            .map_err(|err| VectorError::GeoParquet(format!("WKB decode failed: {err}")))?
            .to_geometry();

        let rings = geo_geometry_to_rings(&geometry)?;
        for ring in &rings {
            for (x, y) in ring.exterior() {
                *min_x = min_x.min(*x);
                *min_y = min_y.min(*y);
                *max_x = max_x.max(*x);
                *max_y = max_y.max(*y);
            }
        }

        let properties = row_properties(batch, row, geometry_idx)?;
        features.push(FeatureRecord {
            id: *next_id,
            properties,
            rings,
        });
        *next_id += 1;
    }

    Ok(())
}

fn row_properties(
    batch: &RecordBatch,
    row: usize,
    geometry_idx: usize,
) -> Result<Value, VectorError> {
    let mut map = Map::new();
    for (idx, field) in batch.schema().fields().iter().enumerate() {
        if idx == geometry_idx {
            continue;
        }
        let value = array_value_at(batch.column(idx), row)?;
        map.insert(field.name().clone(), value);
    }
    Ok(Value::Object(map))
}

fn array_value_at(array: &dyn Array, row: usize) -> Result<Value, VectorError> {
    if array.is_null(row) {
        return Ok(Value::Null);
    }

    Ok(match array.data_type() {
        DataType::Utf8 => {
            let arr = array
                .as_any()
                .downcast_ref::<arrow_array::StringArray>()
                .expect("utf8 array");
            Value::String(arr.value(row).to_string())
        }
        DataType::LargeUtf8 => {
            let arr = array
                .as_any()
                .downcast_ref::<arrow_array::LargeStringArray>()
                .expect("large utf8 array");
            Value::String(arr.value(row).to_string())
        }
        DataType::Int64 => {
            let arr = array
                .as_any()
                .downcast_ref::<arrow_array::Int64Array>()
                .expect("int64 array");
            Value::Number(arr.value(row).into())
        }
        DataType::Int32 => {
            let arr = array
                .as_any()
                .downcast_ref::<arrow_array::Int32Array>()
                .expect("int32 array");
            Value::Number(arr.value(row).into())
        }
        DataType::UInt64 => {
            let arr = array
                .as_any()
                .downcast_ref::<arrow_array::UInt64Array>()
                .expect("uint64 array");
            Value::Number(arr.value(row).into())
        }
        DataType::Float64 => {
            let arr = array
                .as_any()
                .downcast_ref::<arrow_array::Float64Array>()
                .expect("float64 array");
            serde_json::Number::from_f64(arr.value(row))
                .map(Value::Number)
                .unwrap_or(Value::Null)
        }
        DataType::Boolean => {
            let arr = array
                .as_any()
                .downcast_ref::<arrow_array::BooleanArray>()
                .expect("bool array");
            Value::Bool(arr.value(row))
        }
        other => Value::String(format!("unsupported attribute type: {other:?}")),
    })
}

fn primary_geometry_column(metadata: &GeoParquetMetadata) -> Result<&str, VectorError> {
    if metadata.primary_column.is_empty() {
        return Err(VectorError::GeoParquet("no primary geometry column".into()));
    }
    Ok(metadata.primary_column.as_str())
}

fn crs_from_metadata(metadata: &GeoParquetMetadata, geometry_column: &str) -> String {
    metadata
        .columns
        .get(geometry_column)
        .and_then(|col| col.crs.as_ref())
        .map(crs_value_to_string)
        .unwrap_or_else(|| "EPSG:4326".to_string())
}

fn crs_value_to_string(value: &Value) -> String {
    if let Some(code) = value.get("code").and_then(|v| v.as_u64()) {
        let auth = value
            .get("authority")
            .or_else(|| value.get("auth_name"))
            .and_then(|v| v.as_str())
            .unwrap_or("EPSG");
        return format!("{auth}:{code}");
    }
    if let Some(s) = value.as_str() {
        return s.to_string();
    }
    "EPSG:4326".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geojson::read_geojson_path;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use arrow_schema::{DataType, Field, Schema};
    use geo_types::{Coord, Geometry, LineString, Polygon};
    use geoarrow_array::builder::GeometryBuilder;
    use geoarrow_array::GeoArrowArray;
    use geoarrow_schema::GeometryType;
    use geoparquet::writer::{GeoParquetRecordBatchEncoder, GeoParquetWriterOptions};
    use parquet::arrow::ArrowWriter;

    fn nagoya_geojson_path() -> &'static str {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/nagoya-population-density/data/nagoya-wards.geojson"
        )
    }

    fn write_nagoya_geoparquet_bytes() -> Vec<u8> {
        use std::sync::Arc;

        let dataset = read_geojson_path(nagoya_geojson_path()).expect("geojson");
        let mut ward_names = Vec::new();
        let mut ward_codes = Vec::new();
        let mut populations = Vec::new();

        for feature in &dataset.features {
            ward_names.push(
                feature
                    .properties
                    .get("ward_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            );
            ward_codes.push(
                feature
                    .properties
                    .get("ward_code")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            );
            populations.push(
                feature
                    .properties
                    .get("population")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as i64,
            );
        }

        let mut geom_builder = GeometryBuilder::new(GeometryType::default());
        for feature in &dataset.features {
            let geom = rings_to_geometry(&feature.rings);
            geom_builder
                .push_geometry(Some(&geom))
                .expect("push geometry");
        }
        let geom_array = geom_builder.finish();

        let geometry_field = GeometryType::default().to_field("geometry", false);
        let schema = Schema::new(vec![
            Field::new("ward_name", DataType::Utf8, false),
            Field::new("ward_code", DataType::Utf8, false),
            Field::new("population", DataType::Int64, false),
            geometry_field,
        ]);

        let batch = RecordBatch::try_new(
            Arc::new(schema.clone()),
            vec![
                Arc::new(StringArray::from(ward_names)) as _,
                Arc::new(StringArray::from(ward_codes)) as _,
                Arc::new(Int64Array::from(populations)) as _,
                geom_array.into_array_ref(),
            ],
        )
        .expect("record batch");

        let mut buffer = Vec::new();
        let options = GeoParquetWriterOptions::default();
        let mut encoder = GeoParquetRecordBatchEncoder::try_new(&schema, &options).expect("encoder");
        let mut writer =
            ArrowWriter::try_new(&mut buffer, encoder.target_schema(), None).expect("writer");
        let encoded = encoder.encode_record_batch(&batch).expect("encode");
        writer.write(&encoded).expect("write");
        writer
            .append_key_value_metadata(encoder.into_keyvalue().expect("metadata"));
        writer.close().expect("close");
        buffer
    }

    fn rings_to_geometry(rings: &[genegis_geometry::PolygonRing]) -> Geometry {
        let polygons: Vec<Polygon<f64>> = rings
            .iter()
            .map(|ring| {
                let coords: Vec<Coord<f64>> = ring
                    .exterior()
                    .iter()
                    .map(|(x, y)| Coord { x: *x, y: *y })
                    .collect();
                Polygon::new(LineString::from(coords), vec![])
            })
            .collect();
        if polygons.len() == 1 {
            Geometry::Polygon(polygons.into_iter().next().expect("polygon"))
        } else {
            Geometry::MultiPolygon(polygons.into())
        }
    }

    #[test]
    fn reads_nagoya_geoparquet_roundtrip() {
        let bytes = write_nagoya_geoparquet_bytes();
        let dataset = read_geoparquet_bytes(&bytes).expect("read geoparquet");
        assert_eq!(dataset.feature_count(), 16);
        assert!(dataset.crs.starts_with("EPSG:"));
        assert!(dataset.bbox.max.x > dataset.bbox.min.x);
        assert_eq!(
            dataset.features[0]
                .properties
                .get("ward_name")
                .and_then(|v| v.as_str())
                .is_some(),
            true
        );
    }

    #[test]
    fn reads_nagoya_geoparquet_path_roundtrip() {
        let bytes = write_nagoya_geoparquet_bytes();
        let path = std::env::temp_dir().join("genegis-nagoya-wards.parquet");
        std::fs::write(&path, bytes).expect("write temp parquet");
        let dataset = read_geoparquet_path(path.to_str().expect("path")).expect("read path");
        assert_eq!(dataset.feature_count(), 16);
        assert_eq!(dataset.name, "genegis-nagoya-wards");
    }
}
