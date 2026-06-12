use ureq::Error as UreqError;

use crate::error::StorageError;
use crate::range::ByteRange;

/// Result of an HTTP GET (full or ranged).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpFetchResult {
    /// HTTP status code (200 or 206).
    pub status: u16,
    /// Response body bytes.
    pub bytes: Vec<u8>,
    /// Raw `Content-Range` header when present.
    pub content_range: Option<String>,
}

/// Download the full resource body with a plain GET.
pub fn fetch_http_bytes(url: &str) -> Result<HttpFetchResult, StorageError> {
    let mut response = ureq::get(url)
        .call()
        .map_err(map_transport_error)?;

    let status = response.status().as_u16();
    if status != 200 {
        let detail = response.body_mut().read_to_string().unwrap_or_default();
        return Err(StorageError::Http(format!("HTTP {status}: {detail}")));
    }

    let bytes = response
        .body_mut()
        .read_to_vec()
        .map_err(map_transport_error)?;

    Ok(HttpFetchResult {
        status,
        bytes,
        content_range: None,
    })
}

/// Download a byte range using the HTTP `Range` header.
pub fn fetch_http_range(url: &str, range: &ByteRange) -> Result<HttpFetchResult, StorageError> {
    let mut response = ureq::get(url)
        .header("Range", &range.header_value())
        .call()
        .map_err(map_transport_error)?;

    let status = response.status().as_u16();
    if status != 206 && status != 200 {
        let detail = response.body_mut().read_to_string().unwrap_or_default();
        return Err(StorageError::Http(format!("HTTP {status}: {detail}")));
    }

    let content_range = response
        .headers()
        .get("Content-Range")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    let bytes = response
        .body_mut()
        .read_to_vec()
        .map_err(map_transport_error)?;

    if status == 206 && bytes.len() as u64 != range.len() {
        return Err(StorageError::Http(format!(
            "range response length mismatch: expected {}, got {}",
            range.len(),
            bytes.len()
        )));
    }

    Ok(HttpFetchResult {
        status,
        bytes,
        content_range,
    })
}

/// Parse the total object size from an HTTP `Content-Range` header (`bytes a-b/total`).
pub fn parse_content_range_total(content_range: &str) -> Option<u64> {
    let (_, total) = content_range.split_once('/')?;
    total.trim().parse().ok()
}

/// Probe remote object size via `Content-Length` (HEAD) or `Content-Range` (`bytes=0-0`).
pub fn probe_http_content_length(url: &str) -> Result<u64, StorageError> {
    let head = ureq::head(url).call().map_err(map_transport_error)?;
    if head.status().as_u16() == 200 {
        if let Some(len) = head
            .headers()
            .get("Content-Length")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
        {
            return Ok(len);
        }
    }

    let response = ureq::get(url)
        .header("Range", "bytes=0-0")
        .call()
        .map_err(map_transport_error)?;

    let status = response.status().as_u16();
    if status != 206 {
        return Err(StorageError::Http(format!(
            "server does not support HTTP range probes for {url}: status {status}"
        )));
    }

    let content_range = response
        .headers()
        .get("Content-Range")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| StorageError::Http(format!("missing Content-Range for {url}")))?;

    parse_content_range_total(content_range).ok_or_else(|| {
        StorageError::Http(format!(
            "unable to parse Content-Range total from {content_range:?}"
        ))
    })
}

fn map_transport_error(err: UreqError) -> StorageError {
    StorageError::Http(err.to_string())
}
