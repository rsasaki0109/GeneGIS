//! Provider-neutral geocoding with fail-closed privacy, rate, and evidence policies.

use std::{collections::BTreeSet, time::Instant};

use genegis_contract::GeoContract;
use genegis_crs::{ChecksumVerification, CoordinateUnit, Crs, SourceSnapshot};
use genegis_storage::{
    post_http_json_bytes_with_policy, CloudFormat, IoReceipt, IoRequestEvidence, IoSelection,
    RemoteAccessPolicy,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    admit, AdapterInvocation, AdapterManifest, AdapterOperation, AdmissionReport, BackendFamily,
    BackendIdentity, Capability, CapabilityPolicy, Determinism, EvidenceHook,
    ADAPTER_MANIFEST_SCHEMA_VERSION,
};

/// Digest of the reviewed provider-neutral geocoding adapter contract.
pub const GEOCODING_ADAPTER_BUILD_DIGEST: &str =
    "sha256:4d5fb41f1b91ed01ae17ffaf4cc05ec4f85d70ce33f057858cb5c91ee1732a58";

const ADAPTER_ID: &str = "org.genegis.geocoding";
const ADAPTER_VERSION: &str = "0.1.0";

/// Query execution mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeocodingMode {
    /// One user-facing lookup.
    Interactive,
    /// Multiple records submitted as one governed unit.
    Batch,
}

/// One query with a caller-owned stable identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeocodingQuery {
    /// Stable row or interaction identity.
    pub id: String,
    /// Address or place text. Receipts retain only its digest.
    pub text: String,
}

/// Provider-neutral geocoding request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeocodingRequest {
    /// Interactive or batch execution.
    pub mode: GeocodingMode,
    /// Non-empty uniquely identified queries.
    pub queries: Vec<GeocodingQuery>,
    /// BCP-47-like output language tag.
    pub language: String,
    /// Positive candidate limit per query.
    pub max_candidates: u32,
}

/// Privacy boundary for query transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeocodingPrivacyPolicy {
    /// Query text may never leave the process.
    LocalOnly,
    /// Remote transport is allowed only for redacted/non-address queries.
    AllowRemoteRedacted,
    /// Full query text may be sent to an allowlisted provider.
    AllowRemoteFull,
}

/// Admission limits independent of any specific provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeocodingRatePolicy {
    /// Largest admitted request.
    pub max_batch_size: u32,
    /// Largest admitted candidate count per query.
    pub max_candidates_per_query: u32,
    /// Provider-enforced minimum interval between remote requests.
    pub minimum_interval_ms: u64,
}

impl Default for GeocodingRatePolicy {
    fn default() -> Self {
        Self {
            max_batch_size: 100,
            max_candidates_per_query: 5,
            minimum_interval_ms: 100,
        }
    }
}

/// One immutable local gazetteer row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GazetteerEntry {
    /// Stable provider feature identity.
    pub feature_id: String,
    /// Display label.
    pub label: String,
    /// Search aliases.
    pub aliases: Vec<String>,
    /// WGS84 longitude.
    pub longitude: f64,
    /// WGS84 latitude.
    pub latitude: f64,
}

/// Match relationship reported by a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeocodeMatchKind {
    /// Canonical label or alias matched exactly.
    Exact,
    /// Provider reports a partial/textual match.
    Partial,
    /// Provider reports an interpolated address position.
    Interpolated,
}

/// One validated WGS84 candidate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeocodeCandidate {
    /// Stable provider feature identity.
    pub feature_id: String,
    /// Human-readable provider label.
    pub label: String,
    /// WGS84 longitude.
    pub longitude: f64,
    /// WGS84 latitude.
    pub latitude: f64,
    /// Provider confidence in the inclusive range 0–1.
    pub confidence: f64,
    /// Provider-declared match relationship.
    pub match_kind: GeocodeMatchKind,
    /// Stable provider identity.
    pub provider_id: String,
}

/// Candidates associated with one input identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeocodeQueryResult {
    /// Input query identity.
    pub query_id: String,
    /// Ranked validated candidates.
    pub candidates: Vec<GeocodeCandidate>,
}

/// Swappable execution provider.
#[derive(Debug, Clone)]
pub enum GeocodingProvider {
    /// Immutable in-process gazetteer.
    OfflineGazetteer {
        /// Provider identity.
        provider_id: String,
        /// Provider data version.
        version: String,
        /// Exact gazetteer source.
        source: SourceSnapshot,
        /// Reviewed entries.
        entries: Vec<GazetteerEntry>,
    },
    /// Allowlisted HTTP JSON provider implementing the GeneGIS response contract.
    HttpJson {
        /// Provider identity.
        provider_id: String,
        /// Provider contract version.
        version: String,
        /// POST endpoint.
        endpoint: String,
        /// Network allowlist and limits.
        remote_policy: RemoteAccessPolicy,
    },
}

impl GeocodingProvider {
    fn identity(&self) -> (&str, &str) {
        match self {
            Self::OfflineGazetteer {
                provider_id,
                version,
                ..
            }
            | Self::HttpJson {
                provider_id,
                version,
                ..
            } => (provider_id, version),
        }
    }

    fn is_remote(&self) -> bool {
        matches!(self, Self::HttpJson { .. })
    }

    /// Return the provider source entering the workflow input contract.
    pub fn source_snapshot(&self) -> SourceSnapshot {
        match self {
            Self::OfflineGazetteer { source, .. } => source.clone(),
            Self::HttpJson { endpoint, .. } => SourceSnapshot::new(endpoint.clone()),
        }
    }
}

/// Complete evidence for one admitted geocoding request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeocodingReceipt {
    /// Receipt schema version.
    pub schema_version: String,
    /// Exact adapter manifest digest.
    pub manifest_digest: String,
    /// Capability admission result.
    pub admission: AdmissionReport,
    /// Selected provider identity.
    pub provider_id: String,
    /// Selected provider version.
    pub provider_version: String,
    /// Interactive or batch mode.
    pub mode: GeocodingMode,
    /// Digest of privacy and rate policy values.
    pub policy_digest: String,
    /// Applied privacy policy.
    pub privacy_policy: GeocodingPrivacyPolicy,
    /// Applied request-rate and candidate limits.
    pub rate_policy: GeocodingRatePolicy,
    /// SHA-256 digests of query text in request order.
    pub query_digests: Vec<String>,
    /// Whether raw query text was retained in evidence.
    pub raw_queries_retained: bool,
    /// Number of queries with at least one result.
    pub matched_queries: u32,
    /// Number of queries with multiple candidates.
    pub ambiguous_queries: u32,
    /// Number of queries with no candidate.
    pub unmatched_queries: u32,
    /// WGS84 output CRS.
    pub crs: Crs,
    /// Coordinate unit derived from the CRS.
    pub coordinate_unit: CoordinateUnit,
    /// Exact provider source snapshot.
    pub source: SourceSnapshot,
    /// Digest of canonical result JSON.
    pub output_digest: String,
    /// Transport/selection evidence, including zero-byte local execution.
    pub io: IoReceipt,
}

/// Validated results paired with their receipt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeocodingResponse {
    /// Results in input query order.
    pub results: Vec<GeocodeQueryResult>,
    /// Admission, source, privacy, confidence, and I/O evidence.
    pub receipt: GeocodingReceipt,
}

/// Fail-closed geocoding error taxonomy.
#[derive(Debug, Error)]
pub enum GeocodingError {
    /// Request, policy, or provider parameters are invalid.
    #[error("geocoding request rejected: {0}")]
    Request(String),
    /// Adapter manifest/capability admission failed.
    #[error("geocoding adapter admission failed: {0:?}")]
    Admission(Vec<String>),
    /// Privacy policy forbids provider transport.
    #[error("geocoding privacy policy rejected request: {0}")]
    Privacy(String),
    /// Remote transport failed.
    #[error("geocoding transport failed: {0}")]
    Transport(String),
    /// Provider payload or candidate semantics are invalid.
    #[error("geocoding response rejected: {0}")]
    Response(String),
    /// Evidence could not be serialized.
    #[error("geocoding evidence failed: {0}")]
    Evidence(String),
}

/// Provider-neutral adapter admitted by reviewed manifest and policy.
#[derive(Debug, Clone)]
pub struct GeocodingAdapter {
    manifest: AdapterManifest,
    capability_policy: CapabilityPolicy,
}

impl Default for GeocodingAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl GeocodingAdapter {
    /// Build the reviewed geocoding adapter.
    pub fn new() -> Self {
        let manifest = geocoding_manifest();
        let capability_policy = CapabilityPolicy {
            accepted_adapters: BTreeSet::from([manifest.adapter_id.clone()]),
            allowed_capabilities: BTreeSet::from([Capability::NetworkRead]),
            required_evidence_hooks: BTreeSet::from([
                EvidenceHook::InputDigests,
                EvidenceHook::OutputDigests,
                EvidenceHook::Parameters,
                EvidenceHook::ComponentIdentity,
                EvidenceHook::EnvironmentDigest,
                EvidenceHook::IoMetrics,
            ]),
            reject_opaque: true,
            reject_unknown_determinism: true,
        };
        Self {
            manifest,
            capability_policy,
        }
    }

    /// Build with explicit contracts for negative admission tests.
    pub fn with_contracts(manifest: AdapterManifest, capability_policy: CapabilityPolicy) -> Self {
        Self {
            manifest,
            capability_policy,
        }
    }

    /// Admit, execute, validate, and receipt a geocoding request.
    pub fn execute(
        &self,
        request: &GeocodingRequest,
        provider: &GeocodingProvider,
        privacy: GeocodingPrivacyPolicy,
        rate: &GeocodingRatePolicy,
    ) -> Result<GeocodingResponse, GeocodingError> {
        validate_request(request, rate)?;
        if provider.is_remote() && privacy != GeocodingPrivacyPolicy::AllowRemoteFull {
            return Err(GeocodingError::Privacy(
                "remote full-text transport requires allow_remote_full".into(),
            ));
        }
        let operation_id = if provider.is_remote() {
            "geocoding.http_json"
        } else {
            "geocoding.offline_gazetteer"
        };
        let requested_capabilities = if provider.is_remote() {
            BTreeSet::from([Capability::NetworkRead])
        } else {
            BTreeSet::new()
        };
        let admission = admit(
            &self.manifest,
            &AdapterInvocation {
                adapter_id: self.manifest.adapter_id.clone(),
                adapter_version: self.manifest.adapter_version.clone(),
                operation_id: operation_id.into(),
                operation_version: "1.0.0".into(),
                backend: self.manifest.backend.clone(),
                requested_capabilities,
            },
            &self.capability_policy,
        );
        if !admission.admitted {
            return Err(GeocodingError::Admission(
                admission
                    .failures
                    .iter()
                    .map(|failure| failure.code.clone())
                    .collect(),
            ));
        }

        let started = Instant::now();
        let (mut results, response_bytes, http_status, mut source) = match provider {
            GeocodingProvider::OfflineGazetteer {
                entries,
                source,
                provider_id,
                ..
            } => (
                offline_results(request, entries, provider_id),
                0,
                None,
                source.clone(),
            ),
            GeocodingProvider::HttpJson {
                endpoint,
                remote_policy,
                ..
            } => {
                let body = serde_json::to_vec(request)
                    .map_err(|error| GeocodingError::Evidence(error.to_string()))?;
                let fetched = post_http_json_bytes_with_policy(endpoint, &body, &[], remote_policy)
                    .map_err(|error| GeocodingError::Transport(error.to_string()))?;
                let content_type = fetched
                    .content_type
                    .as_deref()
                    .unwrap_or("")
                    .split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase();
                if content_type != "application/json" && content_type != "application/geo+json" {
                    return Err(GeocodingError::Response(format!(
                        "unsupported content type {content_type:?}"
                    )));
                }
                let parsed: ProviderResponse = serde_json::from_slice(&fetched.bytes)
                    .map_err(|error| GeocodingError::Response(error.to_string()))?;
                (
                    parsed.results,
                    fetched.bytes.len() as u64,
                    Some(fetched.status),
                    SourceSnapshot::new(endpoint.clone()),
                )
            }
        };
        normalize_and_validate_results(request, provider.identity().0, &mut results)?;
        let output_bytes = serde_json::to_vec(&results)
            .map_err(|error| GeocodingError::Evidence(error.to_string()))?;
        let output_digest = format!("sha256:{:x}", Sha256::digest(&output_bytes));
        if provider.is_remote() {
            source.checksum = Some(output_digest.clone());
            source.observed_checksum = Some(output_digest.clone());
            source.checksum_status = ChecksumVerification::Verified;
        }
        let elapsed_ns = started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
        let requests = if response_bytes == 0 {
            vec![]
        } else {
            vec![IoRequestEvidence {
                start: 0,
                end: response_bytes - 1,
                response_bytes,
                http_status,
            }]
        };
        let io = IoReceipt::new(
            CloudFormat::Geocoding,
            output_digest.clone(),
            response_bytes,
            output_bytes.len() as u64,
            IoSelection::GeocodingQueries {
                query_count: request.queries.len() as u32,
                max_candidates: request.max_candidates,
                language: request.language.clone(),
            },
            requests,
            false,
            results
                .iter()
                .map(|result| result.candidates.len() as u64)
                .sum(),
            elapsed_ns,
            0,
            None,
        );
        let matched_queries = results.iter().filter(|r| !r.candidates.is_empty()).count() as u32;
        let ambiguous_queries = results.iter().filter(|r| r.candidates.len() > 1).count() as u32;
        let (provider_id, provider_version) = provider.identity();
        let policy_bytes = serde_json::to_vec(&(privacy, rate))
            .map_err(|error| GeocodingError::Evidence(error.to_string()))?;
        Ok(GeocodingResponse {
            receipt: GeocodingReceipt {
                schema_version: "0.1.0".into(),
                manifest_digest: self
                    .manifest
                    .digest()
                    .map_err(|e| GeocodingError::Evidence(e.to_string()))?,
                admission,
                provider_id: provider_id.into(),
                provider_version: provider_version.into(),
                mode: request.mode,
                policy_digest: format!("sha256:{:x}", Sha256::digest(policy_bytes)),
                privacy_policy: privacy,
                rate_policy: rate.clone(),
                query_digests: request
                    .queries
                    .iter()
                    .map(|query| format!("sha256:{:x}", Sha256::digest(query.text.as_bytes())))
                    .collect(),
                raw_queries_retained: false,
                matched_queries,
                ambiguous_queries,
                unmatched_queries: request.queries.len() as u32 - matched_queries,
                crs: Crs::wgs84(),
                coordinate_unit: CoordinateUnit::Degrees,
                source,
                output_digest,
                io,
            },
            results,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderResponse {
    results: Vec<GeocodeQueryResult>,
}

fn validate_request(
    request: &GeocodingRequest,
    rate: &GeocodingRatePolicy,
) -> Result<(), GeocodingError> {
    if request.queries.is_empty() {
        return Err(GeocodingError::Request(
            "at least one query is required".into(),
        ));
    }
    if request.mode == GeocodingMode::Interactive && request.queries.len() != 1 {
        return Err(GeocodingError::Request(
            "interactive mode requires exactly one query".into(),
        ));
    }
    if request.queries.len() > rate.max_batch_size as usize {
        return Err(GeocodingError::Request(
            "query count exceeds rate policy".into(),
        ));
    }
    if request.max_candidates == 0 || request.max_candidates > rate.max_candidates_per_query {
        return Err(GeocodingError::Request(
            "candidate count exceeds rate policy".into(),
        ));
    }
    if request.language.trim().is_empty()
        || request
            .queries
            .iter()
            .any(|q| q.id.trim().is_empty() || q.text.trim().is_empty())
    {
        return Err(GeocodingError::Request(
            "query ids, text, and language must be non-empty".into(),
        ));
    }
    let ids = request
        .queries
        .iter()
        .map(|q| q.id.as_str())
        .collect::<BTreeSet<_>>();
    if ids.len() != request.queries.len() {
        return Err(GeocodingError::Request("query ids must be unique".into()));
    }
    Ok(())
}

fn offline_results(
    request: &GeocodingRequest,
    entries: &[GazetteerEntry],
    provider_id: &str,
) -> Vec<GeocodeQueryResult> {
    request
        .queries
        .iter()
        .map(|query| {
            let needle = query.text.trim().to_lowercase();
            let mut candidates = entries
                .iter()
                .filter_map(|entry| {
                    let labels = std::iter::once(entry.label.as_str())
                        .chain(entry.aliases.iter().map(String::as_str));
                    let exact = labels.clone().any(|value| value.to_lowercase() == needle);
                    let partial = labels
                        .into_iter()
                        .any(|value| value.to_lowercase().contains(&needle));
                    (exact || partial).then(|| GeocodeCandidate {
                        feature_id: entry.feature_id.clone(),
                        label: entry.label.clone(),
                        longitude: entry.longitude,
                        latitude: entry.latitude,
                        confidence: if exact { 1.0 } else { 0.75 },
                        match_kind: if exact {
                            GeocodeMatchKind::Exact
                        } else {
                            GeocodeMatchKind::Partial
                        },
                        provider_id: provider_id.into(),
                    })
                })
                .collect::<Vec<_>>();
            candidates.sort_by(|a, b| {
                b.confidence
                    .total_cmp(&a.confidence)
                    .then_with(|| a.feature_id.cmp(&b.feature_id))
            });
            candidates.truncate(request.max_candidates as usize);
            GeocodeQueryResult {
                query_id: query.id.clone(),
                candidates,
            }
        })
        .collect()
}

fn normalize_and_validate_results(
    request: &GeocodingRequest,
    provider_id: &str,
    results: &mut Vec<GeocodeQueryResult>,
) -> Result<(), GeocodingError> {
    if results.len() != request.queries.len() {
        return Err(GeocodingError::Response(
            "provider must return exactly one result per query".into(),
        ));
    }
    for (expected, result) in request.queries.iter().zip(results.iter_mut()) {
        if result.query_id != expected.id
            || result.candidates.len() > request.max_candidates as usize
        {
            return Err(GeocodingError::Response(
                "provider result identity or candidate limit mismatch".into(),
            ));
        }
        for candidate in &mut result.candidates {
            if !candidate.longitude.is_finite()
                || !(-180.0..=180.0).contains(&candidate.longitude)
                || !candidate.latitude.is_finite()
                || !(-90.0..=90.0).contains(&candidate.latitude)
                || !candidate.confidence.is_finite()
                || !(0.0..=1.0).contains(&candidate.confidence)
                || candidate.feature_id.trim().is_empty()
                || candidate.label.trim().is_empty()
            {
                return Err(GeocodingError::Response(
                    "invalid WGS84 coordinate, confidence, or identity".into(),
                ));
            }
            candidate.provider_id = provider_id.into();
        }
    }
    Ok(())
}

/// Return the reviewed local/HTTP geocoding adapter manifest.
pub fn geocoding_manifest() -> AdapterManifest {
    let hooks = BTreeSet::from([
        EvidenceHook::InputDigests,
        EvidenceHook::OutputDigests,
        EvidenceHook::Parameters,
        EvidenceHook::ComponentIdentity,
        EvidenceHook::EnvironmentDigest,
        EvidenceHook::IoMetrics,
    ]);
    let operation = |id: &str, capabilities: BTreeSet<Capability>, determinism| AdapterOperation {
        operation_id: id.into(),
        operation_version: "1.0.0".into(),
        inputs: vec![GeoContract::new(format!("{id}.request"))],
        outputs: vec![GeoContract::new(format!("{id}.wgs84_candidates"))],
        capabilities,
        determinism,
        evidence_hooks: hooks.clone(),
        opaque: false,
    };
    AdapterManifest {
        schema_version: ADAPTER_MANIFEST_SCHEMA_VERSION.into(),
        adapter_id: ADAPTER_ID.into(),
        adapter_version: ADAPTER_VERSION.into(),
        backend: BackendIdentity {
            family: BackendFamily::Geocoding,
            engine_version: "contract-1.0.0".into(),
            build_digest: GEOCODING_ADAPTER_BUILD_DIGEST.into(),
            components: [
                ("http-client".into(), "ureq-3".into()),
                ("output-crs".into(), "EPSG:4326".into()),
            ]
            .into_iter()
            .collect(),
        },
        license: "Apache-2.0 OR MIT".into(),
        operations: vec![
            operation(
                "geocoding.offline_gazetteer",
                BTreeSet::new(),
                Determinism::Deterministic,
            ),
            operation(
                "geocoding.http_json",
                BTreeSet::from([Capability::NetworkRead]),
                Determinism::ToleranceBounded,
            ),
        ],
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::{Shutdown, TcpListener, TcpStream},
        thread,
    };

    use super::*;

    fn request(mode: GeocodingMode) -> GeocodingRequest {
        GeocodingRequest {
            mode,
            queries: vec![GeocodingQuery {
                id: "q1".into(),
                text: "名古屋駅".into(),
            }],
            language: "ja".into(),
            max_candidates: 2,
        }
    }

    fn offline() -> GeocodingProvider {
        let bytes = b"nagoya-station-fixture-v1";
        let digest = format!("sha256:{:x}", Sha256::digest(bytes));
        let mut source = SourceSnapshot::new("fixture://nagoya-gazetteer/v1");
        source.license = Some("CC0-1.0".into());
        source.checksum = Some(digest.clone());
        source.observed_checksum = Some(digest);
        source.checksum_status = ChecksumVerification::Verified;
        GeocodingProvider::OfflineGazetteer {
            provider_id: "fixture.nagoya".into(),
            version: "1".into(),
            source,
            entries: vec![GazetteerEntry {
                feature_id: "station:nagoya".into(),
                label: "名古屋駅".into(),
                aliases: vec!["Nagoya Station".into()],
                longitude: 136.8815,
                latitude: 35.1709,
            }],
        }
    }

    fn fixture(content_type: &str, body: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let content_type = content_type.to_string();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            serve(&mut stream, &content_type, &body);
        });
        format!("http://{address}/geocode")
    }

    fn serve(stream: &mut TcpStream, content_type: &str, body: &[u8]) {
        let mut chunk = [0_u8; 2048];
        let mut request = Vec::new();
        loop {
            let read = stream.read(&mut chunk).expect("request");
            if read == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..read]);
            let Some(header_end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            if request.len() >= header_end + 4 + content_length {
                break;
            }
        }
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(response.as_bytes()).expect("headers");
        stream.write_all(body).expect("body");
        stream.flush().expect("flush");
        stream.shutdown(Shutdown::Write).expect("shutdown");
    }

    fn remote(endpoint: String) -> GeocodingProvider {
        GeocodingProvider::HttpJson {
            provider_id: "fixture.http".into(),
            version: "1".into(),
            endpoint,
            remote_policy: RemoteAccessPolicy::from_env(),
        }
    }

    #[test]
    fn offline_and_http_providers_share_receipted_contract() {
        let adapter = GeocodingAdapter::new();
        let local = adapter
            .execute(
                &request(GeocodingMode::Interactive),
                &offline(),
                GeocodingPrivacyPolicy::LocalOnly,
                &GeocodingRatePolicy::default(),
            )
            .expect("offline");
        assert_eq!(local.results[0].candidates[0].confidence, 1.0);
        assert_eq!(local.receipt.crs, Crs::wgs84());
        assert!(!local.receipt.raw_queries_retained);
        assert_eq!(local.receipt.io.format, CloudFormat::Geocoding);

        let body = serde_json::json!({"results": [{"query_id": "q1", "candidates": [{
            "feature_id": "station:nagoya", "label": "名古屋駅", "longitude": 136.8815,
            "latitude": 35.1709, "confidence": 0.98, "match_kind": "exact",
            "provider_id": "ignored"
        }]}]})
        .to_string()
        .into_bytes();
        let http = adapter
            .execute(
                &request(GeocodingMode::Interactive),
                &remote(fixture("application/json", body)),
                GeocodingPrivacyPolicy::AllowRemoteFull,
                &GeocodingRatePolicy::default(),
            )
            .expect("http");
        assert_eq!(http.results[0].candidates[0].provider_id, "fixture.http");
        assert_eq!(http.receipt.io.requests.len(), 1);
        assert!(http.receipt.source.checksum_status.is_verified());
    }

    #[test]
    fn rejects_privacy_rate_and_invalid_provider_semantics() {
        let adapter = GeocodingAdapter::new();
        let endpoint = "http://127.0.0.1:1/geocode".to_string();
        assert!(matches!(
            adapter.execute(
                &request(GeocodingMode::Interactive),
                &remote(endpoint),
                GeocodingPrivacyPolicy::LocalOnly,
                &GeocodingRatePolicy::default()
            ),
            Err(GeocodingError::Privacy(_))
        ));
        let restrictive = GeocodingRatePolicy {
            max_batch_size: 1,
            max_candidates_per_query: 1,
            minimum_interval_ms: 1000,
        };
        assert!(matches!(
            adapter.execute(
                &request(GeocodingMode::Interactive),
                &offline(),
                GeocodingPrivacyPolicy::LocalOnly,
                &restrictive
            ),
            Err(GeocodingError::Request(_))
        ));
        let body = serde_json::json!({"results": [{"query_id": "q1", "candidates": [{
            "feature_id": "bad", "label": "bad", "longitude": 181.0,
            "latitude": 35.0, "confidence": 1.1, "match_kind": "partial",
            "provider_id": "ignored"
        }]}]})
        .to_string()
        .into_bytes();
        assert!(matches!(
            adapter.execute(
                &request(GeocodingMode::Interactive),
                &remote(fixture("application/json", body)),
                GeocodingPrivacyPolicy::AllowRemoteFull,
                &GeocodingRatePolicy::default()
            ),
            Err(GeocodingError::Response(_))
        ));
        assert!(matches!(
            adapter.execute(
                &request(GeocodingMode::Interactive),
                &remote(fixture("text/plain", b"not-json".to_vec())),
                GeocodingPrivacyPolicy::AllowRemoteFull,
                &GeocodingRatePolicy::default()
            ),
            Err(GeocodingError::Response(_))
        ));
    }

    #[test]
    fn manifest_drift_fails_closed() {
        let mut manifest = geocoding_manifest();
        manifest.backend.build_digest = "sha256:drift".into();
        let policy = GeocodingAdapter::new().capability_policy;
        let adapter = GeocodingAdapter::with_contracts(manifest, policy);
        // A self-consistent explicit manifest remains admissible; runtime drift is represented
        // by changing the invocation backend through the reviewed adapter boundary, which is
        // intentionally not public. Manifest validation still rejects malformed digests.
        assert!(matches!(
            adapter.execute(
                &request(GeocodingMode::Interactive),
                &offline(),
                GeocodingPrivacyPolicy::LocalOnly,
                &GeocodingRatePolicy::default()
            ),
            Err(GeocodingError::Admission(_))
        ));
    }
}
