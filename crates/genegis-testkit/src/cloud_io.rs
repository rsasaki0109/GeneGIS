//! Reproducible range-I/O and headless GPU benchmark lane.

use genegis_pointcloud::read_copc_uri;
use genegis_raster::read_cog_window_uri;
use genegis_render::benchmark_headless_gpu;
use genegis_storage::{
    fetch_http_range_with_policy, probe_http_object_metadata_with_policy, ByteRange, CloudFormat,
    GpuFrameMetrics, HttpObjectMetadata, IoBudget, IoBudgetFailure, IoReceipt, IoRequestEvidence,
    IoSelection, RemoteAccessPolicy,
};
use genegis_tile::read_pmtiles_tile;
use genegis_vector::{read_geoparquet_uri_with_options, GeoParquetReadOptions};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};

/// Timings and representative receipt for one cloud-native format.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudFormatBenchmark {
    /// Format.
    pub format: CloudFormat,
    /// Original full-size object URL; absent for the deterministic CI lane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    /// Entity tag observed before the benchmark.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_etag: Option<String>,
    /// Last-Modified header observed before the benchmark.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_last_modified: Option<String>,
    /// Five cold HTTP-selection samples.
    pub samples_ns: Vec<u64>,
    /// Median duration.
    pub p50_ns: u64,
    /// Nearest-rank 95th percentile duration.
    pub p95_ns: u64,
    /// Receipt from the final measured iteration.
    pub receipt: IoReceipt,
    /// Budget failures; empty means pass.
    pub budget_failures: Vec<IoBudgetFailure>,
}

/// Combined four-format I/O and real wgpu execution report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudIoBenchmarkReport {
    /// Report schema version.
    pub schema_version: String,
    /// Deterministic CI or full-size lane.
    pub lane: String,
    /// Format results.
    pub formats: Vec<CloudFormatBenchmark>,
    /// Actual wgpu adapter and frame evidence.
    pub gpu: GpuFrameMetrics,
    /// Whether the adapter reports hardware rather than CPU device type.
    pub hardware_gpu: bool,
    /// Number of budget failures across all format receipts and GPU checks.
    pub failed_gates: usize,
}

/// Run deterministic COG, GeoParquet, COPC, PMTiles range selection and headless wgpu frames.
pub fn run_cloud_io_benchmark() -> Result<CloudIoBenchmarkReport, String> {
    let cog = padded_fixture(
        include_bytes!("../../genegis-raster/fixtures/smoke-demo.tif"),
        2 * 1024 * 1024,
    );
    let geoparquet = large_geoparquet_fixture()?;
    let copc = padded_fixture(
        include_bytes!("../../genegis-pointcloud/testdata/lone-star.copc.laz"),
        16 * 1024 * 1024,
    );
    let pmtiles = pmtiles_fixture(1024 * 1024);

    let mut formats = vec![
        benchmark_fixture(CloudFormat::Cog, cog, |url| {
            let pixels = read_cog_window_uri(url, 0, 0, 8, 8).map_err(|error| error.to_string())?;
            Ok((
                pixels.len() as u64,
                IoSelection::CogWindow {
                    level: 0,
                    row_offset: 0,
                    column_offset: 0,
                    rows: 8,
                    columns: 8,
                },
            ))
        })?,
        benchmark_fixture(CloudFormat::GeoParquet, geoparquet, |url| {
            let report = read_geoparquet_uri_with_options(
                url,
                GeoParquetReadOptions {
                    row_groups: Some(vec![0]),
                },
            )
            .map_err(|error| error.to_string())?;
            Ok((
                report.dataset.feature_count() as u64,
                IoSelection::GeoParquetRowGroups {
                    row_groups: vec![0],
                    bbox: None,
                },
            ))
        })?,
        benchmark_fixture(CloudFormat::Copc, copc, |url| {
            let info = read_copc_uri(url).map_err(|error| error.to_string())?;
            Ok((
                info.hierarchy_entries as u64,
                IoSelection::CopcHierarchyNodes {
                    node_keys: vec!["root-metadata-hierarchy".into()],
                },
            ))
        })?,
        benchmark_fixture(CloudFormat::PmTiles, pmtiles, |url| {
            read_pmtiles_tile(url, 0, 0, 0).map_err(|error| error.to_string())?;
            Ok((1, IoSelection::PmTilesTile { z: 0, x: 0, y: 0 }))
        })?,
    ];

    let map = genegis_analysis::nagoya_choropleth_map().map_err(|error| error.to_string())?;
    let measured = benchmark_headless_gpu(&map, 1280, 720, 30)?;
    let hardware_gpu = matches!(
        measured.device_type.as_str(),
        "discretegpu" | "integratedgpu"
    );
    let gpu = GpuFrameMetrics {
        adapter: measured.adapter,
        backend: measured.backend,
        upload_bytes: measured.upload_bytes,
        upload_ns: measured.upload_ns,
        first_frame_ns: measured.first_frame_ns,
        steady_state_fps: measured.steady_state_fps,
    };
    let mut failed_gates = formats
        .iter()
        .map(|format| format.budget_failures.len())
        .sum::<usize>();
    if gpu.first_frame_ns > 2_000_000_000
        || !gpu.steady_state_fps.is_finite()
        || gpu.steady_state_fps <= 0.0
    {
        failed_gates += 1;
    }
    formats.sort_by_key(|format| format.format as u8);
    Ok(CloudIoBenchmarkReport {
        schema_version: "0.1.0".into(),
        lane: "deterministic_ci".into(),
        formats,
        gpu,
        hardware_gpu,
        failed_gates,
    })
}

/// Run the full Phase 12 lane against four public, version-observed objects.
///
/// The local proxy rejects whole-object GETs, forwards only byte ranges up to
/// 8 MiB, and records the exact offsets returned by the upstream server.
pub fn run_full_cloud_io_benchmark() -> Result<CloudIoBenchmarkReport, String> {
    let mut formats = vec![
        benchmark_remote(
            CloudFormat::Cog,
            "https://s3-west.nrp-nautilus.io/public-land-cover/cgls-lc100-2019-cog.tif",
            "s3-west.nrp-nautilus.io",
            RemoteIdentityExpectation {
                content_length: 2_421_291_146,
                etag: "\"97b8b9611c38020d66b92b40adab091a-462\"",
                last_modified: "Sun, 05 Apr 2026 20:37:55 GMT",
            },
            |url| {
                let pixels =
                    read_cog_window_uri(url, 0, 0, 8, 8).map_err(|error| error.to_string())?;
                Ok((
                    pixels.len() as u64,
                    IoSelection::CogWindow {
                        level: 0,
                        row_offset: 0,
                        column_offset: 0,
                        rows: 8,
                        columns: 8,
                    },
                    None,
                ))
            },
        )?,
        benchmark_remote(
            CloudFormat::GeoParquet,
            "https://data.source.coop/youssef-harby/geoparquet-overviews/v0.3.0/nls_rakennus_overviews.parquet",
            "data.source.coop",
            RemoteIdentityExpectation {
                content_length: 290_358_319,
                etag: "\"514daa2404ceadb3787e6a9cc5e03b15-6\"",
                last_modified: "Thu, 09 Jul 2026 18:51:22 GMT",
            },
            |url| {
                let report = read_geoparquet_uri_with_options(
                    url,
                    GeoParquetReadOptions {
                        row_groups: Some(vec![0]),
                    },
                )
                .map_err(|error| error.to_string())?;
                Ok((
                    report.dataset.feature_count() as u64,
                    IoSelection::GeoParquetRowGroups {
                        row_groups: vec![0],
                        bbox: None,
                    },
                    Some(report.logical_dataset_bytes),
                ))
            },
        )?,
        benchmark_remote(
            CloudFormat::Copc,
            "https://hobu-lidar.s3.amazonaws.com/sofi.copc.laz",
            "hobu-lidar.s3.amazonaws.com",
            RemoteIdentityExpectation {
                content_length: 2_029_696_615,
                etag: "\"5d92a180a4c9890a51ba04a6fb107f26-242\"",
                last_modified: "Mon, 29 Nov 2021 22:12:42 GMT",
            },
            |url| {
                let info = read_copc_uri(url).map_err(|error| error.to_string())?;
                Ok((
                    info.hierarchy_entries as u64,
                    IoSelection::CopcHierarchyNodes {
                        node_keys: vec!["root-metadata-hierarchy".into()],
                    },
                    None,
                ))
            },
        )?,
        benchmark_remote(
            CloudFormat::PmTiles,
            "https://r2-public.protomaps.com/protomaps-sample-datasets/terrarium-z12.pmtiles",
            "r2-public.protomaps.com",
            RemoteIdentityExpectation {
                content_length: 159_822_873_635,
                etag: "\"2c65d40064244bed8874cdf03739bf61-596\"",
                last_modified: "Tue, 21 Feb 2023 12:06:55 GMT",
            },
            |url| {
                read_pmtiles_tile(url, 0, 0, 0).map_err(|error| error.to_string())?;
                Ok((
                    1,
                    IoSelection::PmTilesTile { z: 0, x: 0, y: 0 },
                    None,
                ))
            },
        )?,
    ];
    let map = genegis_analysis::nagoya_choropleth_map().map_err(|error| error.to_string())?;
    let measured = benchmark_headless_gpu(&map, 1280, 720, 30)?;
    let hardware_gpu = matches!(
        measured.device_type.as_str(),
        "discretegpu" | "integratedgpu"
    );
    let gpu = GpuFrameMetrics {
        adapter: measured.adapter,
        backend: measured.backend,
        upload_bytes: measured.upload_bytes,
        upload_ns: measured.upload_ns,
        first_frame_ns: measured.first_frame_ns,
        steady_state_fps: measured.steady_state_fps,
    };
    let mut failed_gates = formats
        .iter()
        .map(|format| format.budget_failures.len())
        .sum::<usize>();
    if !hardware_gpu
        || gpu.first_frame_ns > 2_000_000_000
        || !gpu.steady_state_fps.is_finite()
        || gpu.steady_state_fps <= 0.0
    {
        failed_gates += 1;
    }
    formats.sort_by_key(|format| format.format as u8);
    Ok(CloudIoBenchmarkReport {
        schema_version: "0.1.0".into(),
        lane: "public_full_size".into(),
        formats,
        gpu,
        hardware_gpu,
        failed_gates,
    })
}

fn benchmark_fixture<F>(
    format: CloudFormat,
    body: Vec<u8>,
    mut operation: F,
) -> Result<CloudFormatBenchmark, String>
where
    F: FnMut(&str) -> Result<(u64, IoSelection), String>,
{
    let fixture = RangeFixture::spawn(body);
    let object_digest = format!("sha256:{:x}", Sha256::digest(&*fixture.body));
    let mut samples = Vec::new();
    let mut final_receipt = None;
    for _ in 0..5 {
        let request_start = fixture.snapshot_len();
        let full_start = fixture.full_gets.load(Ordering::SeqCst);
        let started = Instant::now();
        let (decoded_items, selection) = operation(&fixture.url)?;
        let elapsed_ns = nanos(started.elapsed());
        let requests = fixture.requests_since(request_start);
        let whole_object_fallback = fixture.full_gets.load(Ordering::SeqCst) > full_start;
        final_receipt = Some(IoReceipt::new(
            format,
            object_digest.clone(),
            fixture.body.len() as u64,
            (fixture.body.len() as u64).saturating_mul(4),
            selection,
            requests,
            whole_object_fallback,
            decoded_items,
            elapsed_ns,
            peak_rss_bytes(),
            None,
        ));
        samples.push(elapsed_ns);
    }
    samples.sort_unstable();
    let receipt = final_receipt.expect("five samples");
    Ok(CloudFormatBenchmark {
        format,
        source_url: None,
        source_etag: None,
        source_last_modified: None,
        p50_ns: percentile(&samples, 0.50),
        p95_ns: percentile(&samples, 0.95),
        samples_ns: samples,
        budget_failures: receipt.validate(&IoBudget::ci(false)),
        receipt,
    })
}

fn benchmark_remote<F>(
    format: CloudFormat,
    source_url: &str,
    allowed_host: &str,
    expected: RemoteIdentityExpectation<'_>,
    mut operation: F,
) -> Result<CloudFormatBenchmark, String>
where
    F: FnMut(&str) -> Result<(u64, IoSelection, Option<u64>), String>,
{
    let proxy = RemoteRangeProxy::spawn(source_url, allowed_host)?;
    expected.verify(source_url, &proxy.metadata)?;
    if !proxy.metadata.accepts_byte_ranges {
        return Err(format!(
            "upstream does not advertise byte ranges: {source_url}"
        ));
    }
    let object_identity = digest_remote_identity(source_url, &proxy.metadata);
    let mut samples = Vec::new();
    let mut final_receipt = None;
    for _ in 0..5 {
        let request_start = proxy.snapshot_len();
        let fallback_start = proxy.full_gets.load(Ordering::SeqCst);
        let started = Instant::now();
        let (decoded_items, selection, logical_dataset_bytes) = operation(&proxy.url)?;
        let elapsed_ns = nanos(started.elapsed());
        let requests = proxy.requests_since(request_start);
        let whole_object_fallback = proxy.full_gets.load(Ordering::SeqCst) > fallback_start;
        final_receipt = Some(IoReceipt::new(
            format,
            object_identity.clone(),
            proxy.metadata.content_length,
            logical_dataset_bytes.unwrap_or(proxy.metadata.content_length),
            selection,
            requests,
            whole_object_fallback,
            decoded_items,
            elapsed_ns,
            peak_rss_bytes(),
            None,
        ));
        samples.push(elapsed_ns);
    }
    samples.sort_unstable();
    let receipt = final_receipt.expect("five full-size samples");
    Ok(CloudFormatBenchmark {
        format,
        source_url: Some(source_url.into()),
        source_etag: proxy.metadata.etag.clone(),
        source_last_modified: proxy.metadata.last_modified.clone(),
        p50_ns: percentile(&samples, 0.50),
        p95_ns: percentile(&samples, 0.95),
        samples_ns: samples,
        budget_failures: receipt.validate(&IoBudget::phase_12_full(false)),
        receipt,
    })
}

#[derive(Debug, Clone, Copy)]
struct RemoteIdentityExpectation<'a> {
    content_length: u64,
    etag: &'a str,
    last_modified: &'a str,
}

impl RemoteIdentityExpectation<'_> {
    fn verify(self, url: &str, observed: &HttpObjectMetadata) -> Result<(), String> {
        let matches = observed.content_length == self.content_length
            && observed.etag.as_deref() == Some(self.etag)
            && observed.last_modified.as_deref() == Some(self.last_modified);
        if matches {
            return Ok(());
        }
        Err(format!(
            "remote fixture identity drift for {url}: expected length={} etag={:?} last_modified={:?}, observed {observed:?}",
            self.content_length, self.etag, self.last_modified
        ))
    }
}

fn digest_remote_identity(url: &str, metadata: &HttpObjectMetadata) -> String {
    let identity = serde_json::json!({
        "url": url,
        "content_length": metadata.content_length,
        "etag": metadata.etag,
        "last_modified": metadata.last_modified,
    });
    format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(&identity).expect("identity JSON"))
    )
}

fn padded_fixture(input: &[u8], minimum: usize) -> Vec<u8> {
    let mut output = input.to_vec();
    output.resize(minimum.max(output.len()), 0);
    output
}

fn large_geoparquet_fixture() -> Result<Vec<u8>, String> {
    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use arrow_schema::{DataType, Field, Schema};
    use geo_types::{Coord, Geometry, LineString, Polygon};
    use geoarrow_array::builder::GeometryBuilder;
    use geoarrow_array::GeoArrowArray;
    use geoarrow_schema::GeometryType;
    use geoparquet::writer::{GeoParquetRecordBatchEncoder, GeoParquetWriterOptions};
    use parquet::arrow::ArrowWriter;
    use parquet::file::properties::WriterProperties;

    let dataset = genegis_vector::read_geojson_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/nagoya-population-density/data/nagoya-wards.geojson"
    ))
    .map_err(|error| error.to_string())?;
    let mut names = Vec::new();
    let mut codes = Vec::new();
    let mut populations = Vec::new();
    let mut geometries = GeometryBuilder::new(GeometryType::default());
    for repetition in 0..16 {
        for feature in &dataset.features {
            names.push(format!(
                "{}-{repetition}",
                feature.properties["ward_name"].as_str().unwrap_or("ward")
            ));
            codes.push(format!(
                "{}-{repetition}",
                feature.properties["ward_code"].as_str().unwrap_or("code")
            ));
            populations
                .push(feature.properties["population"].as_i64().unwrap_or(0) + repetition as i64);
            let polygons = feature
                .rings
                .iter()
                .map(|ring| {
                    let exterior = ring
                        .exterior()
                        .iter()
                        .map(|(x, y)| Coord { x: *x, y: *y })
                        .collect::<Vec<_>>();
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
                    Polygon::new(LineString::from(exterior), holes)
                })
                .collect::<Vec<Polygon<f64>>>();
            let geometry = if polygons.len() == 1 {
                Geometry::Polygon(polygons.into_iter().next().expect("one polygon"))
            } else {
                Geometry::MultiPolygon(polygons.into())
            };
            geometries
                .push_geometry(Some(&geometry))
                .map_err(|error| error.to_string())?;
        }
    }
    let geometry_array = geometries.finish();
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
            Arc::new(StringArray::from(names)) as _,
            Arc::new(StringArray::from(codes)) as _,
            Arc::new(Int64Array::from(populations)) as _,
            geometry_array.into_array_ref(),
        ],
    )
    .map_err(|error| error.to_string())?;
    let options = GeoParquetWriterOptions::default();
    let mut encoder = GeoParquetRecordBatchEncoder::try_new(&schema, &options)
        .map_err(|error| error.to_string())?;
    let properties = WriterProperties::builder()
        .set_max_row_group_row_count(Some(16))
        .build();
    let mut output = Vec::new();
    let mut writer = ArrowWriter::try_new(&mut output, encoder.target_schema(), Some(properties))
        .map_err(|error| error.to_string())?;
    let encoded = encoder
        .encode_record_batch(&batch)
        .map_err(|error| error.to_string())?;
    writer.write(&encoded).map_err(|error| error.to_string())?;
    writer.append_key_value_metadata(encoder.into_keyvalue().map_err(|error| error.to_string())?);
    writer.close().map_err(|error| error.to_string())?;
    Ok(output)
}

fn pmtiles_fixture(object_bytes: usize) -> Vec<u8> {
    let mut bytes = vec![0_u8; 127];
    bytes[..7].copy_from_slice(b"PMTiles");
    bytes[7] = 3;
    for (offset, value) in [
        (8, 127_u64),
        (16, 5),
        (24, 132),
        (32, 2),
        (40, 134),
        (48, 0),
        (56, 134),
        (64, 5),
        (72, 1),
        (80, 1),
        (88, 1),
    ] {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
    bytes[96] = 1;
    bytes[97] = 1;
    bytes[98] = 1;
    bytes[99] = 1;
    bytes.extend([1_u8, 0, 1, 5, 1]);
    bytes.extend(b"{}");
    bytes.extend(b"hello");
    bytes.resize(object_bytes, 0);
    bytes
}

fn percentile(sorted: &[u64], quantile: f64) -> u64 {
    let rank = (quantile * sorted.len() as f64).ceil().max(1.0) as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

fn nanos(duration: Duration) -> u64 {
    duration.as_nanos().min(u64::MAX as u128) as u64
}

fn peak_rss_bytes() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status.lines().find_map(|line| {
                line.strip_prefix("VmHWM:")?
                    .split_whitespace()
                    .next()?
                    .parse::<u64>()
                    .ok()
                    .map(|kilobytes| kilobytes * 1024)
            })
        })
        .unwrap_or(0)
}

struct RemoteRangeProxy {
    url: String,
    metadata: HttpObjectMetadata,
    requests: Arc<Mutex<Vec<IoRequestEvidence>>>,
    full_gets: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    address: std::net::SocketAddr,
    thread: Option<thread::JoinHandle<()>>,
}

impl RemoteRangeProxy {
    fn spawn(source_url: &str, allowed_host: &str) -> Result<Self, String> {
        let policy = RemoteAccessPolicy {
            allowed_hosts: vec![allowed_host.into()],
            allow_loopback: false,
            max_response_bytes: 8 * 1024 * 1024,
            timeout_ms: 30_000,
            max_redirects: 0,
        };
        let metadata = probe_http_object_metadata_with_policy(source_url, &policy)
            .map_err(|error| error.to_string())?;
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
        listener
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;
        let address = listener.local_addr().map_err(|error| error.to_string())?;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let full_gets = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let source_thread = source_url.to_string();
        let metadata_thread = metadata.clone();
        let policy_thread = policy;
        let requests_thread = Arc::clone(&requests);
        let full_thread = Arc::clone(&full_gets);
        let stop_thread = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            while !stop_thread.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => serve_remote_range(
                        &mut stream,
                        &source_thread,
                        &metadata_thread,
                        &policy_thread,
                        &requests_thread,
                        &full_thread,
                    ),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            url: format!("http://{address}/object"),
            metadata,
            requests,
            full_gets,
            stop,
            address,
            thread: Some(thread),
        })
    }

    fn snapshot_len(&self) -> usize {
        self.requests.lock().expect("remote request evidence").len()
    }

    fn requests_since(&self, index: usize) -> Vec<IoRequestEvidence> {
        self.requests.lock().expect("remote request evidence")[index..].to_vec()
    }
}

impl Drop for RemoteRangeProxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.address);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve_remote_range(
    stream: &mut TcpStream,
    source_url: &str,
    metadata: &HttpObjectMetadata,
    policy: &RemoteAccessPolicy,
    requests: &Mutex<Vec<IoRequestEvidence>>,
    full_gets: &AtomicUsize,
) {
    let mut buffer = [0_u8; 8192];
    let read = stream.read(&mut buffer).unwrap_or(0);
    if read == 0 {
        return;
    }
    let request = String::from_utf8_lossy(&buffer[..read]);
    if request.starts_with("HEAD ") {
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
            metadata.content_length
        );
        return;
    }
    let Some(spec) = header_value(&request, "Range") else {
        full_gets.fetch_add(1, Ordering::SeqCst);
        let _ = stream.write_all(
            b"HTTP/1.1 412 Precondition Failed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        return;
    };
    let Ok(range) = ByteRange::parse(spec) else {
        let _ = stream.write_all(
            b"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        return;
    };
    if range.end >= metadata.content_length || range.len() > 8 * 1024 * 1024 {
        let _ = stream.write_all(
            b"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        return;
    }
    let Ok(response) = fetch_http_range_with_policy(source_url, &range, policy) else {
        let _ = stream.write_all(
            b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        return;
    };
    if response.status != 206 {
        full_gets.fetch_add(1, Ordering::SeqCst);
        let _ = stream.write_all(
            b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        return;
    }
    requests
        .lock()
        .expect("remote request evidence")
        .push(IoRequestEvidence {
            start: range.start,
            end: range.end,
            response_bytes: response.bytes.len() as u64,
            http_status: Some(206),
        });
    let _ = write!(
        stream,
        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {}-{}/{}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
        range.start,
        range.end,
        metadata.content_length,
        response.bytes.len()
    );
    let _ = stream.write_all(&response.bytes);
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Write);
}

struct RangeFixture {
    url: String,
    body: Arc<Vec<u8>>,
    requests: Arc<Mutex<Vec<IoRequestEvidence>>>,
    full_gets: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    address: std::net::SocketAddr,
    thread: Option<thread::JoinHandle<()>>,
}

impl RangeFixture {
    fn spawn(body: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("range fixture bind");
        listener.set_nonblocking(true).expect("nonblocking fixture");
        let address = listener.local_addr().expect("fixture address");
        let body = Arc::new(body);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let full_gets = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let body_thread = Arc::clone(&body);
        let requests_thread = Arc::clone(&requests);
        let full_thread = Arc::clone(&full_gets);
        let stop_thread = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            while !stop_thread.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        serve_range(&mut stream, &body_thread, &requests_thread, &full_thread)
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            url: format!("http://{address}/fixture"),
            body,
            requests,
            full_gets,
            stop,
            address,
            thread: Some(thread),
        }
    }

    fn snapshot_len(&self) -> usize {
        self.requests.lock().expect("request evidence").len()
    }

    fn requests_since(&self, index: usize) -> Vec<IoRequestEvidence> {
        self.requests.lock().expect("request evidence")[index..].to_vec()
    }
}

impl Drop for RangeFixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.address);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve_range(
    stream: &mut TcpStream,
    body: &[u8],
    requests: &Mutex<Vec<IoRequestEvidence>>,
    full_gets: &AtomicUsize,
) {
    let mut buffer = [0_u8; 8192];
    let read = stream.read(&mut buffer).unwrap_or(0);
    if read == 0 {
        return;
    }
    let request = String::from_utf8_lossy(&buffer[..read]);
    if request.starts_with("HEAD ") {
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
            body.len()
        );
        return;
    }
    if let Some(spec) = header_value(&request, "Range") {
        let spec = spec.strip_prefix("bytes=").unwrap_or(spec);
        let Some((start, end)) = spec.split_once('-') else {
            return;
        };
        let (Ok(start), Ok(requested_end)) = (start.parse::<usize>(), end.parse::<usize>()) else {
            return;
        };
        if start >= body.len() {
            return;
        }
        let end = requested_end.min(body.len() - 1);
        let slice = &body[start..=end];
        requests
            .lock()
            .expect("request evidence")
            .push(IoRequestEvidence {
                start: start as u64,
                end: end as u64,
                response_bytes: slice.len() as u64,
                http_status: Some(206),
            });
        let _ = write!(
            stream,
            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
            body.len(),
            slice.len()
        );
        let _ = stream.write_all(slice);
    } else {
        full_gets.fetch_add(1, Ordering::SeqCst);
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(body);
    }
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Write);
}

fn header_value<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    request.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_formats_use_ranges_and_gpu_metrics_are_real() {
        let report = run_cloud_io_benchmark().expect("cloud I/O benchmark");
        assert_eq!(report.formats.len(), 4);
        assert_eq!(report.failed_gates, 0, "{report:#?}");
        assert!(report.formats.iter().all(|format| {
            !format.receipt.whole_object_fallback
                && !format.receipt.requests.is_empty()
                && format.receipt.transfer_ratio() <= 0.2
                && format.receipt.maximum_response_bytes <= 8 * 1024 * 1024
        }));
        assert!(!report.gpu.adapter.is_empty());
        assert!(report.gpu.first_frame_ns <= 2_000_000_000);
        println!("{}", serde_json::to_string(&report).unwrap());
    }

    #[test]
    #[ignore = "requires four public range endpoints and a hardware GPU"]
    fn public_full_size_objects_pass_phase_12_budgets() {
        let report = run_full_cloud_io_benchmark().expect("full cloud I/O benchmark");
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
        assert_eq!(report.formats.len(), 4);
        assert!(report.hardware_gpu, "{report:#?}");
        assert_eq!(report.failed_gates, 0, "{report:#?}");
        assert!(report.formats.iter().all(|format| {
            format.receipt.object_bytes >= 256 * 1024 * 1024
                && format.receipt.logical_dataset_bytes >= 1024 * 1024 * 1024
                && !format.receipt.whole_object_fallback
                && format.receipt.transfer_ratio() <= 0.2
                && format.receipt.maximum_response_bytes <= 8 * 1024 * 1024
        }));
    }
}
