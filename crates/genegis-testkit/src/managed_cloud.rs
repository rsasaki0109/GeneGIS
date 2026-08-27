//! Sealed HTTP Range evidence for managed-cloud performance matrices.

use std::time::Instant;

use genegis_core::{DeploymentClass, PerformanceMatrixProfile};
use genegis_storage::{
    fetch_http_range_with_policy, probe_http_object_metadata_with_policy, remote_url_host,
    ByteRange, RemoteAccessPolicy,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// One independently verifiable HTTP 206 observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedCloudRangeRequest {
    /// Inclusive first byte.
    pub start: u64,
    /// Inclusive final byte.
    pub end: u64,
    /// HTTP response status.
    pub status: u16,
    /// Bytes returned by the server.
    pub response_bytes: u64,
    /// Digest of the exact response body.
    pub response_digest: String,
    /// Raw Content-Range header.
    pub content_range: String,
    /// Request wall time.
    pub elapsed_ns: u64,
}

/// Artifact-bound evidence that a declared managed deployment supports bounded byte ranges.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedCloudRangeReceipt {
    /// Receipt schema version.
    pub schema_version: String,
    /// Performance profile identity.
    pub profile_id: String,
    /// Logical dataset identity from the performance profile.
    pub dataset_digest: String,
    /// Cargo lock/build identity from the performance profile.
    pub build_digest: String,
    /// UTC observation time.
    pub observed_at: String,
    /// Runtime operating-system identity.
    pub os: String,
    /// Runtime CPU identity.
    pub cpu: String,
    /// Credential-free benchmark object URL.
    pub source_url: String,
    /// Source host admitted by the explicit remote-access allowlist.
    pub source_host: String,
    /// Digest of URL, size, ETag, and Last-Modified identity.
    pub source_identity_digest: String,
    /// Encoded object size.
    pub content_length: u64,
    /// Server entity tag when supplied.
    pub etag: Option<String>,
    /// Server modification observation when supplied.
    pub last_modified: Option<String>,
    /// Whether HEAD advertised byte ranges.
    pub accepts_byte_ranges: bool,
    /// Exact bounded range requests.
    pub requests: Vec<ManagedCloudRangeRequest>,
    /// True if any ranged request fell back to a whole-object response.
    pub whole_object_fallback: bool,
    /// Digest of all preceding semantic fields.
    pub receipt_digest: String,
}

/// Collect four bounded HTTP ranges under the repository's fail-closed host policy.
pub fn collect_managed_cloud_range_receipt(
    profile: &PerformanceMatrixProfile,
    source_url: &str,
    observed_at: String,
    os: String,
    cpu: String,
) -> Result<ManagedCloudRangeReceipt, String> {
    if profile.deployment_class != DeploymentClass::ManagedCloud {
        return Err("managed-cloud range evidence requires a managed-cloud profile".into());
    }
    let source_host = remote_url_host(source_url).map_err(|error| error.to_string())?;
    let policy = RemoteAccessPolicy::from_env();
    policy
        .validate_url(source_url)
        .map_err(|error| error.to_string())?;
    let metadata = probe_http_object_metadata_with_policy(source_url, &policy)
        .map_err(|error| error.to_string())?;
    if !metadata.accepts_byte_ranges || metadata.content_length < 4 {
        return Err("managed-cloud object does not advertise usable byte ranges".into());
    }
    if metadata.etag.is_none() && metadata.last_modified.is_none() {
        return Err("managed-cloud object has no ETag or Last-Modified identity".into());
    }
    let range_len = metadata.content_length.min(64 * 1024).max(1);
    let maximum_start = metadata.content_length - range_len;
    let starts = [
        0,
        maximum_start / 3,
        maximum_start.saturating_mul(2) / 3,
        maximum_start,
    ];
    let mut requests = Vec::with_capacity(starts.len());
    let mut whole_object_fallback = false;
    for start in starts {
        let end = start + range_len - 1;
        let range = ByteRange::new(start, end).map_err(|error| error.to_string())?;
        let started = Instant::now();
        let response = fetch_http_range_with_policy(source_url, &range, &policy)
            .map_err(|error| error.to_string())?;
        let elapsed_ns = started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
        whole_object_fallback |= response.status != 206;
        requests.push(ManagedCloudRangeRequest {
            start,
            end,
            status: response.status,
            response_bytes: response.bytes.len() as u64,
            response_digest: format!("sha256:{:x}", Sha256::digest(&response.bytes)),
            content_range: response.content_range.unwrap_or_default(),
            elapsed_ns,
        });
    }
    let source_identity_digest = source_identity_digest(
        source_url,
        metadata.content_length,
        metadata.etag.as_deref(),
        metadata.last_modified.as_deref(),
    )?;
    let mut receipt = ManagedCloudRangeReceipt {
        schema_version: "1.0.0".into(),
        profile_id: profile.id.clone(),
        dataset_digest: profile.dataset_digest.clone(),
        build_digest: profile.build_digest.clone(),
        observed_at,
        os,
        cpu,
        source_url: source_url.into(),
        source_host,
        source_identity_digest,
        content_length: metadata.content_length,
        etag: metadata.etag,
        last_modified: metadata.last_modified,
        accepts_byte_ranges: metadata.accepts_byte_ranges,
        requests,
        whole_object_fallback,
        receipt_digest: String::new(),
    };
    receipt.receipt_digest = receipt_digest(&receipt)?;
    verify_managed_cloud_range_receipt(&receipt)?;
    Ok(receipt)
}

/// Recompute source, request, and receipt claims from persisted JSON.
pub fn verify_managed_cloud_range_receipt(
    receipt: &ManagedCloudRangeReceipt,
) -> Result<(), String> {
    let observed_host = remote_url_host(&receipt.source_url).map_err(|error| error.to_string())?;
    if receipt.schema_version != "1.0.0"
        || receipt.profile_id.trim().is_empty()
        || !valid_digest(&receipt.dataset_digest)
        || !valid_digest(&receipt.build_digest)
        || !valid_digest(&receipt.source_identity_digest)
        || !valid_digest(&receipt.receipt_digest)
        || receipt.observed_at.trim().is_empty()
        || receipt.os.trim().is_empty()
        || receipt.cpu.trim().is_empty()
        || !observed_host.eq_ignore_ascii_case(&receipt.source_host)
        || receipt.content_length == 0
        || (receipt.etag.is_none() && receipt.last_modified.is_none())
        || !receipt.accepts_byte_ranges
        || receipt.whole_object_fallback
        || receipt.requests.is_empty()
        || receipt.requests.len() > 64
    {
        return Err("managed-cloud range receipt identity is invalid".into());
    }
    let expected_source_identity = source_identity_digest(
        &receipt.source_url,
        receipt.content_length,
        receipt.etag.as_deref(),
        receipt.last_modified.as_deref(),
    )?;
    if receipt.source_identity_digest != expected_source_identity {
        return Err("managed-cloud source identity digest mismatch".into());
    }
    for request in &receipt.requests {
        let expected_content_range = format!(
            "bytes {}-{}/{}",
            request.start, request.end, receipt.content_length
        );
        if request.end < request.start
            || request.end >= receipt.content_length
            || request.status != 206
            || request.response_bytes != request.end - request.start + 1
            || !valid_digest(&request.response_digest)
            || request.content_range != expected_content_range
            || request.elapsed_ns == 0
        {
            return Err("managed-cloud range request evidence is invalid".into());
        }
    }
    let expected_receipt_digest = receipt_digest(receipt)?;
    if receipt.receipt_digest != expected_receipt_digest {
        return Err("managed-cloud range receipt digest mismatch".into());
    }
    Ok(())
}

fn source_identity_digest(
    source_url: &str,
    content_length: u64,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Result<String, String> {
    digest(&(source_url, content_length, etag, last_modified))
}

fn receipt_digest(receipt: &ManagedCloudRangeReceipt) -> Result<String, String> {
    let mut semantic = receipt.clone();
    semantic.receipt_digest.clear();
    digest(&semantic)
}

fn digest<T: Serialize>(value: &T) -> Result<String, String> {
    let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn valid_digest(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipt() -> ManagedCloudRangeReceipt {
        let mut receipt = ManagedCloudRangeReceipt {
            schema_version: "1.0.0".into(),
            profile_id: "managed-cloud-nagoya".into(),
            dataset_digest: format!("sha256:{}", "a".repeat(64)),
            build_digest: format!("sha256:{}", "b".repeat(64)),
            observed_at: "2026-08-27T00:00:00Z".into(),
            os: "test-os".into(),
            cpu: "test-cpu".into(),
            source_url: "https://object.example.test/nagoya.bin".into(),
            source_host: "object.example.test".into(),
            source_identity_digest: String::new(),
            content_length: 1024,
            etag: Some("fixture-v1".into()),
            last_modified: None,
            accepts_byte_ranges: true,
            requests: vec![ManagedCloudRangeRequest {
                start: 0,
                end: 63,
                status: 206,
                response_bytes: 64,
                response_digest: format!("sha256:{}", "c".repeat(64)),
                content_range: "bytes 0-63/1024".into(),
                elapsed_ns: 1,
            }],
            whole_object_fallback: false,
            receipt_digest: String::new(),
        };
        receipt.source_identity_digest = source_identity_digest(
            &receipt.source_url,
            receipt.content_length,
            receipt.etag.as_deref(),
            receipt.last_modified.as_deref(),
        )
        .expect("source digest");
        receipt.receipt_digest = receipt_digest(&receipt).expect("receipt digest");
        receipt
    }

    #[test]
    fn verifies_sealed_range_evidence_and_rejects_fallback() {
        let receipt = receipt();
        verify_managed_cloud_range_receipt(&receipt).expect("valid receipt");
        let persisted = serde_json::to_vec(&receipt).expect("serialize");
        let roundtrip: ManagedCloudRangeReceipt =
            serde_json::from_slice(&persisted).expect("deserialize");
        verify_managed_cloud_range_receipt(&roundtrip).expect("roundtrip");

        let mut fallback = receipt;
        fallback.whole_object_fallback = true;
        fallback.receipt_digest = receipt_digest(&fallback).expect("re-seal mutation");
        assert!(verify_managed_cloud_range_receipt(&fallback).is_err());
    }
}
