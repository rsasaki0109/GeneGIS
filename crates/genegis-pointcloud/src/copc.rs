use copc_streaming::{CopcStreamingReader, FileSource};
use serde::{Deserialize, Serialize};

use crate::error::PointcloudError;
use crate::http_source::HttpByteSource;
use crate::runtime::block_on;

/// Metadata summary for a Cloud Optimized Point Cloud (COPC) file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CopcInfo {
    pub path: Option<String>,
    pub point_count: u64,
    pub bounds: [f64; 6],
    pub crs: String,
    pub copc_center: [f64; 3],
    pub copc_halfsize: f64,
    pub copc_spacing: f64,
    pub hierarchy_entries: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_mode: Option<String>,
}

impl CopcInfo {
    pub fn summary_json(&self) -> serde_json::Value {
        serde_json::json!({
            "path": self.path,
            "point_count": self.point_count,
            "bounds": {
                "min_x": self.bounds[0],
                "min_y": self.bounds[1],
                "min_z": self.bounds[2],
                "max_x": self.bounds[3],
                "max_y": self.bounds[4],
                "max_z": self.bounds[5],
            },
            "crs": self.crs,
            "copc_center": self.copc_center,
            "copc_halfsize": self.copc_halfsize,
            "copc_spacing": self.copc_spacing,
            "hierarchy_entries": self.hierarchy_entries,
            "read_mode": self.read_mode,
        })
    }
}

/// Read COPC metadata from a local path or HTTP(S) URL.
pub fn read_copc_uri(uri: &str) -> Result<CopcInfo, PointcloudError> {
    if genegis_storage::is_remote_uri(uri) {
        read_copc_http_range(uri)
    } else {
        read_copc_path(uri)
    }
}

/// Read COPC metadata from a local file path.
pub fn read_copc_path(path: &str) -> Result<CopcInfo, PointcloudError> {
    let source = FileSource::open(path).map_err(map_copc_error)?;
    let reader = block_on(CopcStreamingReader::open(source)).map_err(map_copc_error)?;
    Ok(build_info(
        Some(path.to_string()),
        &reader,
        Some("local".into()),
    ))
}

/// Read COPC metadata from in-memory bytes.
pub fn read_copc_bytes(bytes: &[u8]) -> Result<CopcInfo, PointcloudError> {
    let reader = block_on(CopcStreamingReader::open(bytes))
        .map_err(map_copc_error)?;
    Ok(build_info(None, &reader, Some("bytes".into())))
}

fn read_copc_http_range(uri: &str) -> Result<CopcInfo, PointcloudError> {
    let source = HttpByteSource::open(uri).map_err(map_copc_error)?;
    let reader = block_on(CopcStreamingReader::open(source)).map_err(map_copc_error)?;
    Ok(build_info(
        Some(uri.to_string()),
        &reader,
        Some("http_range".into()),
    ))
}

fn build_info<S: copc_streaming::ByteSource>(
    path: Option<String>,
    reader: &CopcStreamingReader<S>,
    read_mode: Option<String>,
) -> CopcInfo {
    let header = reader.header();
    let las = header.las_header();
    let bounds = las.bounds();
    let copc = reader.copc_info();

    CopcInfo {
        path,
        point_count: las.number_of_points(),
        bounds: [
            bounds.min.x,
            bounds.min.y,
            bounds.min.z,
            bounds.max.x,
            bounds.max.y,
            bounds.max.z,
        ],
        crs: format_crs(las),
        copc_center: copc.center,
        copc_halfsize: copc.halfsize,
        copc_spacing: copc.spacing,
        hierarchy_entries: reader.node_count(),
        read_mode,
    }
}

fn format_crs(header: &las::Header) -> String {
    header
        .vlrs()
        .iter()
        .find_map(|vlr| {
            if vlr.user_id.eq_ignore_ascii_case("LASF_Projection") {
                Some("projected".to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn map_copc_error(err: copc_streaming::CopcError) -> PointcloudError {
    PointcloudError::Copc(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    fn fixture_path() -> &'static str {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/testdata/lone-star.copc.laz"
        )
    }

    struct HttpCopcFixture {
        url: String,
        range_requests: Arc<AtomicUsize>,
    }

    impl HttpCopcFixture {
        fn spawn(body: Vec<u8>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            listener.set_nonblocking(true).expect("nonblocking");
            let addr = listener.local_addr().expect("addr");
            let range_requests = Arc::new(AtomicUsize::new(0));
            let range_thread = Arc::clone(&range_requests);
            let payload = Arc::new(body);

            thread::spawn(move || {
                let deadline = std::time::Instant::now() + Duration::from_secs(10);
                while std::time::Instant::now() < deadline {
                    let Ok((mut stream, _)) = listener.accept() else {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    };
                    handle_request(&mut stream, &payload, &range_thread);
                }
            });

            Self {
                url: format!("http://{addr}/lone-star.copc.laz"),
                range_requests,
            }
        }
    }

    fn handle_request(
        stream: &mut TcpStream,
        body: &[u8],
        range_requests: &AtomicUsize,
    ) {
        let mut buffer = [0u8; 4096];
        let read = stream.read(&mut buffer).unwrap_or(0);
        if read == 0 {
            return;
        }
        let request = String::from_utf8_lossy(&buffer[..read]);
        let request_line = request.lines().next().unwrap_or("");

        if request_line.starts_with("HEAD ") {
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            return;
        }

        if let Some(spec) = header_value(&request, "Range") {
            range_requests.fetch_add(1, Ordering::SeqCst);
            let spec = spec.strip_prefix("bytes=").unwrap_or(spec);
            let (start, end) = spec.split_once('-').expect("range");
            let start: usize = start.parse().expect("start");
            let end: usize = end.parse().expect("end");
            let slice = &body[start..=end.min(body.len() - 1)];
            let response = format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Type: application/octet-stream\r\nAccept-Ranges: bytes\r\nContent-Range: bytes {start}-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                end.min(body.len() - 1),
                body.len(),
                slice.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.write_all(slice);
            return;
        }

        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.write_all(body);
    }

    fn header_value<'a>(request: &'a str, name: &str) -> Option<&'a str> {
        request.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            if key.eq_ignore_ascii_case(name) {
                Some(value.trim())
            } else {
                None
            }
        })
    }

    #[test]
    fn reads_copc_metadata_from_path() {
        let info = read_copc_path(fixture_path()).expect("read");
        assert_eq!(info.point_count, 518_862);
        assert_eq!(info.read_mode.as_deref(), Some("local"));
        assert!(info.hierarchy_entries > 0);
        assert!(info.copc_halfsize > 0.0);
    }

    #[test]
    fn reads_copc_metadata_from_bytes() {
        let bytes = std::fs::read(fixture_path()).expect("read fixture");
        let info = read_copc_bytes(&bytes).expect("read bytes");
        assert_eq!(info.point_count, 518_862);
        assert_eq!(info.read_mode.as_deref(), Some("bytes"));
    }

    #[test]
    fn reads_copc_metadata_over_http_range() {
        let bytes = std::fs::read(fixture_path()).expect("read fixture");
        let fixture = HttpCopcFixture::spawn(bytes);
        thread::sleep(Duration::from_millis(50));

        let info = read_copc_uri(&fixture.url).expect("read http");
        assert_eq!(info.point_count, 518_862);
        assert_eq!(info.read_mode.as_deref(), Some("http_range"));
        assert!(
            fixture.range_requests.load(Ordering::SeqCst) >= 1,
            "expected at least one HTTP range request"
        );
    }
}
