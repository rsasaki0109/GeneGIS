use geotiff_reader::GeoTiffFile;
use geotiff_reader::cog::{HttpGeoTiffFile, HttpOpenOptions};
use serde::{Deserialize, Serialize};

use crate::error::RasterError;

/// Metadata summary for a Cloud Optimized GeoTIFF (COG) or GeoTIFF file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CogInfo {
    pub path: Option<String>,
    pub width: u32,
    pub height: u32,
    pub band_count: u32,
    pub epsg: Option<u32>,
    pub crs: String,
    pub geo_bounds: Option<[f64; 4]>,
    pub tiled: bool,
    pub tile_width: Option<u32>,
    pub tile_height: Option<u32>,
    pub overview_count: usize,
    pub cloud_optimized: bool,
    /// How the asset was opened (`local`, `http_range`, …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_mode: Option<String>,
}

impl CogInfo {
    pub fn summary_json(&self) -> serde_json::Value {
        serde_json::json!({
            "path": self.path,
            "width": self.width,
            "height": self.height,
            "band_count": self.band_count,
            "epsg": self.epsg,
            "crs": self.crs,
            "geo_bounds": self.geo_bounds,
            "tiled": self.tiled,
            "tile_width": self.tile_width,
            "tile_height": self.tile_height,
            "overview_count": self.overview_count,
            "cloud_optimized": self.cloud_optimized,
            "read_mode": self.read_mode,
        })
    }
}

/// Read COG / GeoTIFF metadata from a local path or HTTP(S) URL.
pub fn read_cog_uri(uri: &str) -> Result<CogInfo, RasterError> {
    read_cog_uri_with_options(uri, HttpOpenOptions::default())
}

/// Read COG / GeoTIFF metadata with explicit HTTP range-cache options.
pub fn read_cog_uri_with_options(
    uri: &str,
    options: HttpOpenOptions,
) -> Result<CogInfo, RasterError> {
    if genegis_storage::is_remote_uri(uri) {
        read_cog_http_range(uri, options)
    } else {
        read_cog_path(uri)
    }
}

/// Read COG / GeoTIFF metadata from a local file path.
pub fn read_cog_path(path: &str) -> Result<CogInfo, RasterError> {
    let file = GeoTiffFile::open(path).map_err(map_geo_error)?;
    build_info(Some(path.to_string()), &file, Some("local".into()))
}

/// Read COG / GeoTIFF metadata from in-memory bytes.
pub fn read_cog_bytes(bytes: &[u8]) -> Result<CogInfo, RasterError> {
    let file = GeoTiffFile::from_bytes(bytes.to_vec()).map_err(map_geo_error)?;
    build_info(None, &file, Some("bytes".into()))
}

/// Decode a pixel window from a local path or HTTP(S) COG URL.
pub fn read_cog_window_uri(
    uri: &str,
    row_off: u32,
    col_off: u32,
    rows: u32,
    cols: u32,
) -> Result<Vec<u8>, RasterError> {
    if genegis_storage::is_remote_uri(uri) {
        let file = HttpGeoTiffFile::open(uri).map_err(map_geo_error)?;
        read_window_from_file(file.inner(), row_off, col_off, rows, cols)
    } else {
        read_cog_window_u8(uri, row_off, col_off, rows, cols)
    }
}

/// Smoke partial read — decode a pixel window from the base resolution image.
pub fn read_cog_window_u8(
    path: &str,
    row_off: u32,
    col_off: u32,
    rows: u32,
    cols: u32,
) -> Result<Vec<u8>, RasterError> {
    let file = GeoTiffFile::open(path).map_err(map_geo_error)?;
    read_window_from_file(&file, row_off, col_off, rows, cols)
}

fn read_cog_http_range(uri: &str, options: HttpOpenOptions) -> Result<CogInfo, RasterError> {
    let file = HttpGeoTiffFile::open_with_options(uri, options).map_err(map_geo_error)?;
    build_info(
        Some(uri.to_string()),
        file.inner(),
        Some("http_range".into()),
    )
}

fn read_window_from_file(
    file: &GeoTiffFile,
    row_off: u32,
    col_off: u32,
    rows: u32,
    cols: u32,
) -> Result<Vec<u8>, RasterError> {
    let array = file
        .read_window::<u8>(
            row_off as usize,
            col_off as usize,
            rows as usize,
            cols as usize,
        )
        .map_err(map_geo_error)?;
    Ok(array.iter().copied().collect())
}

fn build_info(
    path: Option<String>,
    file: &GeoTiffFile,
    read_mode: Option<String>,
) -> Result<CogInfo, RasterError> {
    let ifd = file
        .tiff()
        .ifd(file.base_ifd_index())
        .map_err(map_geo_error)?;

    let crs = file
        .epsg()
        .map(|code| format!("EPSG:{code}"))
        .unwrap_or_else(|| "unknown".to_string());

    let overview_count = file.overview_count();
    let tiled = ifd.is_tiled();
    let cloud_optimized = tiled && overview_count > 0;

    Ok(CogInfo {
        path,
        width: file.width(),
        height: file.height(),
        band_count: file.band_count(),
        epsg: file.epsg(),
        crs,
        geo_bounds: file.geo_bounds(),
        tiled,
        tile_width: ifd.tile_width(),
        tile_height: ifd.tile_height(),
        overview_count,
        cloud_optimized,
        read_mode,
    })
}

fn map_geo_error(err: impl ToString) -> RasterError {
    RasterError::Cog(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use geotiff_writer::{CogBuilder, GeoTiffBuilder};
    use ndarray::Array2;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    fn write_smoke_cog(path: &std::path::Path) {
        let width = 64u32;
        let height = 48u32;
        let mut data = Array2::<u8>::zeros((height as usize, width as usize));
        for y in 0..height as usize {
            for x in 0..width as usize {
                data[[y, x]] = ((x + y) % 256) as u8;
            }
        }

        let builder = GeoTiffBuilder::new(width, height)
            .epsg(4326)
            .pixel_scale(0.01, 0.01)
            .origin(-180.0, 90.0);

        CogBuilder::new(builder)
            .no_overviews()
            .write_2d(path, data.view())
            .expect("write cog");
    }

    fn write_large_smoke_cog(path: &std::path::Path) {
        let width = 512u32;
        let height = 512u32;
        let mut data = Array2::<u8>::zeros((height as usize, width as usize));
        for y in 0..height as usize {
            for x in 0..width as usize {
                data[[y, x]] = ((x + y) % 256) as u8;
            }
        }

        let builder = GeoTiffBuilder::new(width, height)
            .epsg(4326)
            .pixel_scale(0.01, 0.01)
            .origin(-180.0, 90.0);

        CogBuilder::new(builder)
            .no_overviews()
            .write_2d(path, data.view())
            .expect("write cog");
    }

    struct HttpCogFixture {
        url: String,
        body_len: usize,
        bytes_sent: Arc<AtomicUsize>,
        range_requests: Arc<AtomicUsize>,
    }

    impl HttpCogFixture {
        fn spawn(body: Vec<u8>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            listener.set_nonblocking(true).expect("nonblocking");
            let addr = listener.local_addr().expect("addr");
            let bytes_sent = Arc::new(AtomicUsize::new(0));
            let range_requests = Arc::new(AtomicUsize::new(0));
            let sent_thread = Arc::clone(&bytes_sent);
            let range_thread = Arc::clone(&range_requests);
            let payload = Arc::new(body.clone());
            let body_len = body.len();

            thread::spawn(move || {
                let deadline = std::time::Instant::now() + Duration::from_secs(10);
                while std::time::Instant::now() < deadline {
                    let Ok((mut stream, _)) = listener.accept() else {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    };
                    handle_request(
                        &mut stream,
                        &payload,
                        &sent_thread,
                        &range_thread,
                    );
                }
            });

            Self {
                url: format!("http://{addr}/cog.tif"),
                body_len,
                bytes_sent,
                range_requests,
            }
        }
    }

    fn handle_request(
        stream: &mut TcpStream,
        body: &[u8],
        bytes_sent: &AtomicUsize,
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
            bytes_sent.fetch_add(slice.len(), Ordering::SeqCst);
            let response = format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Type: image/tiff\r\nAccept-Ranges: bytes\r\nContent-Range: bytes {start}-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                end.min(body.len() - 1),
                body.len(),
                slice.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.write_all(slice);
            return;
        }

        bytes_sent.fetch_add(body.len(), Ordering::SeqCst);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: image/tiff\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
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
    fn reads_cog_metadata_from_path() {
        let temp = tempfile::NamedTempFile::new().expect("temp");
        write_smoke_cog(temp.path());

        let info = read_cog_path(temp.path().to_str().expect("path")).expect("read");
        assert_eq!(info.width, 64);
        assert_eq!(info.height, 48);
        assert_eq!(info.epsg, Some(4326));
        assert!(info.tiled);
        assert_eq!(info.read_mode.as_deref(), Some("local"));
        assert!(info.geo_bounds.is_some());
    }

    #[test]
    fn reads_cog_window_smoke() {
        let temp = tempfile::NamedTempFile::new().expect("temp");
        write_smoke_cog(temp.path());
        let path = temp.path().to_str().expect("path");

        let pixels = read_cog_window_u8(path, 0, 0, 8, 8).expect("window");
        assert_eq!(pixels.len(), 64);
    }

    #[test]
    fn reads_cog_metadata_from_bytes() {
        let temp = tempfile::NamedTempFile::new().expect("temp");
        write_smoke_cog(temp.path());
        let bytes = std::fs::read(temp.path()).expect("read bytes");
        let info = read_cog_bytes(&bytes).expect("read bytes");
        assert_eq!(info.band_count, 1);
        assert_eq!(info.crs, "EPSG:4326");
        assert_eq!(info.read_mode.as_deref(), Some("bytes"));
    }

    #[test]
    fn reads_remote_cog_metadata_via_http_ranges() {
        let temp = tempfile::NamedTempFile::new().expect("temp");
        write_large_smoke_cog(temp.path());
        let bytes = std::fs::read(temp.path()).expect("read bytes");
        let fixture = HttpCogFixture::spawn(bytes.clone());
        thread::sleep(Duration::from_millis(50));

        let info = read_cog_uri_with_options(
            &fixture.url,
            HttpOpenOptions {
                chunk_size: 16 * 1024,
                cache_bytes: 1024 * 1024,
                cache_slots: 16,
                ..HttpOpenOptions::default()
            },
        )
        .expect("read remote cog");

        assert_eq!(info.read_mode.as_deref(), Some("http_range"));
        assert_eq!(info.width, 512);
        assert_eq!(info.height, 512);
        assert_eq!(info.epsg, Some(4326));
        assert!(fixture.range_requests.load(Ordering::SeqCst) >= 1);
        assert!(fixture.bytes_sent.load(Ordering::SeqCst) < fixture.body_len);
    }

    #[test]
    fn reads_remote_cog_window_via_http_ranges() {
        let temp = tempfile::NamedTempFile::new().expect("temp");
        write_smoke_cog(temp.path());
        let bytes = std::fs::read(temp.path()).expect("read bytes");
        let fixture = HttpCogFixture::spawn(bytes);
        thread::sleep(Duration::from_millis(50));

        let pixels = read_cog_window_uri(&fixture.url, 0, 0, 8, 8).expect("window");
        assert_eq!(pixels.len(), 64);
        assert!(fixture.range_requests.load(Ordering::SeqCst) >= 1);
    }
}
