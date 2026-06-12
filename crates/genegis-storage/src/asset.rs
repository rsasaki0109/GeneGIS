use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use crate::error::StorageError;
use crate::http::{fetch_http_bytes, fetch_http_range};
use crate::range::ByteRange;

/// Recommended prefix size for COG / GeoTIFF header probes over HTTP.
pub const COG_HEADER_PREFIX_BYTES: u64 = 65_536;

/// Returns true when the URI uses an HTTP(S) scheme.
pub fn is_remote_uri(uri: &str) -> bool {
    uri.starts_with("http://") || uri.starts_with("https://")
}

/// Read a local file in full.
pub fn read_local_bytes(path: &str) -> Result<Vec<u8>, StorageError> {
    std::fs::read(path).map_err(|err| StorageError::Local(err.to_string()))
}

/// Read an inclusive byte range from a local file.
pub fn read_local_range(path: &str, range: &ByteRange) -> Result<Vec<u8>, StorageError> {
    let mut file = File::open(path).map_err(|err| StorageError::Local(err.to_string()))?;
    file.seek(SeekFrom::Start(range.start))
        .map_err(|err| StorageError::Local(err.to_string()))?;

    let mut buffer = vec![0u8; range.len() as usize];
    file.read_exact(&mut buffer)
        .map_err(|err| StorageError::Local(err.to_string()))?;
    Ok(buffer)
}

/// Read a catalog asset in full (local path or HTTP URL).
pub fn read_asset_bytes(uri: &str) -> Result<Vec<u8>, StorageError> {
    if is_remote_uri(uri) {
        Ok(fetch_http_bytes(uri)?.bytes)
    } else {
        read_local_bytes(uri)
    }
}

/// Read a byte range from a catalog asset (local path or HTTP URL).
pub fn read_asset_range(uri: &str, range: &ByteRange) -> Result<Vec<u8>, StorageError> {
    if is_remote_uri(uri) {
        Ok(fetch_http_range(uri, range)?.bytes)
    } else {
        read_local_range(uri, range)
    }
}

/// Fetch a catalog asset over HTTP or local IO, returning HTTP metadata when remote.
pub fn fetch_asset(uri: &str, range: Option<&ByteRange>) -> Result<AssetFetchResult, StorageError> {
    if is_remote_uri(uri) {
        let http = match range {
            Some(range) => fetch_http_range(uri, range)?,
            None => fetch_http_bytes(uri)?,
        };
        Ok(AssetFetchResult {
            uri: uri.to_string(),
            range: range.copied(),
            byte_len: http.bytes.len(),
            status: Some(http.status),
            content_range: http.content_range,
            bytes: http.bytes,
        })
    } else {
        let bytes = match range {
            Some(range) => read_local_range(uri, range)?,
            None => read_local_bytes(uri)?,
        };
        Ok(AssetFetchResult {
            uri: uri.to_string(),
            range: range.copied(),
            byte_len: bytes.len(),
            status: None,
            content_range: None,
            bytes,
        })
    }
}

/// Unified fetch result for CLI and diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetFetchResult {
    /// Source URI or path.
    pub uri: String,
    /// Requested byte range, if any.
    pub range: Option<ByteRange>,
    /// Number of bytes returned.
    pub byte_len: usize,
    /// HTTP status when fetched remotely.
    pub status: Option<u16>,
    /// HTTP `Content-Range` when fetched remotely with a range.
    pub content_range: Option<String>,
    /// Response body bytes.
    pub bytes: Vec<u8>,
}

impl AssetFetchResult {
    /// Serialize a diagnostic summary (omits raw bytes).
    pub fn summary_json(&self) -> serde_json::Value {
        serde_json::json!({
            "uri": self.uri,
            "range": self.range.map(|range| range.header_value()),
            "byte_len": self.byte_len,
            "status": self.status,
            "content_range": self.content_range,
        })
    }
}
