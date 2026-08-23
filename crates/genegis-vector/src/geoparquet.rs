use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use arrow_array::{Array, BinaryArray, RecordBatch};
use arrow_schema::DataType;
use bytes::Bytes;
use genegis_geometry::BoundingBox;
use geo_traits::to_geo::ToGeoGeometry;
use geoparquet::metadata::GeoParquetMetadata;
use geoparquet::reader::{GeoParquetReaderBuilder, GeoParquetRecordBatchReader};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::errors::ParquetError;
use parquet::file::reader::{ChunkReader, Length};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use wkb::reader::read_wkb;

use crate::dataset::{FeatureRecord, VectorDataset};
use crate::error::VectorError;
use crate::geometry::geo_geometry_to_rings;

/// Expected Nagoya ward feature count for bundled GeoParquet fixtures.
pub const NAGOYA_WARD_FEATURE_COUNT: usize = 16;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeoParquetReadOptions {
    /// Zero-based row groups to decode. `None` reads all row groups.
    pub row_groups: Option<Vec<usize>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoParquetReadReport {
    pub dataset: VectorDataset,
    pub source_uri: String,
    pub read_mode: String,
    pub content_length: u64,
    pub row_group_count: usize,
    pub selected_row_groups: Vec<usize>,
    pub schema_fields: Vec<String>,
    pub range_requests: usize,
    pub bytes_fetched: u64,
    pub retrieved_at: String,
}

/// Summarize a GeoParquet dataset for agent / CLI diagnostics.
pub fn geoparquet_summary(dataset: &VectorDataset) -> serde_json::Value {
    serde_json::json!({
        "name": dataset.name,
        "feature_count": dataset.feature_count(),
        "crs": dataset.crs,
        "bbox": {
            "min_x": dataset.bbox.min.x,
            "min_y": dataset.bbox.min.y,
            "max_x": dataset.bbox.max.x,
            "max_y": dataset.bbox.max.y,
        },
    })
}

/// Verify bundled Nagoya GeoParquet smoke expectations.
pub fn verify_nagoya_geoparquet(dataset: &VectorDataset) -> Result<bool, VectorError> {
    Ok(dataset.feature_count() == NAGOYA_WARD_FEATURE_COUNT && dataset.crs.starts_with("EPSG:"))
}

/// Read a GeoParquet file from disk into the shared [`VectorDataset`] model.
pub fn read_geoparquet_path(path: &str) -> Result<VectorDataset, VectorError> {
    let file = File::open(path)?;
    let name = Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unnamed")
        .to_string();
    read_geoparquet_chunk_reader(file, Some(name), None).map(|decoded| decoded.dataset)
}

/// Read GeoParquet from a local path or HTTP(S) URL.
pub fn read_geoparquet_uri(uri: &str) -> Result<VectorDataset, VectorError> {
    if genegis_storage::is_remote_uri(uri) {
        read_geoparquet_uri_with_options(uri, GeoParquetReadOptions::default())
            .map(|report| report.dataset)
    } else {
        read_geoparquet_path(uri)
    }
}

/// Read remote GeoParquet metadata and selected row groups using HTTP ranges.
pub fn read_geoparquet_uri_with_options(
    uri: &str,
    options: GeoParquetReadOptions,
) -> Result<GeoParquetReadReport, VectorError> {
    read_geoparquet_uri_with_options_and_policy(
        uri,
        options,
        genegis_storage::RemoteAccessPolicy::default(),
    )
}

/// Read remote GeoParquet under an explicit host, redirect, timeout, and size policy.
pub fn read_geoparquet_uri_with_options_and_policy(
    uri: &str,
    options: GeoParquetReadOptions,
    policy: genegis_storage::RemoteAccessPolicy,
) -> Result<GeoParquetReadReport, VectorError> {
    if !genegis_storage::is_remote_uri(uri) {
        return Err(VectorError::GeoParquet(
            "range-backed GeoParquet reader requires an HTTP(S) URI".into(),
        ));
    }
    let reader = HttpRangeChunkReader::open(uri, policy)?;
    let content_length = reader.len();
    let stats = Arc::clone(&reader.stats);
    let name = remote_dataset_name(uri);
    let decoded = read_geoparquet_chunk_reader(reader, Some(name), options.row_groups.as_deref())?;
    let selected_row_groups = options
        .row_groups
        .unwrap_or_else(|| (0..decoded.row_group_count).collect());
    Ok(GeoParquetReadReport {
        dataset: decoded.dataset,
        source_uri: uri.to_string(),
        read_mode: "http_range".into(),
        content_length,
        row_group_count: decoded.row_group_count,
        selected_row_groups,
        schema_fields: decoded.schema_fields,
        range_requests: stats.requests.load(Ordering::SeqCst),
        bytes_fetched: stats.bytes.load(Ordering::SeqCst),
        retrieved_at: chrono::Utc::now().to_rfc3339(),
    })
}

/// Read GeoParquet bytes (e.g. cloud object download) into [`VectorDataset`].
pub fn read_geoparquet_bytes(bytes: &[u8]) -> Result<VectorDataset, VectorError> {
    read_geoparquet_chunk_reader(Bytes::copy_from_slice(bytes), None, None)
        .map(|decoded| decoded.dataset)
}

struct DecodedGeoParquet {
    dataset: VectorDataset,
    row_group_count: usize,
    schema_fields: Vec<String>,
}

fn read_geoparquet_chunk_reader<R: ChunkReader + 'static>(
    reader: R,
    name: Option<String>,
    row_groups: Option<&[usize]>,
) -> Result<DecodedGeoParquet, VectorError> {
    let builder = ParquetRecordBatchReaderBuilder::try_new(reader)
        .map_err(|err| VectorError::GeoParquet(err.to_string()))?;
    let row_group_count = builder.metadata().num_row_groups();
    let schema_fields = builder
        .schema()
        .fields()
        .iter()
        .map(|field| field.name().clone())
        .collect();
    if let Some(row_groups) = row_groups {
        if row_groups.iter().any(|index| *index >= row_group_count) {
            return Err(VectorError::GeoParquet(format!(
                "row group selection {row_groups:?} exceeds available count {row_group_count}"
            )));
        }
    }

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

    let builder = builder.with_batch_size(1024);
    let builder = match row_groups {
        Some(row_groups) => builder.with_row_groups(row_groups.to_vec()),
        None => builder,
    };
    let parquet_reader = builder
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

    Ok(DecodedGeoParquet {
        dataset: VectorDataset {
            name: dataset_name,
            crs,
            features,
            bbox,
        },
        row_group_count,
        schema_fields,
    })
}

#[derive(Default)]
struct HttpRangeStats {
    requests: AtomicUsize,
    bytes: AtomicU64,
}

#[derive(Clone)]
struct HttpRangeChunkReader {
    uri: Arc<str>,
    content_length: u64,
    stats: Arc<HttpRangeStats>,
    policy: genegis_storage::RemoteAccessPolicy,
}

impl HttpRangeChunkReader {
    fn open(uri: &str, policy: genegis_storage::RemoteAccessPolicy) -> Result<Self, VectorError> {
        let content_length = genegis_storage::probe_http_content_length_with_policy(uri, &policy)
            .map_err(|error| VectorError::GeoParquet(error.to_string()))?;
        Ok(Self {
            uri: Arc::from(uri),
            content_length,
            stats: Arc::new(HttpRangeStats::default()),
            policy,
        })
    }

    fn fetch(&self, start: u64, length: usize) -> Result<Bytes, ParquetError> {
        if length == 0 {
            return Ok(Bytes::new());
        }
        let end = start
            .checked_add(length as u64 - 1)
            .ok_or_else(|| ParquetError::General("HTTP range overflow".into()))?;
        if end >= self.content_length {
            return Err(ParquetError::EOF(format!(
                "range {start}-{end} exceeds object length {}",
                self.content_length
            )));
        }
        let range = genegis_storage::ByteRange::new(start, end)
            .map_err(|error| ParquetError::General(error.to_string()))?;
        let bytes = genegis_storage::read_asset_range_with_policy(&self.uri, &range, &self.policy)
            .map_err(|error| ParquetError::General(error.to_string()))?;
        if bytes.len() != length {
            return Err(ParquetError::General(format!(
                "HTTP range length mismatch: requested {length}, received {}",
                bytes.len()
            )));
        }
        self.stats.requests.fetch_add(1, Ordering::SeqCst);
        self.stats
            .bytes
            .fetch_add(bytes.len() as u64, Ordering::SeqCst);
        Ok(Bytes::from(bytes))
    }
}

impl Length for HttpRangeChunkReader {
    fn len(&self) -> u64 {
        self.content_length
    }
}

impl ChunkReader for HttpRangeChunkReader {
    type T = HttpRangeStream;

    fn get_read(&self, start: u64) -> parquet::errors::Result<Self::T> {
        if start > self.content_length {
            return Err(ParquetError::EOF(format!(
                "offset {start} exceeds object length {}",
                self.content_length
            )));
        }
        Ok(HttpRangeStream {
            source: self.clone(),
            position: start,
            buffered: Bytes::new(),
        })
    }

    fn get_bytes(&self, start: u64, length: usize) -> parquet::errors::Result<Bytes> {
        self.fetch(start, length)
    }
}

struct HttpRangeStream {
    source: HttpRangeChunkReader,
    position: u64,
    buffered: Bytes,
}

impl Read for HttpRangeStream {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if buffer.is_empty() || self.position >= self.source.content_length {
            return Ok(0);
        }
        if self.buffered.is_empty() {
            let length = 65_536usize
                .max(buffer.len())
                .min((self.source.content_length - self.position) as usize);
            self.buffered = self
                .source
                .fetch(self.position, length)
                .map_err(std::io::Error::other)?;
        }
        let length = buffer.len().min(self.buffered.len());
        buffer[..length].copy_from_slice(&self.buffered.split_to(length));
        self.position += length as u64;
        Ok(length)
    }
}

fn remote_dataset_name(uri: &str) -> String {
    uri.split(['/', '\\'])
        .next_back()
        .and_then(|name| name.split('?').next())
        .and_then(|name| name.strip_suffix(".parquet").or(Some(name)))
        .filter(|name| !name.is_empty())
        .unwrap_or("geoparquet")
        .to_string()
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
    use parquet::file::properties::WriterProperties;

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
        let mut encoder =
            GeoParquetRecordBatchEncoder::try_new(&schema, &options).expect("encoder");
        let properties = WriterProperties::builder()
            .set_max_row_group_row_count(Some(8))
            .build();
        let mut writer =
            ArrowWriter::try_new(&mut buffer, encoder.target_schema(), Some(properties))
                .expect("writer");
        let encoded = encoder.encode_record_batch(&batch).expect("encode");
        writer.write(&encoded).expect("write");
        writer.append_key_value_metadata(encoder.into_keyvalue().expect("metadata"));
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
                let holes = ring
                    .holes()
                    .iter()
                    .map(|hole| {
                        LineString::from(
                            hole.iter()
                                .map(|(x, y)| Coord { x: *x, y: *y })
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect();
                Polygon::new(LineString::from(coords), holes)
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

    #[test]
    fn reads_selected_remote_row_group_with_http_ranges() {
        let bytes = write_nagoya_geoparquet_bytes();
        let url = spawn_range_fixture(bytes);
        let report = read_geoparquet_uri_with_options(
            &url,
            GeoParquetReadOptions {
                row_groups: Some(vec![1]),
            },
        )
        .expect("remote row group");

        assert_eq!(report.read_mode, "http_range");
        assert_eq!(report.row_group_count, 2);
        assert_eq!(report.selected_row_groups, vec![1]);
        assert_eq!(report.dataset.feature_count(), 8);
        assert!(report.range_requests > 0);
        assert!(report.bytes_fetched > 0);
        assert!(report.schema_fields.iter().any(|field| field == "geometry"));
    }

    #[test]
    fn probes_remote_metadata_without_decoding_row_groups() {
        let bytes = write_nagoya_geoparquet_bytes();
        let url = spawn_range_fixture(bytes);
        let report = read_geoparquet_uri_with_options(
            &url,
            GeoParquetReadOptions {
                row_groups: Some(Vec::new()),
            },
        )
        .expect("remote metadata");

        assert_eq!(report.row_group_count, 2);
        assert!(report.selected_row_groups.is_empty());
        assert_eq!(report.dataset.feature_count(), 0);
        assert!(report.dataset.crs.starts_with("EPSG:"));
    }

    fn spawn_range_fixture(body: Vec<u8>) -> String {
        use std::io::Write;
        use std::net::{Shutdown, TcpListener};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::{Duration, Instant};

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.set_nonblocking(true).expect("nonblocking");
        let address = listener.local_addr().expect("address");
        let body = Arc::new(body);
        let requests = Arc::new(AtomicUsize::new(0));
        std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline && requests.load(Ordering::SeqCst) < 1_000 {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                };
                let mut request = Vec::new();
                let mut buffer = [0u8; 4096];
                loop {
                    let read = stream.read(&mut buffer).unwrap_or(0);
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..read]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                requests.fetch_add(1, Ordering::SeqCst);
                let request = String::from_utf8_lossy(&request);
                if request.starts_with("HEAD ") {
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    stream.write_all(response.as_bytes()).expect("head");
                } else if let Some(range) = test_header_value(&request, "Range") {
                    let (start, end) = range
                        .strip_prefix("bytes=")
                        .unwrap_or(range)
                        .split_once('-')
                        .expect("range");
                    let start: usize = start.parse().expect("start");
                    let end: usize = end.parse().expect("end");
                    let slice = &body[start..=end];
                    let response = format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{end}/{}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                        slice.len(),
                        body.len()
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("range headers");
                    stream.write_all(slice).expect("range body");
                } else {
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    stream.write_all(response.as_bytes()).expect("headers");
                    stream.write_all(&body).expect("body");
                }
                stream.flush().expect("flush");
                let _ = stream.shutdown(Shutdown::Write);
            }
        });
        format!("http://{address}/wards.parquet")
    }

    fn test_header_value<'a>(request: &'a str, name: &str) -> Option<&'a str> {
        request.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name).then_some(value.trim())
        })
    }
}
