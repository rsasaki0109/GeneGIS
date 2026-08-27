//! Typed, fail-closed boundaries for external GIS engines.
//!
//! An adapter is admitted only when its manifest, exact backend build,
//! semantic operation, declared capabilities, and required evidence hooks all
//! agree with the selected policy. Unrepresentable semantics remain explicit
//! opaque boundaries and never inherit verified trust silently.

#![deny(missing_docs)]

use genegis_contract::GeoContract;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

mod geocoding;
mod grass;
mod live_feed;
mod ogc;
mod postgis;
mod qgis;

pub use geocoding::{
    geocoding_manifest, GazetteerEntry, GeocodeCandidate, GeocodeMatchKind, GeocodeQueryResult,
    GeocodingAdapter, GeocodingError, GeocodingMode, GeocodingPrivacyPolicy, GeocodingProvider,
    GeocodingQuery, GeocodingRatePolicy, GeocodingReceipt, GeocodingRequest, GeocodingResponse,
    GEOCODING_ADAPTER_BUILD_DIGEST,
};
pub use grass::{
    grass_manifest, GrassAdapter, GrassError, GrassOperation, GrassReceipt, SandboxEvidence,
    GRASS_IMAGE_DIGEST, GRASS_IMAGE_REFERENCE,
};
pub use live_feed::{
    live_feed_manifest, FeedDomain, FeedFreshnessPolicy, FeedObservation, FeedObservationSnapshot,
    LiveFeedAdapter, LiveFeedError, LiveFeedReceipt, LiveFeedRequest, LiveFeedResponse,
    LIVE_FEED_ADAPTER_BUILD_DIGEST,
};
pub use ogc::{
    ogc_web_service_manifest, OgcAdapterError, OgcOperation, OgcRequest, OgcResponse,
    OgcServiceAdapter, OgcServiceReceipt, WfsGetFeatureRequest, WmsGetMapRequest,
    OGC_ADAPTER_BUILD_DIGEST,
};
pub use postgis::{
    postgis_manifest, PostgisAdapter, PostgisError, PostgisOperation, PostgisReceipt,
    POSTGIS_IMAGE_DIGEST, POSTGIS_IMAGE_REFERENCE,
};
pub use qgis::{
    qgis_manifest, QgisAdapter, QgisError, QgisOperation, QgisReceipt, QgisRepairMethod,
    QgisVectorSummary, QGIS_IMAGE_DIGEST, QGIS_IMAGE_REFERENCE,
};

/// Current adapter-manifest schema version.
pub const ADAPTER_MANIFEST_SCHEMA_VERSION: &str = "0.1.0";

/// Stable identifier for the committed JSON Schema.
pub const ADAPTER_MANIFEST_SCHEMA_ID: &str =
    "https://genegis.org/schemas/adapter-manifest/0.1.0/schema.json";

fn default_schema_version() -> String {
    ADAPTER_MANIFEST_SCHEMA_VERSION.into()
}

/// Return the committed Adapter Manifest JSON Schema.
pub fn adapter_manifest_json_schema() -> serde_json::Value {
    serde_json::from_str(include_str!("../schema/adapter-manifest-v0.schema.json"))
        .expect("bundled Adapter Manifest schema must be valid JSON")
}

/// External execution family supported by an adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendFamily {
    /// GDAL/OGR command or library execution.
    Gdal,
    /// DuckDB with its Spatial extension.
    DuckDbSpatial,
    /// PostgreSQL with PostGIS.
    Postgis,
    /// GRASS GIS module execution.
    Grass,
    /// QGIS Processing provider execution.
    QgisProcessing,
    /// Native OGC HTTP service client.
    OgcWebService,
    /// Provider-neutral local or HTTP geocoding engine.
    Geocoding,
    /// Cursor/watermark-based live spatial feed client.
    LiveFeed,
}

/// Exact backend implementation used by a manifest or invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendIdentity {
    /// Backend family.
    pub family: BackendFamily,
    /// Engine or application version.
    pub engine_version: String,
    /// Digest of the binary, container, or reproducible build description.
    pub build_digest: String,
    /// Versioned extensions, providers, drivers, or modules affecting semantics.
    #[serde(default)]
    pub components: BTreeMap<String, String>,
}

/// Side effect or host privilege required by an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Read files selected by the host.
    FileRead,
    /// Write files into a host-approved destination.
    FileWrite,
    /// Fetch from an allowlisted remote endpoint.
    NetworkRead,
    /// Send or publish data to a remote endpoint.
    NetworkWrite,
    /// Execute a read-only database transaction.
    DatabaseRead,
    /// Mutate database state.
    DatabaseWrite,
    /// Spawn a separately sandboxed process.
    ProcessSpawn,
    /// Load or execute native code in the host process.
    NativeCode,
    /// Submit work to a GPU device.
    Gpu,
}

/// Evidence an adapter promises to expose after execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceHook {
    /// Content identities of every admitted input.
    InputDigests,
    /// Content identities of produced values or files.
    OutputDigests,
    /// SQL, module arguments, or normalized operation parameters.
    Parameters,
    /// Query-plan identity, when the engine supplies one.
    QueryPlanDigest,
    /// Backend warnings that may change interpretation.
    Warnings,
    /// Driver, provider, extension, and module versions.
    ComponentIdentity,
    /// Container, environment, or build identity.
    EnvironmentDigest,
    /// Bytes and requests used for cloud-native access.
    IoMetrics,
    /// Database transaction and isolation context.
    TransactionContext,
}

/// Reproducibility characteristics declared by an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Determinism {
    /// Same content inputs and backend build must produce identical canonical output.
    Deterministic,
    /// Deterministic only when the recorded seed is supplied.
    Seeded,
    /// Ordering or floating-point values may vary within an explicit contract tolerance.
    ToleranceBounded,
    /// Determinism is not established.
    Unknown,
}

/// One semantic operation exposed by an adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterOperation {
    /// Stable operation identifier, such as `postgis.query.read_only`.
    pub operation_id: String,
    /// Semantic version of the operation contract.
    pub operation_version: String,
    /// Input port contracts.
    #[serde(default)]
    pub inputs: Vec<GeoContract>,
    /// Output port contracts.
    #[serde(default)]
    pub outputs: Vec<GeoContract>,
    /// Exact host privileges required for execution.
    #[serde(default)]
    pub capabilities: BTreeSet<Capability>,
    /// Reproducibility classification.
    pub determinism: Determinism,
    /// Evidence promised by a successful execution.
    #[serde(default)]
    pub evidence_hooks: BTreeSet<EvidenceHook>,
    /// Whether some backend semantics cannot be represented by GeneGIS.
    #[serde(default)]
    pub opaque: bool,
}

/// Versioned manifest for one external GIS adapter build.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterManifest {
    /// Manifest schema version.
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    /// Stable adapter identifier.
    pub adapter_id: String,
    /// Adapter implementation version.
    pub adapter_version: String,
    /// Exact backend build targeted by this manifest.
    pub backend: BackendIdentity,
    /// License expression for the adapter implementation.
    pub license: String,
    /// Semantic operations exposed by the adapter.
    pub operations: Vec<AdapterOperation>,
}

impl AdapterManifest {
    /// Validate stable identities, operation uniqueness, and all GeoContracts.
    pub fn validate(&self) -> Vec<AdmissionFailure> {
        validate_manifest(self)
    }

    /// Return a deterministic digest covering backend, semantics, and capabilities.
    pub fn digest(&self) -> Result<String, serde_json::Error> {
        let bytes = serde_json::to_vec(self)?;
        Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
    }
}

/// Exact runtime request checked before an adapter may execute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterInvocation {
    /// Adapter identifier selected by the workflow node.
    pub adapter_id: String,
    /// Adapter implementation version loaded by the worker.
    pub adapter_version: String,
    /// Semantic operation requested by the workflow node.
    pub operation_id: String,
    /// Semantic operation version requested by the workflow node.
    pub operation_version: String,
    /// Backend actually discovered at runtime.
    pub backend: BackendIdentity,
    /// Capabilities the runtime is about to exercise.
    pub requested_capabilities: BTreeSet<Capability>,
}

/// Host or organization policy for adapter admission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityPolicy {
    /// Adapter identities explicitly accepted by this policy.
    #[serde(default)]
    pub accepted_adapters: BTreeSet<String>,
    /// Host privileges that may be granted.
    #[serde(default)]
    pub allowed_capabilities: BTreeSet<Capability>,
    /// Evidence hooks required from every admitted operation.
    #[serde(default)]
    pub required_evidence_hooks: BTreeSet<EvidenceHook>,
    /// Reject operations with unrepresented backend semantics.
    #[serde(default)]
    pub reject_opaque: bool,
    /// Require deterministic or explicitly tolerance-bounded execution.
    #[serde(default)]
    pub reject_unknown_determinism: bool,
}

impl CapabilityPolicy {
    /// A conservative policy for analytical database access without mutation.
    pub fn read_only_database(adapter_id: impl Into<String>) -> Self {
        Self {
            accepted_adapters: BTreeSet::from([adapter_id.into()]),
            allowed_capabilities: BTreeSet::from([Capability::DatabaseRead]),
            required_evidence_hooks: BTreeSet::from([
                EvidenceHook::InputDigests,
                EvidenceHook::OutputDigests,
                EvidenceHook::Parameters,
                EvidenceHook::ComponentIdentity,
                EvidenceHook::EnvironmentDigest,
                EvidenceHook::TransactionContext,
            ]),
            reject_opaque: true,
            reject_unknown_determinism: true,
        }
    }

    /// A conservative policy for a network-isolated, ephemeral process adapter.
    pub fn sandboxed_process(adapter_id: impl Into<String>) -> Self {
        Self {
            accepted_adapters: BTreeSet::from([adapter_id.into()]),
            allowed_capabilities: BTreeSet::from([Capability::ProcessSpawn]),
            required_evidence_hooks: BTreeSet::from([
                EvidenceHook::InputDigests,
                EvidenceHook::OutputDigests,
                EvidenceHook::Parameters,
                EvidenceHook::Warnings,
                EvidenceHook::ComponentIdentity,
                EvidenceHook::EnvironmentDigest,
            ]),
            reject_opaque: true,
            reject_unknown_determinism: true,
        }
    }

    /// A sandboxed process policy with explicit read-only input and write-only output mounts.
    pub fn sandboxed_file_process(adapter_id: impl Into<String>) -> Self {
        let mut policy = Self::sandboxed_process(adapter_id);
        policy
            .allowed_capabilities
            .extend([Capability::FileRead, Capability::FileWrite]);
        policy
    }

    /// A conservative policy for allowlisted read-only web-service access.
    pub fn read_only_network(adapter_id: impl Into<String>) -> Self {
        Self {
            accepted_adapters: BTreeSet::from([adapter_id.into()]),
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
        }
    }
}

/// One reason why an adapter invocation cannot execute or inherit trust.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionFailure {
    /// Stable machine-readable failure code.
    pub code: String,
    /// Adapter, operation, backend, capability, contract, or evidence hook.
    pub subject: String,
    /// Human-readable safe remediation context.
    pub detail: String,
}

/// Fail-closed admission result computed before external execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionReport {
    /// Whether the adapter may execute under the selected policy.
    pub admitted: bool,
    /// Whether a successful execution may remain eligible for verified trust.
    pub verification_eligible: bool,
    /// Structured admission failures.
    pub failures: Vec<AdmissionFailure>,
}

/// Validate and admit one exact adapter invocation.
pub fn admit(
    manifest: &AdapterManifest,
    invocation: &AdapterInvocation,
    policy: &CapabilityPolicy,
) -> AdmissionReport {
    let mut failures = validate_manifest(manifest);
    if invocation.adapter_id != manifest.adapter_id
        || invocation.adapter_version != manifest.adapter_version
    {
        fail(
            &mut failures,
            "adapter_identity_mismatch",
            &invocation.adapter_id,
            "runtime adapter identity does not match its reviewed manifest",
        );
    }
    if !policy.accepted_adapters.contains(&manifest.adapter_id) {
        fail(
            &mut failures,
            "adapter_not_accepted",
            &manifest.adapter_id,
            "adapter is not accepted by the selected capability policy",
        );
    }
    if invocation.backend != manifest.backend {
        fail(
            &mut failures,
            "backend_identity_mismatch",
            &format!("{:?}", invocation.backend.family),
            "runtime backend version, build, or components differ from the manifest",
        );
    }

    let operation = manifest
        .operations
        .iter()
        .find(|operation| operation.operation_id == invocation.operation_id);
    let Some(operation) = operation else {
        fail(
            &mut failures,
            "operation_not_declared",
            &invocation.operation_id,
            "runtime operation is not declared by the adapter manifest",
        );
        return admission_report(failures, false);
    };
    if operation.operation_version != invocation.operation_version {
        fail(
            &mut failures,
            "operation_version_mismatch",
            &invocation.operation_id,
            "runtime operation version differs from the reviewed semantic contract",
        );
    }
    if invocation.requested_capabilities != operation.capabilities {
        fail(
            &mut failures,
            "capability_declaration_mismatch",
            &invocation.operation_id,
            "runtime capabilities must exactly match the reviewed operation declaration",
        );
    }
    for capability in &invocation.requested_capabilities {
        if !policy.allowed_capabilities.contains(capability) {
            fail(
                &mut failures,
                "capability_denied",
                &format!("{capability:?}"),
                "capability is denied by the selected policy",
            );
        }
    }
    for hook in &policy.required_evidence_hooks {
        if !operation.evidence_hooks.contains(hook) {
            fail(
                &mut failures,
                "required_evidence_hook_missing",
                &format!("{hook:?}"),
                "adapter operation cannot emit evidence required by policy",
            );
        }
    }
    if policy.reject_opaque && operation.opaque {
        fail(
            &mut failures,
            "opaque_operation_denied",
            &operation.operation_id,
            "operation has unrepresented semantics and policy rejects opaque execution",
        );
    }
    if policy.reject_unknown_determinism && operation.determinism == Determinism::Unknown {
        fail(
            &mut failures,
            "unknown_determinism_denied",
            &operation.operation_id,
            "operation determinism is unknown",
        );
    }
    let verification_eligible = !operation.opaque && operation.determinism != Determinism::Unknown;
    admission_report(failures, verification_eligible)
}

fn validate_manifest(manifest: &AdapterManifest) -> Vec<AdmissionFailure> {
    let mut failures = Vec::new();
    for (code, subject, value) in [
        (
            "unsupported_manifest_schema",
            "manifest",
            manifest.schema_version.as_str(),
        ),
        (
            "missing_adapter_id",
            "manifest",
            manifest.adapter_id.as_str(),
        ),
        (
            "missing_adapter_version",
            manifest.adapter_id.as_str(),
            manifest.adapter_version.as_str(),
        ),
        (
            "missing_backend_version",
            manifest.adapter_id.as_str(),
            manifest.backend.engine_version.as_str(),
        ),
        (
            "invalid_backend_digest",
            manifest.adapter_id.as_str(),
            manifest.backend.build_digest.as_str(),
        ),
        (
            "missing_license",
            manifest.adapter_id.as_str(),
            manifest.license.as_str(),
        ),
    ] {
        let valid = match code {
            "unsupported_manifest_schema" => value == ADAPTER_MANIFEST_SCHEMA_VERSION,
            "invalid_backend_digest" => valid_digest(value),
            _ => !value.trim().is_empty(),
        };
        if !valid {
            fail(
                &mut failures,
                code,
                subject,
                "required adapter-manifest identity is missing or unsupported",
            );
        }
    }
    if manifest.operations.is_empty() {
        fail(
            &mut failures,
            "missing_operations",
            &manifest.adapter_id,
            "adapter manifest declares no semantic operations",
        );
    }
    let mut operation_ids = BTreeSet::new();
    for operation in &manifest.operations {
        if operation.operation_id.trim().is_empty()
            || operation.operation_version.trim().is_empty()
            || !operation_ids.insert(&operation.operation_id)
        {
            fail(
                &mut failures,
                "invalid_operation_identity",
                &operation.operation_id,
                "operation identifier/version is empty or duplicated",
            );
        }
        if operation.inputs.is_empty() || operation.outputs.is_empty() {
            fail(
                &mut failures,
                "missing_operation_contract",
                &operation.operation_id,
                "operation must declare at least one input and output GeoContract",
            );
        }
        for contract in operation.inputs.iter().chain(&operation.outputs) {
            if let Err(error) = contract.validate() {
                fail(
                    &mut failures,
                    "invalid_operation_contract",
                    &contract.id,
                    &error.to_string(),
                );
            }
        }
    }
    failures
}

fn admission_report(
    failures: Vec<AdmissionFailure>,
    verification_eligible: bool,
) -> AdmissionReport {
    AdmissionReport {
        admitted: failures.is_empty(),
        verification_eligible: failures.is_empty() && verification_eligible,
        failures,
    }
}

fn fail(failures: &mut Vec<AdmissionFailure>, code: &str, subject: &str, detail: &str) {
    failures.push(AdmissionFailure {
        code: code.into(),
        subject: subject.into(),
        detail: detail.into(),
    });
}

fn valid_digest(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn contract(id: &str) -> GeoContract {
        GeoContract::new(id)
    }

    fn manifest(capabilities: BTreeSet<Capability>) -> AdapterManifest {
        AdapterManifest {
            schema_version: ADAPTER_MANIFEST_SCHEMA_VERSION.into(),
            adapter_id: "org.genegis.postgis".into(),
            adapter_version: "0.1.0".into(),
            backend: BackendIdentity {
                family: BackendFamily::Postgis,
                engine_version: "3.6.1".into(),
                build_digest: DIGEST.into(),
                components: BTreeMap::from([("postgresql".into(), "18.1".into())]),
            },
            license: "Apache-2.0 OR MIT".into(),
            operations: vec![AdapterOperation {
                operation_id: "postgis.query".into(),
                operation_version: "1.0.0".into(),
                inputs: vec![contract("query-input")],
                outputs: vec![contract("query-output")],
                capabilities,
                determinism: Determinism::ToleranceBounded,
                evidence_hooks: BTreeSet::from([
                    EvidenceHook::InputDigests,
                    EvidenceHook::OutputDigests,
                    EvidenceHook::Parameters,
                    EvidenceHook::ComponentIdentity,
                    EvidenceHook::EnvironmentDigest,
                    EvidenceHook::TransactionContext,
                ]),
                opaque: false,
            }],
        }
    }

    fn invocation(manifest: &AdapterManifest) -> AdapterInvocation {
        let operation = &manifest.operations[0];
        AdapterInvocation {
            adapter_id: manifest.adapter_id.clone(),
            adapter_version: manifest.adapter_version.clone(),
            operation_id: operation.operation_id.clone(),
            operation_version: operation.operation_version.clone(),
            backend: manifest.backend.clone(),
            requested_capabilities: operation.capabilities.clone(),
        }
    }

    #[test]
    fn admits_exact_read_only_postgis_operation() {
        let manifest = manifest(BTreeSet::from([Capability::DatabaseRead]));
        let report = admit(
            &manifest,
            &invocation(&manifest),
            &CapabilityPolicy::read_only_database(&manifest.adapter_id),
        );
        assert!(report.admitted);
        assert!(report.verification_eligible);
    }

    #[test]
    fn read_only_policy_rejects_postgis_write() {
        let manifest = manifest(BTreeSet::from([
            Capability::DatabaseRead,
            Capability::DatabaseWrite,
        ]));
        let report = admit(
            &manifest,
            &invocation(&manifest),
            &CapabilityPolicy::read_only_database(&manifest.adapter_id),
        );
        assert!(!report.admitted);
        assert!(report
            .failures
            .iter()
            .any(|failure| failure.code == "capability_denied"));
    }

    #[test]
    fn runtime_cannot_underdeclare_manifest_capabilities() {
        let manifest = manifest(BTreeSet::from([
            Capability::DatabaseRead,
            Capability::DatabaseWrite,
        ]));
        let mut invocation = invocation(&manifest);
        invocation
            .requested_capabilities
            .remove(&Capability::DatabaseWrite);
        let mut policy = CapabilityPolicy::read_only_database(&manifest.adapter_id);
        policy
            .allowed_capabilities
            .insert(Capability::DatabaseWrite);
        let report = admit(&manifest, &invocation, &policy);
        assert!(report
            .failures
            .iter()
            .any(|failure| failure.code == "capability_declaration_mismatch"));
    }

    #[test]
    fn opaque_or_backend_drift_cannot_inherit_verified_trust() {
        let mut manifest = manifest(BTreeSet::from([Capability::DatabaseRead]));
        manifest.operations[0].opaque = true;
        let opaque = admit(
            &manifest,
            &invocation(&manifest),
            &CapabilityPolicy::read_only_database(&manifest.adapter_id),
        );
        assert!(!opaque.admitted);
        assert!(!opaque.verification_eligible);

        manifest.operations[0].opaque = false;
        let mut invocation = invocation(&manifest);
        invocation.backend.engine_version = "3.7.0".into();
        let drift = admit(
            &manifest,
            &invocation,
            &CapabilityPolicy::read_only_database(&manifest.adapter_id),
        );
        assert!(drift
            .failures
            .iter()
            .any(|failure| failure.code == "backend_identity_mismatch"));
    }

    #[test]
    fn schema_is_committed_and_rejects_unknown_fields() {
        let schema = adapter_manifest_json_schema();
        assert_eq!(schema["$id"], ADAPTER_MANIFEST_SCHEMA_ID);
        let mut value =
            serde_json::to_value(manifest(BTreeSet::from([Capability::DatabaseRead]))).unwrap();
        value["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<AdapterManifest>(value).is_err());
    }
}
