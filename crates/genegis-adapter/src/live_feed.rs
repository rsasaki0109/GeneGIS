//! Cursor/watermark live spatial feeds with freshness and immutable snapshots.

use std::{collections::BTreeSet, time::Instant};

use chrono::{DateTime, Utc};
use genegis_contract::GeoContract;
use genegis_crs::{ChecksumVerification, Crs, SourceSnapshot};
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

/// Digest of the reviewed live-feed adapter contract.
pub const LIVE_FEED_ADAPTER_BUILD_DIGEST: &str =
    "sha256:30f3b84ce623e454876103d83b2e73e5f12953d7ef40dbfe79bb43ec09f7fe11";

const ADAPTER_ID: &str = "org.genegis.live-feed";
const ADAPTER_VERSION: &str = "0.1.0";

/// Supported live spatial feed family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedDomain {
    /// Meteorological observations and forecasts.
    Weather,
    /// Hazard observations, warnings, and footprints.
    Hazard,
    /// Aggregated movement, traffic, and transit observations.
    Mobility,
    /// Fixed or moving environmental/IoT sensor readings.
    Sensor,
    /// Detected changes derived from imagery or point clouds.
    Change,
}

impl FeedDomain {
    fn name(self) -> &'static str {
        match self {
            Self::Weather => "weather",
            Self::Hazard => "hazard",
            Self::Mobility => "mobility",
            Self::Sensor => "sensor",
            Self::Change => "change",
        }
    }
}

/// Caller-selected freshness, lateness, page, and retention policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedFreshnessPolicy {
    /// Maximum age of the newest returned observation.
    pub max_age_seconds: u64,
    /// Maximum accepted event-time lateness behind the provider watermark.
    pub allowed_lateness_seconds: u64,
    /// Maximum observations in one page.
    pub max_page_size: u32,
    /// Maximum immutable observation snapshots retained per execution.
    pub max_retained_observations: u32,
    /// Reject an empty or stale response rather than merely marking it stale.
    pub reject_stale: bool,
}

impl Default for FeedFreshnessPolicy {
    fn default() -> Self {
        Self {
            max_age_seconds: 900,
            allowed_lateness_seconds: 300,
            max_page_size: 1000,
            max_retained_observations: 1000,
            reject_stale: true,
        }
    }
}

/// One cursor-bounded feed request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveFeedRequest {
    /// Feed family.
    pub domain: FeedDomain,
    /// Allowlisted HTTP JSON endpoint.
    pub endpoint: String,
    /// Stable provider identity.
    pub provider_id: String,
    /// Stable provider contract/data version.
    pub provider_version: String,
    /// Exclusive provider sequence cursor.
    pub after_cursor: u64,
    /// Last committed event-time watermark in RFC 3339 form.
    pub watermark: String,
    /// Inclusive page limit.
    pub limit: u32,
    /// Explicit evaluation time used for deterministic freshness decisions.
    pub evaluated_at: String,
}

/// One provider observation before immutable snapshot sealing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedObservation {
    /// Stable provider observation identity.
    pub id: String,
    /// Strictly increasing provider sequence.
    pub sequence: u64,
    /// Observation/event time in RFC 3339 form.
    pub observed_at: String,
    /// Geometry CRS.
    pub crs: Crs,
    /// GeoJSON geometry object.
    pub geometry: serde_json::Value,
    /// Domain values with explicit unit-bearing keys or values.
    pub values: serde_json::Value,
    /// Provider source revision for this observation.
    pub source_revision: String,
}

/// Content-addressed immutable observation retained by GeneGIS.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedObservationSnapshot {
    /// Exact observation.
    pub observation: FeedObservation,
    /// Canonical SHA-256 identity of the observation.
    pub snapshot_digest: String,
}

/// Evidence for one admitted live-feed page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveFeedReceipt {
    /// Receipt schema version.
    pub schema_version: String,
    /// Exact adapter manifest digest.
    pub manifest_digest: String,
    /// Capability admission report.
    pub admission: AdmissionReport,
    /// Feed family.
    pub domain: FeedDomain,
    /// Provider identity.
    pub provider_id: String,
    /// Provider version.
    pub provider_version: String,
    /// Requested exclusive cursor.
    pub requested_cursor: u64,
    /// Provider next cursor committed by this page.
    pub next_cursor: u64,
    /// Previous committed watermark.
    pub requested_watermark: String,
    /// Provider watermark committed by this page.
    pub next_watermark: String,
    /// Deterministic freshness evaluation time.
    pub evaluated_at: String,
    /// Applied freshness/retention policy.
    pub freshness_policy: FeedFreshnessPolicy,
    /// Whether the newest observation met the freshness policy.
    pub fresh: bool,
    /// Number of accepted late observations.
    pub late_observations: u32,
    /// Exact response source snapshot.
    pub source: SourceSnapshot,
    /// Canonical immutable snapshot-set digest.
    pub output_digest: String,
    /// Shared request/selection/timing evidence.
    pub io: IoReceipt,
}

/// Immutable observations and their live-feed receipt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveFeedResponse {
    /// Content-addressed observations in sequence order.
    pub snapshots: Vec<FeedObservationSnapshot>,
    /// Cursor, watermark, freshness, source, and I/O evidence.
    pub receipt: LiveFeedReceipt,
}

/// Fail-closed live-feed error taxonomy.
#[derive(Debug, Error)]
pub enum LiveFeedError {
    /// Request or policy is invalid.
    #[error("live-feed request rejected: {0}")]
    Request(String),
    /// Adapter capability admission failed.
    #[error("live-feed adapter admission failed: {0:?}")]
    Admission(Vec<String>),
    /// Network policy or transport failed.
    #[error("live-feed transport failed: {0}")]
    Transport(String),
    /// Provider payload violates cursor, watermark, spatial, or freshness contracts.
    #[error("live-feed response rejected: {0}")]
    Response(String),
    /// Evidence serialization failed.
    #[error("live-feed evidence failed: {0}")]
    Evidence(String),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderPage {
    next_cursor: u64,
    watermark: String,
    observations: Vec<FeedObservation>,
}

/// Reviewed HTTP JSON live-feed adapter.
#[derive(Debug, Clone)]
pub struct LiveFeedAdapter {
    manifest: AdapterManifest,
    capability_policy: CapabilityPolicy,
    remote_policy: RemoteAccessPolicy,
}

impl LiveFeedAdapter {
    /// Build the reviewed adapter under a caller-selected network allowlist.
    pub fn new(remote_policy: RemoteAccessPolicy) -> Self {
        let manifest = live_feed_manifest();
        Self {
            capability_policy: CapabilityPolicy::read_only_network(&manifest.adapter_id),
            manifest,
            remote_policy,
        }
    }

    /// Build with explicit contracts for negative admission tests.
    pub fn with_contracts(
        manifest: AdapterManifest,
        capability_policy: CapabilityPolicy,
        remote_policy: RemoteAccessPolicy,
    ) -> Self {
        Self {
            manifest,
            capability_policy,
            remote_policy,
        }
    }

    /// Admit, fetch, validate, snapshot, and receipt one feed page.
    pub fn execute(
        &self,
        request: &LiveFeedRequest,
        policy: &FeedFreshnessPolicy,
    ) -> Result<LiveFeedResponse, LiveFeedError> {
        let requested_watermark = timestamp(&request.watermark, "request watermark")?;
        let evaluated_at = timestamp(&request.evaluated_at, "evaluation time")?;
        if request.provider_id.trim().is_empty()
            || request.provider_version.trim().is_empty()
            || request.limit == 0
            || request.limit > policy.max_page_size
            || request.limit > policy.max_retained_observations
            || requested_watermark > evaluated_at
        {
            return Err(LiveFeedError::Request(
                "provider identity, page limit, retention, or request time is invalid".into(),
            ));
        }
        let admission = admit(
            &self.manifest,
            &AdapterInvocation {
                adapter_id: self.manifest.adapter_id.clone(),
                adapter_version: self.manifest.adapter_version.clone(),
                operation_id: format!("live_feed.{}.read", request.domain.name()),
                operation_version: "1.0.0".into(),
                backend: self.manifest.backend.clone(),
                requested_capabilities: BTreeSet::from([Capability::NetworkRead]),
            },
            &self.capability_policy,
        );
        if !admission.admitted {
            return Err(LiveFeedError::Admission(
                admission
                    .failures
                    .iter()
                    .map(|failure| failure.code.clone())
                    .collect(),
            ));
        }

        let started = Instant::now();
        let request_bytes = serde_json::to_vec(request)
            .map_err(|error| LiveFeedError::Evidence(error.to_string()))?;
        let fetched = post_http_json_bytes_with_policy(
            &request.endpoint,
            &request_bytes,
            &[],
            &self.remote_policy,
        )
        .map_err(|error| LiveFeedError::Transport(error.to_string()))?;
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
            return Err(LiveFeedError::Response(format!(
                "unsupported content type {content_type:?}"
            )));
        }
        let page: ProviderPage = serde_json::from_slice(&fetched.bytes)
            .map_err(|error| LiveFeedError::Response(error.to_string()))?;
        let next_watermark = timestamp(&page.watermark, "provider watermark")?;
        if page.next_cursor < request.after_cursor
            || next_watermark < requested_watermark
            || next_watermark > evaluated_at
            || page.observations.len() > request.limit as usize
        {
            return Err(LiveFeedError::Response(
                "cursor, watermark, or page limit regressed".into(),
            ));
        }

        let mut previous_sequence = request.after_cursor;
        let mut latest: Option<DateTime<Utc>> = None;
        let mut late_observations = 0_u32;
        let mut snapshots = Vec::with_capacity(page.observations.len());
        for observation in page.observations {
            let observed_at = timestamp(&observation.observed_at, "observation time")?;
            let lateness = next_watermark
                .signed_duration_since(observed_at)
                .num_seconds()
                .max(0) as u64;
            if observation.id.trim().is_empty()
                || observation.source_revision.trim().is_empty()
                || observation.sequence <= previous_sequence
                || observation.sequence > page.next_cursor
                || observed_at > evaluated_at
                || !valid_geometry(&observation.geometry)
                || !observation.values.is_object()
            {
                return Err(LiveFeedError::Response(
                    "observation identity, sequence, time, geometry, or values are invalid".into(),
                ));
            }
            if lateness > policy.allowed_lateness_seconds {
                return Err(LiveFeedError::Response(format!(
                    "observation {} exceeds allowed lateness",
                    observation.id
                )));
            }
            if lateness > 0 {
                late_observations += 1;
            }
            previous_sequence = observation.sequence;
            latest = Some(latest.map_or(observed_at, |value| value.max(observed_at)));
            let bytes = serde_json::to_vec(&observation)
                .map_err(|error| LiveFeedError::Evidence(error.to_string()))?;
            snapshots.push(FeedObservationSnapshot {
                observation,
                snapshot_digest: format!("sha256:{:x}", Sha256::digest(bytes)),
            });
        }
        if !snapshots.is_empty() && previous_sequence != page.next_cursor {
            return Err(LiveFeedError::Response(
                "next cursor does not identify the final observation".into(),
            ));
        }
        let fresh = latest.is_some_and(|latest| {
            evaluated_at
                .signed_duration_since(latest)
                .num_seconds()
                .max(0) as u64
                <= policy.max_age_seconds
        });
        if policy.reject_stale && !fresh {
            return Err(LiveFeedError::Response(
                "page is empty or newest observation is stale".into(),
            ));
        }

        let snapshot_bytes = serde_json::to_vec(&snapshots)
            .map_err(|error| LiveFeedError::Evidence(error.to_string()))?;
        let output_digest = format!("sha256:{:x}", Sha256::digest(snapshot_bytes));
        let response_digest = format!("sha256:{:x}", Sha256::digest(&fetched.bytes));
        let mut source = SourceSnapshot::new(request.endpoint.clone());
        source.dataset_id = Some(format!("{}:{}", request.provider_id, request.domain.name()));
        source.source_version = Some(genegis_crs::SourceVersion::new(
            request.provider_version.clone(),
        ));
        source.checksum = Some(response_digest.clone());
        source.observed_checksum = Some(response_digest.clone());
        source.checksum_status = ChecksumVerification::Verified;
        let response_bytes = fetched.bytes.len() as u64;
        let io = IoReceipt::new(
            CloudFormat::LiveFeed,
            response_digest,
            response_bytes,
            response_bytes,
            IoSelection::LiveFeedWindow {
                domain: request.domain.name().into(),
                after_cursor: request.after_cursor,
                limit: request.limit,
                watermark: request.watermark.clone(),
            },
            vec![IoRequestEvidence {
                start: 0,
                end: response_bytes.saturating_sub(1),
                response_bytes,
                http_status: Some(fetched.status),
            }],
            false,
            snapshots.len() as u64,
            started.elapsed().as_nanos().min(u64::MAX as u128) as u64,
            0,
            None,
        );
        Ok(LiveFeedResponse {
            snapshots,
            receipt: LiveFeedReceipt {
                schema_version: "0.1.0".into(),
                manifest_digest: self
                    .manifest
                    .digest()
                    .map_err(|error| LiveFeedError::Evidence(error.to_string()))?,
                admission,
                domain: request.domain,
                provider_id: request.provider_id.clone(),
                provider_version: request.provider_version.clone(),
                requested_cursor: request.after_cursor,
                next_cursor: page.next_cursor,
                requested_watermark: request.watermark.clone(),
                next_watermark: page.watermark,
                evaluated_at: request.evaluated_at.clone(),
                freshness_policy: policy.clone(),
                fresh,
                late_observations,
                source,
                output_digest,
                io,
            },
        })
    }
}

fn timestamp(value: &str, label: &str) -> Result<DateTime<Utc>, LiveFeedError> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| LiveFeedError::Request(format!("invalid {label}")))
}

fn valid_geometry(value: &serde_json::Value) -> bool {
    value
        .as_object()
        .and_then(|object| object.get("type")?.as_str())
        .is_some_and(|kind| {
            matches!(
                kind,
                "Point"
                    | "MultiPoint"
                    | "LineString"
                    | "MultiLineString"
                    | "Polygon"
                    | "MultiPolygon"
            ) && value.get("coordinates").is_some()
        })
}

/// Return the reviewed manifest for all live-feed domains.
pub fn live_feed_manifest() -> AdapterManifest {
    let hooks = BTreeSet::from([
        EvidenceHook::InputDigests,
        EvidenceHook::OutputDigests,
        EvidenceHook::Parameters,
        EvidenceHook::ComponentIdentity,
        EvidenceHook::EnvironmentDigest,
        EvidenceHook::IoMetrics,
    ]);
    let operation = |domain: FeedDomain| AdapterOperation {
        operation_id: format!("live_feed.{}.read", domain.name()),
        operation_version: "1.0.0".into(),
        inputs: vec![GeoContract::new(format!(
            "live_feed.{}.cursor",
            domain.name()
        ))],
        outputs: vec![GeoContract::new(format!(
            "live_feed.{}.observation_snapshots",
            domain.name()
        ))],
        capabilities: BTreeSet::from([Capability::NetworkRead]),
        determinism: Determinism::ToleranceBounded,
        evidence_hooks: hooks.clone(),
        opaque: false,
    };
    AdapterManifest {
        schema_version: ADAPTER_MANIFEST_SCHEMA_VERSION.into(),
        adapter_id: ADAPTER_ID.into(),
        adapter_version: ADAPTER_VERSION.into(),
        backend: BackendIdentity {
            family: BackendFamily::LiveFeed,
            engine_version: "cursor-watermark-contract-1.0.0".into(),
            build_digest: LIVE_FEED_ADAPTER_BUILD_DIGEST.into(),
            components: [("http-client".into(), "ureq-3".into())]
                .into_iter()
                .collect(),
        },
        license: "Apache-2.0 OR MIT".into(),
        operations: [
            FeedDomain::Weather,
            FeedDomain::Hazard,
            FeedDomain::Mobility,
            FeedDomain::Sensor,
            FeedDomain::Change,
        ]
        .into_iter()
        .map(operation)
        .collect(),
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

    fn fixture(body: serde_json::Value) -> String {
        let bytes = serde_json::to_vec(&body).expect("json");
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            read_request(&mut stream);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            );
            stream.write_all(response.as_bytes()).expect("headers");
            stream.write_all(&bytes).expect("body");
            stream.flush().expect("flush");
            stream.shutdown(Shutdown::Write).expect("shutdown");
        });
        format!("http://{address}/feed")
    }

    fn read_request(stream: &mut TcpStream) {
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 2048];
        loop {
            let read = stream.read(&mut chunk).expect("read");
            bytes.extend_from_slice(&chunk[..read]);
            let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&bytes[..end]);
            let length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            if bytes.len() >= end + 4 + length {
                break;
            }
        }
    }

    fn page(sequence: u64, observed_at: &str) -> serde_json::Value {
        serde_json::json!({
            "next_cursor": sequence,
            "watermark": "2026-08-26T09:00:00Z",
            "observations": [{
                "id": format!("observation-{sequence}"),
                "sequence": sequence,
                "observed_at": observed_at,
                "crs": "EPSG:4326",
                "geometry": {"type": "Point", "coordinates": [136.9, 35.18]},
                "values": {"value": 24.5, "unit": "celsius"},
                "source_revision": "fixture-v1"
            }]
        })
    }

    fn request(domain: FeedDomain, endpoint: String) -> LiveFeedRequest {
        LiveFeedRequest {
            domain,
            endpoint,
            provider_id: "fixture.live".into(),
            provider_version: "1".into(),
            after_cursor: 40,
            watermark: "2026-08-26T08:50:00Z".into(),
            limit: 10,
            evaluated_at: "2026-08-26T09:01:00Z".into(),
        }
    }

    #[test]
    fn all_domains_emit_cursor_watermark_freshness_and_immutable_snapshots() {
        for domain in [
            FeedDomain::Weather,
            FeedDomain::Hazard,
            FeedDomain::Mobility,
            FeedDomain::Sensor,
            FeedDomain::Change,
        ] {
            let response = LiveFeedAdapter::new(RemoteAccessPolicy::from_env())
                .execute(
                    &request(domain, fixture(page(41, "2026-08-26T08:59:30Z"))),
                    &FeedFreshnessPolicy::default(),
                )
                .expect("feed");
            assert_eq!(response.receipt.next_cursor, 41);
            assert!(response.receipt.fresh);
            assert!(response.receipt.source.checksum_status.is_verified());
            assert!(response.snapshots[0].snapshot_digest.starts_with("sha256:"));
        }
    }

    #[test]
    fn rejects_cursor_regression_lateness_staleness_and_bad_geometry() {
        let adapter = LiveFeedAdapter::new(RemoteAccessPolicy::from_env());
        let mut regressed = page(39, "2026-08-26T08:59:30Z");
        regressed["observations"] = serde_json::json!([]);
        assert!(adapter
            .execute(
                &request(FeedDomain::Weather, fixture(regressed)),
                &FeedFreshnessPolicy::default()
            )
            .is_err());

        assert!(adapter
            .execute(
                &request(
                    FeedDomain::Hazard,
                    fixture(page(41, "2026-08-26T08:00:00Z"))
                ),
                &FeedFreshnessPolicy::default()
            )
            .is_err());

        let mut policy = FeedFreshnessPolicy::default();
        policy.allowed_lateness_seconds = 7200;
        policy.max_age_seconds = 60;
        assert!(adapter
            .execute(
                &request(
                    FeedDomain::Sensor,
                    fixture(page(41, "2026-08-26T08:00:00Z"))
                ),
                &policy
            )
            .is_err());

        let mut invalid = page(41, "2026-08-26T08:59:30Z");
        invalid["observations"][0]["geometry"] = serde_json::json!({"type": "Point"});
        assert!(adapter
            .execute(
                &request(FeedDomain::Change, fixture(invalid)),
                &FeedFreshnessPolicy::default()
            )
            .is_err());
    }
}
