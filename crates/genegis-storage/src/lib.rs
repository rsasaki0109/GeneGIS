//! GeneGIS storage — cloud-native asset IO (HTTP range reads).

#![deny(missing_docs)]

mod asset;
mod error;
mod http;
mod range;

pub use asset::{
    fetch_asset, is_remote_uri, read_asset_bytes, read_asset_range, read_local_bytes,
    read_local_range, AssetFetchResult, COG_HEADER_PREFIX_BYTES,
};
pub use error::StorageError;
pub use http::{
    fetch_http_bytes, fetch_http_range, parse_content_range_total, post_http_json_bytes,
    probe_http_content_length, HttpFetchResult,
};
pub use range::ByteRange;

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use super::*;

    fn spawn_http_fixture(body: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.set_nonblocking(true).expect("nonblocking");
        let addr = listener.local_addr().expect("addr");
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_thread = Arc::clone(&hits);
        let payload = Arc::new(body);

        thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while hits_thread.load(Ordering::SeqCst) < 4
                && std::time::Instant::now() < deadline
            {
                if let Ok((mut stream, _)) = listener.accept() {
                    handle_http_request(&mut stream, &payload, &hits_thread);
                } else {
                    thread::sleep(Duration::from_millis(10));
                }
            }
        });

        format!("http://{addr}/asset.bin")
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

    fn handle_http_request(stream: &mut TcpStream, body: &[u8], hits: &AtomicUsize) {
        let mut buffer = [0u8; 4096];
        let mut request_bytes = Vec::new();
        loop {
            let read = stream.read(&mut buffer).unwrap_or(0);
            if read == 0 {
                break;
            }
            request_bytes.extend_from_slice(&buffer[..read]);

            let Some(header_end) = request_bytes
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|position| position + 4)
            else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request_bytes[..header_end]);
            let content_length = header_value(&headers, "Content-Length")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            if request_bytes.len() >= header_end + content_length {
                break;
            }
        }
        if request_bytes.is_empty() {
            return;
        }
        let request = String::from_utf8_lossy(&request_bytes);
        hits.fetch_add(1, Ordering::SeqCst);

        if request.starts_with("HEAD ") {
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            return;
        }

        if let Some(spec) = header_value(&request, "Range") {
            let spec = spec.strip_prefix("bytes=").unwrap_or(spec);
            let (start, end) = spec.split_once('-').expect("range");
            let start: usize = start.parse().expect("start");
            let end: usize = end.parse().expect("end");
            let slice = &body[start..=end];
            let response = format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Type: application/octet-stream\r\nContent-Range: bytes {start}-{end}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len(),
                slice.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.write_all(slice);
            return;
        }

        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.write_all(body);
    }

    #[test]
    fn fetch_http_range_returns_partial_body() {
        let body: Vec<u8> = (0..10_000).map(|i| (i % 256) as u8).collect();
        let url = spawn_http_fixture(body.clone());
        thread::sleep(Duration::from_millis(50));

        let range = ByteRange::new(128, 255).expect("range");
        let result = fetch_http_range(&url, &range).expect("fetch");
        assert_eq!(result.status, 206);
        assert_eq!(result.bytes, body[128..=255]);
    }

    #[test]
    fn post_http_json_returns_response_body() {
        let response_body = br#"{"type":"FeatureCollection","features":[]}"#.to_vec();
        let url = spawn_http_fixture(response_body.clone());
        thread::sleep(Duration::from_millis(50));

        let result = post_http_json_bytes(&url, br#"{"limit":10}"#).expect("post");
        assert_eq!(result.status, 200);
        assert_eq!(result.bytes, response_body);
    }

    #[test]
    fn read_asset_range_works_for_local_files() {
        let temp = tempfile::NamedTempFile::new().expect("temp");
        let bytes: Vec<u8> = (0..=255).collect();
        std::fs::write(temp.path(), &bytes).expect("write");

        let path = temp.path().to_str().expect("path");
        let range = ByteRange::new(10, 19).expect("range");
        let slice = read_asset_range(path, &range).expect("read");
        assert_eq!(slice, bytes[10..=19]);
    }

    #[test]
    fn parse_content_range_total_reads_object_size() {
        assert_eq!(parse_content_range_total("bytes 0-0/12345"), Some(12345));
    }

    #[test]
    fn probe_http_content_length_uses_head_or_range() {
        let body: Vec<u8> = (0..5000).map(|i| (i % 256) as u8).collect();
        let url = spawn_http_fixture(body);
        thread::sleep(Duration::from_millis(50));

        let len = probe_http_content_length(&url).expect("probe");
        assert_eq!(len, 5000);
    }
}
