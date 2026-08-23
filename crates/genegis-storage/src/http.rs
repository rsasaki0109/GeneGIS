use std::time::Duration;

use ureq::Error as UreqError;

use crate::error::StorageError;
use crate::policy::RemoteAccessPolicy;
use crate::range::ByteRange;

/// Stable HTTP metadata used to identify and budget a remote range object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpObjectMetadata {
    /// Encoded object length from `Content-Length`.
    pub content_length: u64,
    /// Server entity tag, when supplied.
    pub etag: Option<String>,
    /// Server publication/modification observation, when supplied.
    pub last_modified: Option<String>,
    /// Whether the server explicitly advertises byte ranges.
    pub accepts_byte_ranges: bool,
}

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
    fetch_http_bytes_with_policy(url, &RemoteAccessPolicy::default())
}

/// Download a full resource under an explicit remote access policy.
pub fn fetch_http_bytes_with_policy(
    url: &str,
    policy: &RemoteAccessPolicy,
) -> Result<HttpFetchResult, StorageError> {
    policy.validate_url(url)?;
    let agent = policy_agent(policy);
    let mut response = agent.get(url).call().map_err(map_transport_error)?;

    let status = response.status().as_u16();
    if status != 200 {
        let detail = response
            .body_mut()
            .with_config()
            .limit(policy.max_response_bytes)
            .read_to_string()
            .unwrap_or_default();
        return Err(StorageError::Http(format!("HTTP {status}: {detail}")));
    }

    let bytes = response
        .body_mut()
        .with_config()
        .limit(policy.max_response_bytes)
        .read_to_vec()
        .map_err(map_transport_error)?;

    Ok(HttpFetchResult {
        status,
        bytes,
        content_range: None,
    })
}

/// POST a JSON request and return the response body.
pub fn post_http_json_bytes(url: &str, body: &[u8]) -> Result<HttpFetchResult, StorageError> {
    post_http_json_bytes_with_headers(url, body, &[])
}

/// POST JSON with caller-provided headers. Header values are never persisted by storage.
pub fn post_http_json_bytes_with_headers(
    url: &str,
    body: &[u8],
    headers: &[(String, String)],
) -> Result<HttpFetchResult, StorageError> {
    post_http_json_bytes_with_policy(url, body, headers, &RemoteAccessPolicy::default())
}

/// POST JSON under an explicit remote access policy.
pub fn post_http_json_bytes_with_policy(
    url: &str,
    body: &[u8],
    headers: &[(String, String)],
    policy: &RemoteAccessPolicy,
) -> Result<HttpFetchResult, StorageError> {
    policy.validate_url(url)?;
    if body.len() as u64 > policy.max_response_bytes {
        return Err(StorageError::Http(format!(
            "request body exceeds configured limit of {} bytes",
            policy.max_response_bytes
        )));
    }
    let agent = policy_agent(policy);
    let mut request = agent
        .post(url)
        .header("Content-Type", "application/json")
        .header("Content-Length", &body.len().to_string());
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let mut response = request.send(body).map_err(map_transport_error)?;

    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        let detail = response
            .body_mut()
            .with_config()
            .limit(policy.max_response_bytes)
            .read_to_string()
            .unwrap_or_default();
        return Err(StorageError::Http(format!("HTTP {status}: {detail}")));
    }

    let bytes = response
        .body_mut()
        .with_config()
        .limit(policy.max_response_bytes)
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
    fetch_http_range_with_policy(url, range, &RemoteAccessPolicy::default())
}

/// Download a byte range under an explicit remote access policy.
pub fn fetch_http_range_with_policy(
    url: &str,
    range: &ByteRange,
    policy: &RemoteAccessPolicy,
) -> Result<HttpFetchResult, StorageError> {
    policy.validate_url(url)?;
    if range.len() > policy.max_response_bytes {
        return Err(StorageError::Http(format!(
            "requested range exceeds configured limit of {} bytes",
            policy.max_response_bytes
        )));
    }
    let agent = policy_agent(policy);
    let mut response = agent
        .get(url)
        .header("Range", &range.header_value())
        .call()
        .map_err(map_transport_error)?;

    let status = response.status().as_u16();
    if status != 206 && status != 200 {
        let detail = response
            .body_mut()
            .with_config()
            .limit(policy.max_response_bytes)
            .read_to_string()
            .unwrap_or_default();
        return Err(StorageError::Http(format!("HTTP {status}: {detail}")));
    }

    let content_range = response
        .headers()
        .get("Content-Range")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    let bytes = response
        .body_mut()
        .with_config()
        .limit(policy.max_response_bytes)
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
    probe_http_content_length_with_policy(url, &RemoteAccessPolicy::default())
}

/// Probe object size under an explicit remote access policy.
pub fn probe_http_content_length_with_policy(
    url: &str,
    policy: &RemoteAccessPolicy,
) -> Result<u64, StorageError> {
    policy.validate_url(url)?;
    let agent = policy_agent(policy);
    if let Ok(head) = agent.head(url).call() {
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
    }

    let response = agent
        .get(url)
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

/// Probe object length and immutable-version headers with a HEAD request.
pub fn probe_http_object_metadata(url: &str) -> Result<HttpObjectMetadata, StorageError> {
    probe_http_object_metadata_with_policy(url, &RemoteAccessPolicy::default())
}

/// Probe object metadata under an explicit remote access policy.
pub fn probe_http_object_metadata_with_policy(
    url: &str,
    policy: &RemoteAccessPolicy,
) -> Result<HttpObjectMetadata, StorageError> {
    policy.validate_url(url)?;
    let response = policy_agent(policy)
        .head(url)
        .call()
        .map_err(map_transport_error)?;
    if response.status().as_u16() != 200 {
        return Err(StorageError::Http(format!(
            "HTTP {} while probing {url}",
            response.status().as_u16()
        )));
    }
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    };
    let content_length = header("Content-Length")
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| StorageError::Http(format!("missing Content-Length for {url}")))?;
    let accepts_byte_ranges =
        header("Accept-Ranges").is_some_and(|value| value.eq_ignore_ascii_case("bytes"));
    Ok(HttpObjectMetadata {
        content_length,
        etag: header("ETag"),
        last_modified: header("Last-Modified"),
        accepts_byte_ranges,
    })
}

fn policy_agent(policy: &RemoteAccessPolicy) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_millis(policy.timeout_ms)))
        .max_redirects(policy.max_redirects)
        .max_redirects_will_error(true)
        .build()
        .into()
}

fn map_transport_error(err: UreqError) -> StorageError {
    StorageError::Http(err.to_string())
}
