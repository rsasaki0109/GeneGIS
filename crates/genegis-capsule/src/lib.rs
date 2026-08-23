//! Open, content-addressed result capsules with an offline verifier.

#![deny(missing_docs)]

use base64::{engine::general_purpose::STANDARD, Engine as _};
use genegis_analysis::{
    canonical_nagoya_execution_digest, AnalysisResult, AskPipelineResult, ExecutionReceipt,
    NagoyaArtifactDigests, NagoyaExecutionOutput,
};
use genegis_contract::{
    SourceAssurance, TrustAssessment, TrustLevel, VerificationGraph, VerificationPolicy,
};
use genegis_core::{Command, CommandEnvelope};
use genegis_workflow::GeoWorkflow;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

mod standards;

pub use standards::{
    create_dsse_attestation, ed25519_public_key, execute_ogc_verify_request,
    export_standard_bundle, validate_openlineage, validate_prov_json, validate_ro_crate,
    verify_dsse_attestation, DsseEnvelope, DsseSignature, StandardExportReport,
};

/// Current directory-capsule manifest version.
pub const CAPSULE_SCHEMA_VERSION: &str = "0.1.0";

const ANALYSIS_PATH: &str = "metadata/analysis.json";
const COMMAND_PATH: &str = "metadata/command.json";
const WORKFLOW_PATH: &str = "metadata/workflow.json";
const RECEIPT_PATH: &str = "metadata/receipt.json";
const POLICY_PATH: &str = "metadata/verification-policy.json";
const GRAPH_PATH: &str = "metadata/verification-graph.json";
const ASSURANCE_DIR: &str = "metadata/source-assurance";
const HTML_PATH: &str = "artifacts/map.html";
const PNG_PATH: &str = "artifacts/map.png";
const TRUST_REPORT_PATH: &str = "reports/trust.html";

/// One content-addressed file in a capsule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapsuleEntry {
    /// Portable path relative to the capsule root.
    pub path: String,
    /// Stable semantic role understood by verifiers and exporters.
    pub role: String,
    /// Media type of the exact bytes.
    pub media_type: String,
    /// SHA-256 digest of the exact bytes.
    pub sha256: String,
    /// Exact byte length.
    pub bytes: u64,
}

/// Canonical inventory stored as `capsule.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapsuleManifest {
    /// Manifest shape and verifier compatibility version.
    pub schema_version: String,
    /// Open capsule profile implemented by this crate.
    pub profile: String,
    /// Canonical execution-result digest carried by the capsule.
    pub subject_result_digest: String,
    /// Content-addressed files, sorted by portable path.
    pub entries: Vec<CapsuleEntry>,
}

/// Successful offline verification result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapsuleVerification {
    /// Capsule schema verified by this implementation.
    pub schema_version: String,
    /// Canonical execution-result digest recomputed from capsule subjects.
    pub result_digest: String,
    /// Canonical verification-graph digest recomputed offline.
    pub verification_graph_digest: String,
    /// Policy-derived trust recomputed offline.
    pub trust: TrustAssessment,
    /// Number of content-addressed files checked.
    pub verified_entries: usize,
}

/// Semantic area affected by a capsule change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticCategory {
    /// Input identity, license, citation, or source-admission evidence.
    Source,
    /// GeoContract meaning or compatibility requirement.
    Contract,
    /// Command or execution Workflow Graph semantics.
    Workflow,
    /// Numeric, geometry, styling, or declared result meaning.
    Result,
    /// Verification claim, evidence, tolerance, graph, or trust state.
    Verification,
    /// Release-policy rule.
    Policy,
    /// Rendered or exported artifact bytes.
    Artifact,
    /// A role not recognized by this adapter.
    Unclassified,
}

/// Structural form of one semantic change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    /// Value exists only in the new capsule.
    Added,
    /// Value exists only in the old capsule.
    Removed,
    /// Both capsules contain different values.
    Modified,
}

/// One leaf-level semantic capsule change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticChange {
    /// Classified area affected by the change.
    pub category: SemanticCategory,
    /// Stable manifest role of the changed subject.
    pub subject_role: String,
    /// JSON pointer or subject path locating the change.
    pub path: String,
    /// Whether the value was added, removed, or modified.
    pub kind: ChangeKind,
    /// Old leaf value, omitted for additions and binary artifacts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old: Option<serde_json::Value>,
    /// New leaf value, omitted for removals and binary artifacts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new: Option<serde_json::Value>,
}

/// Complete semantic comparison between two capsules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticDiffReport {
    /// Old canonical result identity.
    pub old_result_digest: String,
    /// New canonical result identity.
    pub new_result_digest: String,
    /// Deterministically ordered semantic changes.
    pub changes: Vec<SemanticChange>,
    /// Count of changes whose manifest role has no adapter.
    pub unclassified_changes: usize,
}

/// Reviewer approval bound to every semantic release identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisApproval {
    /// Approval document schema version.
    pub schema_version: String,
    /// Human or institutional reviewer identity.
    pub reviewer: String,
    /// RFC 3339 approval time supplied by the caller.
    pub approved_at: String,
    /// Exact capsule manifest bytes reviewed.
    pub capsule_manifest_digest: String,
    /// Canonical result identity reviewed.
    pub result_digest: String,
    /// Canonical Workflow Graph identity reviewed.
    pub workflow_digest: String,
    /// Exact versioned policy identity reviewed.
    pub policy_digest: String,
    /// Canonical Verification Graph identity reviewed.
    pub verification_graph_digest: String,
    /// Optional semantic diff identity reviewed for an update.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_diff_digest: Option<String>,
}

/// One verification claim prepared for reviewer navigation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimReview {
    /// Stable claim/check identifier.
    pub check_id: String,
    /// Human-readable claim made by the verifier.
    pub claim: String,
    /// Verifier engine and implementation identity.
    pub verifier: String,
    /// Declared executor/verifier relationship.
    pub independence: String,
    /// Maximum policy error in parts per million.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum_error_ppm: Option<u64>,
    /// Observed error in parts per million.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_error_ppm: Option<u64>,
    /// Whether normalized evidence says the claim passed.
    pub passed: bool,
    /// Claims that must be established first.
    pub depends_on: Vec<String>,
    /// Workflow nodes whose outputs or inputs are tested by this claim.
    pub workflow_nodes: Vec<String>,
}

/// One immutable input source prepared for reviewer inspection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceReview {
    /// Stable dataset identity, or a deterministic position when absent.
    pub source_id: String,
    /// Source URI or local path recorded by execution.
    pub uri: String,
    /// Declared source release/version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Declared license or usage terms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Expected content digest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_checksum: Option<String>,
    /// Digest observed from executed bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_checksum: Option<String>,
    /// Human-readable checksum admission state.
    pub checksum_status: String,
    /// Digest binding the complete Source Assurance dossier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assurance_digest: Option<String>,
    /// Highest assurance level justified by the selected release policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assurance_level: Option<String>,
    /// Source caveats and explicit non-claims shown without raw JSON.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limitations: Vec<String>,
    /// Number of open or acknowledged source disputes.
    pub unresolved_disputes: usize,
}

/// One executable node in the reviewed Workflow Graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowNodeReview {
    /// Stable graph node identity.
    pub stable_id: String,
    /// Operation dispatched by the node.
    pub operation: String,
    /// Stable dependency node identities.
    pub depends_on: Vec<String>,
    /// Exact operator parameters.
    pub parameters: serde_json::Value,
}

/// Semantic identities shown consistently by JSON, TUI, HTML, and capsule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DigestIdentities {
    /// Canonical analysis result identity.
    pub result_digest: String,
    /// Canonical Workflow Graph identity.
    pub workflow_digest: String,
    /// Exact embedded policy identity.
    pub policy_digest: String,
    /// Canonical Verification Graph identity.
    pub verification_graph_digest: String,
    /// Exact map HTML bytes identity.
    pub map_html_digest: String,
    /// Exact map PNG bytes identity.
    pub map_png_digest: String,
}

/// Policy failure enriched with a deterministic review target and remediation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionableFailure {
    /// Trust gate that rejected the evidence.
    pub gate: genegis_contract::TrustGate,
    /// Stable machine-readable predicate code.
    pub code: String,
    /// Affected contract, source, check, artifact, or signer.
    pub subject: String,
    /// Human-readable evidence failure.
    pub detail: String,
    /// Workflow nodes to inspect first.
    pub affected_nodes: Vec<String>,
    /// Safe next action that does not weaken policy.
    pub remediation: String,
}

/// Stable review model shared by JSON output and the TUI Trust Debugger.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustReview {
    /// Capsule path shown to the reviewer.
    pub capsule: String,
    /// Recomputed verification result, when the capsule is internally valid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<CapsuleVerification>,
    /// Integrity error retained for debugging instead of hiding all subjects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integrity_error: Option<String>,
    /// Stable identities shared by every review representation.
    pub identities: DigestIdentities,
    /// Verification claims in graph order.
    pub claims: Vec<ClaimReview>,
    /// GeoContract evidence used by the policy.
    pub contracts: Vec<genegis_contract::ContractEvidence>,
    /// Immutable source snapshots used by the execution.
    pub sources: Vec<SourceReview>,
    /// Executable Workflow DAG in stored order.
    pub workflow_nodes: Vec<WorkflowNodeReview>,
    /// Content-addressed artifacts and metadata subjects.
    pub artifacts: Vec<CapsuleEntry>,
    /// Structured policy failures explaining the current trust ceiling.
    pub failures: Vec<ActionableFailure>,
    /// Optional semantic comparison against another capsule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_diff: Option<SemanticDiffReport>,
}

/// Capsule creation or verification failure.
#[derive(Debug, Error)]
pub enum CapsuleError {
    /// Filesystem access failed.
    #[error("capsule I/O failed for {path}: {source}")]
    Io {
        /// Affected path.
        path: PathBuf,
        /// Underlying error.
        source: std::io::Error,
    },
    /// JSON parsing or serialization failed.
    #[error("capsule JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    /// Capsule contents failed a semantic or integrity check.
    #[error("capsule verification failed: {0}")]
    Verification(String),
}

/// Seal one successful Nagoya run into a new ordinary directory.
///
/// The destination must not already contain files. This prevents an older,
/// unlisted artifact from being mistaken for part of the new capsule.
pub fn seal_nagoya_capsule(
    result: &AskPipelineResult,
    root: impl AsRef<Path>,
) -> Result<CapsuleManifest, CapsuleError> {
    let root = root.as_ref();
    if root.exists() {
        let mut entries = fs::read_dir(root).map_err(|source| io_error(root, source))?;
        if entries
            .next()
            .transpose()
            .map_err(|source| io_error(root, source))?
            .is_some()
        {
            return Err(CapsuleError::Verification(format!(
                "destination is not empty: {}",
                root.display()
            )));
        }
    }
    fs::create_dir_all(root.join("metadata")).map_err(|source| io_error(root, source))?;
    fs::create_dir_all(root.join(ASSURANCE_DIR)).map_err(|source| io_error(root, source))?;
    fs::create_dir_all(root.join("artifacts")).map_err(|source| io_error(root, source))?;
    fs::create_dir_all(root.join("reports")).map_err(|source| io_error(root, source))?;

    let policy = result
        .execution_receipt
        .verification_policy
        .as_ref()
        .ok_or_else(|| CapsuleError::Verification("receipt has no verification policy".into()))?;
    let graph = result
        .execution_receipt
        .verification_graph
        .as_ref()
        .ok_or_else(|| CapsuleError::Verification("receipt has no verification graph".into()))?;
    let trust = result
        .execution_receipt
        .trust_assessment
        .as_ref()
        .ok_or_else(|| CapsuleError::Verification("receipt has no trust assessment".into()))?;
    let identities = DigestIdentities {
        result_digest: result.execution_receipt.result_digest.clone(),
        workflow_digest: result
            .execution_receipt
            .workflow_digest
            .as_str()
            .to_string(),
        policy_digest: digest(&serde_json::to_vec(policy)?),
        verification_graph_digest: graph
            .stable_digest()
            .map_err(|error| verify_error(error.to_string()))?,
        map_html_digest: digest(result.html.as_bytes()),
        map_png_digest: digest(&result.png),
    };
    let mut subjects = vec![
        json_subject(
            ANALYSIS_PATH,
            "analysis-result",
            result.analysis.as_ref().ok_or_else(|| {
                CapsuleError::Verification("ask result has no typed analysis".into())
            })?,
        )?,
        json_subject(COMMAND_PATH, "command", &result.command)?,
        json_subject(WORKFLOW_PATH, "workflow", &result.workflow)?,
        json_subject(RECEIPT_PATH, "execution-receipt", &result.execution_receipt)?,
        json_subject(POLICY_PATH, "verification-policy", policy)?,
        json_subject(GRAPH_PATH, "verification-graph", graph)?,
        Subject::new(
            HTML_PATH,
            "map-artifact",
            "text/html; charset=utf-8",
            result.html.as_bytes().to_vec(),
        ),
        Subject::new(PNG_PATH, "map-artifact", "image/png", result.png.clone()),
        Subject::new(
            TRUST_REPORT_PATH,
            "trust-report",
            "text/html; charset=utf-8",
            trust_report_html(trust, &identities).into_bytes(),
        ),
    ];
    if let Some(evidence) = result.execution_receipt.trust_evidence.as_ref() {
        for (index, source) in evidence.sources.iter().enumerate() {
            if let Some(assurance) = source.assurance.as_ref() {
                subjects.push(json_subject(
                    &format!("{ASSURANCE_DIR}/{index:03}.json"),
                    "source-assurance",
                    assurance,
                )?);
            }
        }
    }
    let mut manifest_entries = Vec::with_capacity(subjects.len());
    for subject in subjects {
        write_file(&root.join(&subject.path), &subject.bytes)?;
        manifest_entries.push(subject.entry());
    }
    manifest_entries.sort_by(|left, right| left.path.cmp(&right.path));
    let manifest = CapsuleManifest {
        schema_version: CAPSULE_SCHEMA_VERSION.into(),
        profile: "https://genegis.org/profiles/proof-carrying-spatial-analysis/0.1".into(),
        subject_result_digest: result.execution_receipt.result_digest.clone(),
        entries: manifest_entries,
    };
    write_file(
        &root.join("capsule.json"),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(manifest)
}

/// Verify every capsule subject and recompute policy-derived trust offline.
///
/// When `expected_policy` is supplied, the embedded policy must match it
/// exactly. This prevents a capsule from weakening its own release rules.
pub fn verify_nagoya_capsule(
    root: impl AsRef<Path>,
    expected_policy: Option<&VerificationPolicy>,
) -> Result<CapsuleVerification, CapsuleError> {
    let root = root.as_ref();
    let manifest: CapsuleManifest = read_json(&root.join("capsule.json"))?;
    if manifest.schema_version != CAPSULE_SCHEMA_VERSION {
        return Err(verify_error("unsupported capsule schema version"));
    }
    if !valid_digest(&manifest.subject_result_digest) {
        return Err(verify_error("malformed subject result digest"));
    }
    let mut paths = BTreeSet::new();
    for entry in &manifest.entries {
        validate_relative_path(&entry.path)?;
        if !paths.insert(entry.path.as_str()) {
            return Err(verify_error(format!(
                "duplicate manifest path: {}",
                entry.path
            )));
        }
        let path = root.join(&entry.path);
        let metadata = fs::symlink_metadata(&path).map_err(|source| io_error(&path, source))?;
        if !metadata.file_type().is_file() {
            return Err(verify_error(format!(
                "subject is not a regular file: {}",
                entry.path
            )));
        }
        let bytes = read_file(&path)?;
        if bytes.len() as u64 != entry.bytes || digest(&bytes) != entry.sha256 {
            return Err(verify_error(format!(
                "subject digest/size mismatch: {}",
                entry.path
            )));
        }
    }
    for required in [
        ANALYSIS_PATH,
        COMMAND_PATH,
        WORKFLOW_PATH,
        RECEIPT_PATH,
        POLICY_PATH,
        GRAPH_PATH,
        HTML_PATH,
        PNG_PATH,
        TRUST_REPORT_PATH,
    ] {
        if !paths.contains(required) {
            return Err(verify_error(format!(
                "required subject is missing: {required}"
            )));
        }
    }

    let receipt: ExecutionReceipt = read_json(&root.join(RECEIPT_PATH))?;
    let command: CommandEnvelope = read_json(&root.join(COMMAND_PATH))?;
    let embedded_policy: VerificationPolicy = read_json(&root.join(POLICY_PATH))?;
    let graph: VerificationGraph = read_json(&root.join(GRAPH_PATH))?;
    if let Some(expected) = expected_policy {
        if expected != &embedded_policy {
            return Err(verify_error("embedded policy differs from required policy"));
        }
    }
    if receipt.verification_policy.as_ref() != Some(&embedded_policy)
        || receipt.verification_graph.as_ref() != Some(&graph)
    {
        return Err(verify_error(
            "receipt policy/graph differs from capsule subjects",
        ));
    }
    graph
        .validate_against_policy(&embedded_policy)
        .map_err(|error| verify_error(error.to_string()))?;
    let graph_digest = graph
        .stable_digest()
        .map_err(|error| verify_error(error.to_string()))?;
    if receipt.verification_graph_digest.as_deref() != Some(graph_digest.as_str()) {
        return Err(verify_error("verification graph digest mismatch"));
    }
    let trust_evidence = receipt
        .trust_evidence
        .as_ref()
        .ok_or_else(|| verify_error("receipt has no normalized trust evidence"))?;
    let assurance_entries = manifest
        .entries
        .iter()
        .filter(|entry| entry.role == "source-assurance")
        .collect::<Vec<_>>();
    let expected_assurance_count = trust_evidence
        .sources
        .iter()
        .filter(|source| source.assurance.is_some())
        .count();
    if assurance_entries.len() != expected_assurance_count {
        return Err(verify_error(
            "source assurance subject count differs from trust evidence",
        ));
    }
    let mut assurance_source_ids = BTreeSet::new();
    for entry in assurance_entries {
        let assurance: SourceAssurance = read_json(&root.join(&entry.path))?;
        if !assurance_source_ids.insert(assurance.source_id.clone()) {
            return Err(verify_error("duplicate source assurance identity"));
        }
        let source = trust_evidence
            .sources
            .iter()
            .find(|source| source.source_id == assurance.source_id)
            .ok_or_else(|| verify_error("source assurance has no matching trust evidence"))?;
        if source.assurance.as_ref() != Some(&assurance) {
            return Err(verify_error(
                "source assurance subject differs from trust evidence",
            ));
        }
        let assurance_digest = assurance.digest()?;
        if source.assurance_digest.as_deref() != Some(assurance_digest.as_str())
            || source.snapshot_digest.as_deref() != Some(assurance.snapshot_digest.as_str())
        {
            return Err(verify_error(
                "source assurance digest or snapshot binding mismatch",
            ));
        }
    }
    let trust = embedded_policy.assess(trust_evidence);
    if receipt.trust_assessment.as_ref() != Some(&trust)
        || receipt.verification_passed != (trust.level >= TrustLevel::Verified)
    {
        return Err(verify_error(
            "stored trust state differs from policy derivation",
        ));
    }

    let analysis: AnalysisResult = read_json(&root.join(ANALYSIS_PATH))?;
    let workflow: GeoWorkflow = read_json(&root.join(WORKFLOW_PATH))?;
    match command.command {
        Command::RunWorkflow { workflow_id } if workflow_id == workflow.id => {}
        _ => {
            return Err(verify_error(
                "command does not authorize the capsule workflow",
            ))
        }
    }
    let workflow_digest = workflow
        .stable_digest()
        .map_err(|error| verify_error(error.to_string()))?;
    if command.workflow_digest.as_ref() != Some(&receipt.workflow_digest)
        || command
            .workflow_digest
            .as_ref()
            .map(|digest| digest.as_str())
            != Some(workflow_digest.as_str())
        || receipt.workflow_id != workflow.id
        || receipt.command_id != command.id
    {
        return Err(verify_error(
            "command, workflow, and receipt identities do not agree",
        ));
    }
    let analysis_workflow_digest = analysis
        .workflow
        .stable_digest()
        .map_err(|error| verify_error(error.to_string()))?;
    if analysis_workflow_digest != workflow_digest
        || receipt.workflow_digest.as_str() != workflow_digest
    {
        return Err(verify_error("workflow identities do not agree"));
    }
    let html = read_file(&root.join(HTML_PATH))?;
    let png = read_file(&root.join(PNG_PATH))?;
    let artifacts = NagoyaArtifactDigests {
        html_sha256: Some(digest(&html)),
        png_sha256: Some(digest(&png)),
        html_bytes: html.len(),
        png_bytes: png.len(),
    };
    let identities = DigestIdentities {
        result_digest: receipt.result_digest.clone(),
        workflow_digest: workflow_digest.clone(),
        policy_digest: digest(&serde_json::to_vec(&embedded_policy)?),
        verification_graph_digest: graph_digest.clone(),
        map_html_digest: artifacts.html_sha256.clone().expect("HTML digest"),
        map_png_digest: artifacts.png_sha256.clone().expect("PNG digest"),
    };
    let stored_report = read_file(&root.join(TRUST_REPORT_PATH))?;
    if stored_report != trust_report_html(&trust, &identities).as_bytes() {
        return Err(verify_error(
            "trust HTML differs from policy-derived state or digest identities",
        ));
    }
    let replay = &trust_evidence.replay;
    let expected_backend = format!(
        "{}:{}:{}",
        receipt.engine.name, receipt.engine.version, receipt.engine.build
    );
    let expected_artifacts = BTreeSet::from([
        artifacts.html_sha256.clone().expect("HTML digest"),
        artifacts.png_sha256.clone().expect("PNG digest"),
    ]);
    if replay.workflow_digest.as_deref() != Some(receipt.workflow_digest.as_str())
        || replay.backend_identity.as_deref() != Some(expected_backend.as_str())
        || replay
            .artifact_digests
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            != expected_artifacts
    {
        return Err(verify_error(
            "trust replay evidence differs from receipt or artifact subjects",
        ));
    }
    let output = NagoyaExecutionOutput {
        analysis,
        verification_passed: receipt.verification_passed,
        html: String::from_utf8(html).map_err(|_| verify_error("HTML artifact is not UTF-8"))?,
        png_base64: STANDARD.encode(&png),
        artifact_digests: artifacts,
    };
    let result_digest = canonical_nagoya_execution_digest(&output, &receipt.evidence);
    if result_digest != manifest.subject_result_digest || result_digest != receipt.result_digest {
        return Err(verify_error("canonical execution result digest mismatch"));
    }
    if trust_evidence.replay.result_digest.as_deref() != Some(result_digest.as_str()) {
        return Err(verify_error(
            "trust evidence names a different result digest",
        ));
    }
    Ok(CapsuleVerification {
        schema_version: manifest.schema_version,
        result_digest,
        verification_graph_digest: graph_digest,
        trust,
        verified_entries: manifest.entries.len(),
    })
}

/// Compare two capsules by semantic JSON leaves and stable artifact subjects.
///
/// Runtime UUIDs, command timestamps, and retrieval observations are removed
/// before comparison so two equivalent replays produce an empty diff.
pub fn diff_capsules(
    old_root: impl AsRef<Path>,
    new_root: impl AsRef<Path>,
) -> Result<SemanticDiffReport, CapsuleError> {
    let old_root = old_root.as_ref();
    let new_root = new_root.as_ref();
    let old: CapsuleManifest = read_json(&old_root.join("capsule.json"))?;
    let new: CapsuleManifest = read_json(&new_root.join("capsule.json"))?;
    let old_entries = old
        .entries
        .iter()
        .map(|entry| (entry.path.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    let new_entries = new
        .entries
        .iter()
        .map(|entry| (entry.path.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    let paths = old_entries
        .keys()
        .chain(new_entries.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    let mut changes = Vec::new();
    for path in paths {
        match (old_entries.get(path), new_entries.get(path)) {
            (Some(old_entry), Some(new_entry)) if old_entry.sha256 == new_entry.sha256 => {}
            (Some(old_entry), Some(new_entry))
                if old_entry.media_type.starts_with("application/json")
                    && new_entry.media_type.starts_with("application/json") =>
            {
                let mut old_value: serde_json::Value = read_json(&old_root.join(path))?;
                let mut new_value: serde_json::Value = read_json(&new_root.join(path))?;
                strip_diff_runtime(&mut old_value);
                strip_diff_runtime(&mut new_value);
                diff_json(&old_value, &new_value, "", &old_entry.role, &mut changes);
            }
            (Some(old_entry), Some(_new_entry)) => changes.push(SemanticChange {
                category: category_for(&old_entry.role, path),
                subject_role: old_entry.role.clone(),
                path: path.into(),
                kind: ChangeKind::Modified,
                old: None,
                new: None,
            }),
            (Some(entry), None) => changes.push(SemanticChange {
                category: category_for(&entry.role, path),
                subject_role: entry.role.clone(),
                path: path.into(),
                kind: ChangeKind::Removed,
                old: None,
                new: None,
            }),
            (None, Some(entry)) => changes.push(SemanticChange {
                category: category_for(&entry.role, path),
                subject_role: entry.role.clone(),
                path: path.into(),
                kind: ChangeKind::Added,
                old: None,
                new: None,
            }),
            (None, None) => unreachable!(),
        }
    }
    if old.subject_result_digest != new.subject_result_digest {
        changes.push(SemanticChange {
            category: SemanticCategory::Result,
            subject_role: "capsule-manifest".into(),
            path: "/subject_result_digest".into(),
            kind: ChangeKind::Modified,
            old: Some(serde_json::json!(old.subject_result_digest)),
            new: Some(serde_json::json!(new.subject_result_digest)),
        });
    }
    changes.sort_by(|left, right| {
        left.category
            .cmp(&right.category)
            .then_with(|| left.subject_role.cmp(&right.subject_role))
            .then_with(|| left.path.cmp(&right.path))
    });
    let unclassified_changes = changes
        .iter()
        .filter(|change| change.category == SemanticCategory::Unclassified)
        .count();
    Ok(SemanticDiffReport {
        old_result_digest: old.subject_result_digest,
        new_result_digest: new.subject_result_digest,
        changes,
        unclassified_changes,
    })
}

/// Create an approval bound to a capsule and optional semantic diff.
pub fn create_approval(
    root: impl AsRef<Path>,
    reviewer: impl Into<String>,
    approved_at: impl Into<String>,
    diff: Option<&SemanticDiffReport>,
) -> Result<AnalysisApproval, CapsuleError> {
    let reviewer = reviewer.into();
    let approved_at = approved_at.into();
    if reviewer.trim().is_empty() || approved_at.trim().is_empty() {
        return Err(verify_error("approval reviewer and time are required"));
    }
    let context = approval_context(root.as_ref(), diff)?;
    Ok(AnalysisApproval {
        schema_version: "0.1.0".into(),
        reviewer,
        approved_at,
        capsule_manifest_digest: context.capsule_manifest_digest,
        result_digest: context.result_digest,
        workflow_digest: context.workflow_digest,
        policy_digest: context.policy_digest,
        verification_graph_digest: context.verification_graph_digest,
        semantic_diff_digest: context.semantic_diff_digest,
    })
}

/// Build the stable data model used by the Trust Debugger and JSON report.
pub fn review_capsule(
    root: impl AsRef<Path>,
    expected_policy: Option<&VerificationPolicy>,
) -> Result<TrustReview, CapsuleError> {
    review_capsule_with_diff(root, expected_policy, None)
}

/// Build a Trust Debugger model with an optional semantic capsule comparison.
pub fn review_capsule_with_diff(
    root: impl AsRef<Path>,
    expected_policy: Option<&VerificationPolicy>,
    semantic_diff: Option<SemanticDiffReport>,
) -> Result<TrustReview, CapsuleError> {
    let root = root.as_ref();
    let manifest: CapsuleManifest = read_json(&root.join("capsule.json"))?;
    let receipt: ExecutionReceipt = read_json(&root.join(RECEIPT_PATH))?;
    let workflow: GeoWorkflow = read_json(&root.join(WORKFLOW_PATH))?;
    let policy: VerificationPolicy = read_json(&root.join(POLICY_PATH))?;
    let graph: VerificationGraph = read_json(&root.join(GRAPH_PATH))?;
    let artifact_digest = |path: &str| {
        manifest
            .entries
            .iter()
            .find(|entry| entry.path == path)
            .map(|entry| entry.sha256.clone())
            .ok_or_else(|| verify_error(format!("review subject is missing: {path}")))
    };
    let identities = DigestIdentities {
        result_digest: manifest.subject_result_digest.clone(),
        workflow_digest: workflow
            .stable_digest()
            .map_err(|error| verify_error(error.to_string()))?,
        policy_digest: digest(&serde_json::to_vec(&policy)?),
        verification_graph_digest: graph
            .stable_digest()
            .map_err(|error| verify_error(error.to_string()))?,
        map_html_digest: artifact_digest(HTML_PATH)?,
        map_png_digest: artifact_digest(PNG_PATH)?,
    };
    let evidence = receipt.trust_evidence.as_ref();
    let claims = graph
        .nodes
        .iter()
        .map(|node| {
            let observation = evidence.and_then(|evidence| {
                evidence
                    .checks
                    .iter()
                    .find(|check| check.check_id == node.check_id)
            });
            ClaimReview {
                check_id: node.check_id.clone(),
                claim: node.claim.clone(),
                verifier: format!(
                    "{}:{}:{}",
                    node.verifier.verifier_id, node.verifier.engine, node.verifier.implementation
                ),
                independence: format!("{:?}", node.verifier.independence).to_lowercase(),
                maximum_error_ppm: node
                    .tolerance
                    .as_ref()
                    .map(|tolerance| tolerance.max_error_ppm),
                observed_error_ppm: observation.and_then(|check| check.observed_error_ppm),
                passed: observation.is_some_and(|check| check.passed),
                depends_on: node.depends_on.clone(),
                workflow_nodes: workflow_nodes_for_check(&node.check_id, &workflow),
            }
        })
        .collect();
    let contracts = evidence
        .map(|evidence| evidence.contracts.clone())
        .unwrap_or_default();
    let stored_failures = receipt
        .trust_assessment
        .as_ref()
        .map(|assessment| assessment.failures.clone())
        .unwrap_or_default();
    let (verification, integrity_error, raw_failures) =
        match verify_nagoya_capsule(root, expected_policy) {
            Ok(verification) => {
                let failures = verification.trust.failures.clone();
                (Some(verification), None, failures)
            }
            Err(error) => (None, Some(error.to_string()), stored_failures),
        };
    let failures = raw_failures
        .iter()
        .map(|failure| actionable_failure(failure, &workflow))
        .collect();
    Ok(TrustReview {
        capsule: root.display().to_string(),
        verification,
        integrity_error,
        identities,
        claims,
        contracts,
        sources: receipt
            .source_snapshots
            .iter()
            .enumerate()
            .map(|(index, source)| {
                let source_id = source
                    .dataset_id
                    .clone()
                    .unwrap_or_else(|| format!("source-{index}"));
                let source_evidence = evidence.and_then(|evidence| {
                    evidence
                        .sources
                        .iter()
                        .find(|candidate| candidate.source_id == source_id)
                });
                let assurance = source_evidence.and_then(|evidence| evidence.assurance.as_ref());
                let assurance_level = policy.source_assurance.as_ref().and_then(|policy| {
                    assurance.map(|assurance| {
                        format!("{:?}", policy.assess(assurance).level).to_ascii_lowercase()
                    })
                });
                SourceReview {
                    source_id,
                    uri: source.uri.clone(),
                    version: source.source_version.as_ref().map(ToString::to_string),
                    license: source.license.clone(),
                    expected_checksum: source.expected_checksum.clone(),
                    observed_checksum: source.observed_checksum.clone(),
                    checksum_status: source.checksum_status.to_string(),
                    assurance_digest: source_evidence
                        .and_then(|evidence| evidence.assurance_digest.clone()),
                    assurance_level,
                    limitations: assurance
                        .map(|assurance| assurance.limitations.clone())
                        .unwrap_or_default(),
                    unresolved_disputes: assurance
                        .map(|assurance| {
                            assurance
                                .disputes
                                .iter()
                                .filter(|dispute| {
                                    dispute.status != genegis_contract::DisputeStatus::Resolved
                                })
                                .count()
                        })
                        .unwrap_or(0),
                }
            })
            .collect(),
        workflow_nodes: workflow
            .steps
            .iter()
            .map(|step| WorkflowNodeReview {
                stable_id: step.stable_id.clone(),
                operation: step.operation.clone(),
                depends_on: step
                    .depends_on
                    .iter()
                    .map(|dependency| dependency.0.clone())
                    .collect(),
                parameters: step.parameters.clone(),
            })
            .collect(),
        artifacts: manifest.entries,
        failures,
        semantic_diff,
    })
}

fn workflow_nodes_for_check(check_id: &str, workflow: &GeoWorkflow) -> Vec<String> {
    let candidates: &[&str] = match check_id {
        "source_checksum" => &["load-boundary", "load-population"],
        "population_total_oracle" => &["load-population", "join-population-to-geometry"],
        "ward_coverage_oracle" => &["normalize-schema", "join-population-to-geometry"],
        "area_oracle_relative_error" => &["reproject-for-area", "calculate-area-km2"],
        "density_oracle" => &["calculate-density"],
        _ => &[],
    };
    candidates
        .iter()
        .filter(|candidate| {
            workflow
                .steps
                .iter()
                .any(|step| step.stable_id == **candidate)
        })
        .map(|candidate| (*candidate).to_string())
        .collect()
}

fn actionable_failure(
    failure: &genegis_contract::TrustFailure,
    workflow: &GeoWorkflow,
) -> ActionableFailure {
    let mut affected_nodes = workflow_nodes_for_check(&failure.subject, workflow);
    if affected_nodes.is_empty() {
        affected_nodes = match failure.code.as_str() {
            code if code.contains("source") => vec!["load-boundary", "load-population"],
            code if code.contains("contract") => vec!["normalize-schema"],
            code if code.contains("workflow") => vec!["resolve-place"],
            code if code.contains("artifact") => vec!["render-map"],
            _ => Vec::new(),
        }
        .into_iter()
        .filter(|candidate| {
            workflow
                .steps
                .iter()
                .any(|step| step.stable_id == *candidate)
        })
        .map(ToOwned::to_owned)
        .collect();
    }
    let remediation = match failure.code.as_str() {
        "source_not_verified" => {
            "Restore authorized bytes or update the source snapshot through a reviewed command."
        }
        "missing_source_version" => "Pin a stable provider release before replay.",
        "missing_source_license" => "Record explicit source usage terms before release.",
        "missing_contract_evidence" | "contract_not_compatible" => {
            "Correct the GeoContract or transform the input explicitly; do not bypass compatibility."
        }
        "check_failed" | "check_tolerance_exceeded" => {
            "Inspect the affected workflow node, correct its input or operation, and rerun the independent check."
        }
        "insufficient_verifier_independence" => {
            "Run the claim with a policy-accepted independent engine or authoritative oracle."
        }
        "missing_check_evidence" | "missing_check_digest" | "missing_check_error" => {
            "Regenerate content-addressed check evidence with the required observation."
        }
        code if code.contains("attestation") || code.contains("attester") => {
            "Verify the DSSE signature and signer policy; a signature must never upgrade failed spatial evidence."
        }
        code if code.contains("workflow") || code.contains("result") || code.contains("backend") => {
            "Replay through Command + Workflow Graph and regenerate stable identities."
        }
        code if code.contains("artifact") => {
            "Regenerate the artifact from the verified numeric result and reseal the capsule."
        }
        _ => "Restore the required evidence and rerun verification without weakening policy.",
    };
    ActionableFailure {
        gate: failure.gate,
        code: failure.code.clone(),
        subject: failure.subject.clone(),
        detail: failure.detail.clone(),
        affected_nodes,
        remediation: remediation.into(),
    }
}

fn trust_report_html(trust: &TrustAssessment, identities: &DigestIdentities) -> String {
    let trust_level = format!("{:?}", trust.level).to_lowercase();
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><title>GeneGIS trust report</title></head><body>\n<h1>GeneGIS trust report</h1>\n<dl><dt>Trust</dt><dd>{}</dd><dt>Policy</dt><dd>{}</dd><dt>Result digest</dt><dd>{}</dd><dt>Workflow digest</dt><dd>{}</dd><dt>Policy digest</dt><dd>{}</dd><dt>Verification graph digest</dt><dd>{}</dd><dt>Map HTML digest</dt><dd>{}</dd><dt>Map PNG digest</dt><dd>{}</dd></dl>\n</body></html>\n",
        escape_html(&trust_level),
        escape_html(&trust.policy_id),
        escape_html(&identities.result_digest),
        escape_html(&identities.workflow_digest),
        escape_html(&identities.policy_digest),
        escape_html(&identities.verification_graph_digest),
        escape_html(&identities.map_html_digest),
        escape_html(&identities.map_png_digest),
    )
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Verify that an approval still names the exact capsule and optional diff.
pub fn verify_approval(
    root: impl AsRef<Path>,
    approval: &AnalysisApproval,
    diff: Option<&SemanticDiffReport>,
) -> Result<(), CapsuleError> {
    if approval.schema_version != "0.1.0" {
        return Err(verify_error("unsupported approval schema version"));
    }
    let context = approval_context(root.as_ref(), diff)?;
    if approval.capsule_manifest_digest != context.capsule_manifest_digest
        || approval.result_digest != context.result_digest
        || approval.workflow_digest != context.workflow_digest
        || approval.policy_digest != context.policy_digest
        || approval.verification_graph_digest != context.verification_graph_digest
        || approval.semantic_diff_digest != context.semantic_diff_digest
    {
        return Err(verify_error(
            "approval is stale for the capsule or semantic diff",
        ));
    }
    Ok(())
}

struct ApprovalContext {
    capsule_manifest_digest: String,
    result_digest: String,
    workflow_digest: String,
    policy_digest: String,
    verification_graph_digest: String,
    semantic_diff_digest: Option<String>,
}

fn approval_context(
    root: &Path,
    diff: Option<&SemanticDiffReport>,
) -> Result<ApprovalContext, CapsuleError> {
    let manifest_bytes = read_file(&root.join("capsule.json"))?;
    let manifest: CapsuleManifest = serde_json::from_slice(&manifest_bytes)?;
    let workflow: GeoWorkflow = read_json(&root.join(WORKFLOW_PATH))?;
    let policy: VerificationPolicy = read_json(&root.join(POLICY_PATH))?;
    let graph: VerificationGraph = read_json(&root.join(GRAPH_PATH))?;
    Ok(ApprovalContext {
        capsule_manifest_digest: digest(&manifest_bytes),
        result_digest: manifest.subject_result_digest,
        workflow_digest: workflow
            .stable_digest()
            .map_err(|error| verify_error(error.to_string()))?,
        policy_digest: digest(&serde_json::to_vec(&policy)?),
        verification_graph_digest: graph
            .stable_digest()
            .map_err(|error| verify_error(error.to_string()))?,
        semantic_diff_digest: diff
            .map(serde_json::to_vec)
            .transpose()?
            .map(|bytes| digest(&bytes)),
    })
}

struct Subject {
    path: String,
    role: String,
    media_type: String,
    bytes: Vec<u8>,
}

impl Subject {
    fn new(path: &str, role: &str, media_type: &str, bytes: Vec<u8>) -> Self {
        Self {
            path: path.into(),
            role: role.into(),
            media_type: media_type.into(),
            bytes,
        }
    }

    fn entry(&self) -> CapsuleEntry {
        CapsuleEntry {
            path: self.path.clone(),
            role: self.role.clone(),
            media_type: self.media_type.clone(),
            sha256: digest(&self.bytes),
            bytes: self.bytes.len() as u64,
        }
    }
}

fn json_subject<T: Serialize>(path: &str, role: &str, value: &T) -> Result<Subject, CapsuleError> {
    Ok(Subject::new(
        path,
        role,
        "application/json",
        serde_json::to_vec_pretty(value)?,
    ))
}

fn validate_relative_path(path: &str) -> Result<(), CapsuleError> {
    let candidate = Path::new(path);
    if path.is_empty()
        || candidate.is_absolute()
        || candidate
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(verify_error(format!("unsafe capsule path: {path}")));
    }
    Ok(())
}

fn valid_digest(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn read_file(path: &Path) -> Result<Vec<u8>, CapsuleError> {
    fs::read(path).map_err(|source| io_error(path, source))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, CapsuleError> {
    Ok(serde_json::from_slice(&read_file(path)?)?)
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), CapsuleError> {
    fs::write(path, bytes).map_err(|source| io_error(path, source))
}

fn io_error(path: &Path, source: std::io::Error) -> CapsuleError {
    CapsuleError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn verify_error(message: impl Into<String>) -> CapsuleError {
    CapsuleError::Verification(message.into())
}

fn strip_diff_runtime(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for key in [
                "id",
                "workflow_id",
                "command_id",
                "command_timestamp",
                "timestamp",
                "retrieved_at",
                "observed_at",
                "retrieval_events",
                "state_digest",
            ] {
                map.remove(key);
            }
            for child in map.values_mut() {
                strip_diff_runtime(child);
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                strip_diff_runtime(child);
            }
        }
        _ => {}
    }
}

fn diff_json(
    old: &serde_json::Value,
    new: &serde_json::Value,
    pointer: &str,
    role: &str,
    changes: &mut Vec<SemanticChange>,
) {
    match (old, new) {
        (serde_json::Value::Object(old), serde_json::Value::Object(new)) => {
            let keys = old.keys().chain(new.keys()).collect::<BTreeSet<_>>();
            for key in keys {
                let escaped = key.replace('~', "~0").replace('/', "~1");
                let path = format!("{pointer}/{escaped}");
                match (old.get(key), new.get(key)) {
                    (Some(old), Some(new)) => diff_json(old, new, &path, role, changes),
                    (Some(old), None) => push_leaf_change(
                        role,
                        path,
                        ChangeKind::Removed,
                        Some(old.clone()),
                        None,
                        changes,
                    ),
                    (None, Some(new)) => push_leaf_change(
                        role,
                        path,
                        ChangeKind::Added,
                        None,
                        Some(new.clone()),
                        changes,
                    ),
                    (None, None) => unreachable!(),
                }
            }
        }
        (serde_json::Value::Array(old), serde_json::Value::Array(new)) => {
            let length = old.len().max(new.len());
            for index in 0..length {
                let path = format!("{pointer}/{index}");
                match (old.get(index), new.get(index)) {
                    (Some(old), Some(new)) => diff_json(old, new, &path, role, changes),
                    (Some(old), None) => push_leaf_change(
                        role,
                        path,
                        ChangeKind::Removed,
                        Some(old.clone()),
                        None,
                        changes,
                    ),
                    (None, Some(new)) => push_leaf_change(
                        role,
                        path,
                        ChangeKind::Added,
                        None,
                        Some(new.clone()),
                        changes,
                    ),
                    (None, None) => unreachable!(),
                }
            }
        }
        _ if old != new => push_leaf_change(
            role,
            pointer.into(),
            ChangeKind::Modified,
            Some(old.clone()),
            Some(new.clone()),
            changes,
        ),
        _ => {}
    }
}

fn push_leaf_change(
    role: &str,
    path: String,
    kind: ChangeKind,
    old: Option<serde_json::Value>,
    new: Option<serde_json::Value>,
    changes: &mut Vec<SemanticChange>,
) {
    changes.push(SemanticChange {
        category: category_for(role, &path),
        subject_role: role.into(),
        path,
        kind,
        old,
        new,
    });
}

fn category_for(role: &str, path: &str) -> SemanticCategory {
    match role {
        "verification-policy" => SemanticCategory::Policy,
        "verification-graph" => SemanticCategory::Verification,
        "source-assurance" => SemanticCategory::Source,
        "map-artifact" => SemanticCategory::Artifact,
        "trust-report" => SemanticCategory::Verification,
        "command" | "workflow" if path.contains("contract") => SemanticCategory::Contract,
        "command" | "workflow" => SemanticCategory::Workflow,
        "analysis-result" if path.contains("source") || path.contains("citation") => {
            SemanticCategory::Source
        }
        "analysis-result" if path.contains("contract") => SemanticCategory::Contract,
        "analysis-result" => SemanticCategory::Result,
        "execution-receipt" if path.contains("verification_policy") => SemanticCategory::Policy,
        "execution-receipt" if path.contains("source") => SemanticCategory::Source,
        "execution-receipt"
            if path.contains("trust")
                || path.contains("check")
                || path.contains("verification_graph") =>
        {
            SemanticCategory::Verification
        }
        "execution-receipt" => SemanticCategory::Workflow,
        _ => SemanticCategory::Unclassified,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use genegis_analysis::run_ask_pipeline;

    #[test]
    fn seals_and_verifies_nagoya_offline() {
        let result = run_ask_pipeline("名古屋市の人口密度を表示").expect("north-star run");
        let direct_output = NagoyaExecutionOutput {
            analysis: result.analysis.clone().expect("typed analysis"),
            verification_passed: result.execution_receipt.verification_passed,
            html: result.html.clone(),
            png_base64: result.png_base64.clone(),
            artifact_digests: NagoyaArtifactDigests {
                html_sha256: Some(digest(result.html.as_bytes())),
                png_sha256: Some(digest(&result.png)),
                html_bytes: result.html.len(),
                png_bytes: result.png.len(),
            },
        };
        assert_eq!(
            canonical_nagoya_execution_digest(&direct_output, &result.execution_receipt.evidence),
            result.execution_receipt.result_digest,
            "ask result must retain every canonical execution subject"
        );
        let temporary = tempfile::tempdir().expect("temporary directory");
        let root = temporary.path().join("capsule");
        let manifest = seal_nagoya_capsule(&result, &root).expect("seal capsule");
        assert_eq!(manifest.entries.len(), 10);
        assert!(manifest.entries.iter().any(|entry| {
            entry.role == "source-assurance" && entry.path == "metadata/source-assurance/000.json"
        }));
        let round_tripped: AnalysisResult = read_json(&root.join(ANALYSIS_PATH)).unwrap();
        let original = result.analysis.as_ref().expect("typed analysis");
        assert_eq!(
            genegis_analysis::canonical_analysis_result_digest(&round_tripped),
            genegis_analysis::canonical_analysis_result_digest(original),
            "analysis JSON must preserve its canonical values"
        );

        let policy = result
            .execution_receipt
            .verification_policy
            .as_ref()
            .expect("release policy");
        let verified = verify_nagoya_capsule(&root, Some(policy)).expect("offline verification");
        assert_eq!(verified.trust.level, TrustLevel::Verified);
        assert_eq!(
            verified.result_digest,
            result.execution_receipt.result_digest
        );
        assert_eq!(verified.verified_entries, 10);
    }

    #[test]
    fn rejects_tampered_subject_manifest_and_external_policy() {
        let result = run_ask_pipeline("名古屋市の人口密度を表示").expect("north-star run");
        let temporary = tempfile::tempdir().expect("temporary directory");

        let artifact_root = temporary.path().join("artifact");
        seal_nagoya_capsule(&result, &artifact_root).expect("seal artifact case");
        fs::write(artifact_root.join(HTML_PATH), b"tampered").expect("tamper HTML");
        assert!(verify_nagoya_capsule(&artifact_root, None).is_err());

        let report_root = temporary.path().join("report");
        seal_nagoya_capsule(&result, &report_root).expect("seal report case");
        fs::write(
            report_root.join(TRUST_REPORT_PATH),
            b"<html>false verified claim</html>",
        )
        .expect("tamper report");
        refresh_manifest_entry(&report_root, TRUST_REPORT_PATH);
        assert!(verify_nagoya_capsule(&report_root, None).is_err());

        let assurance_root = temporary.path().join("assurance");
        seal_nagoya_capsule(&result, &assurance_root).expect("seal assurance case");
        mutate_json_subject(
            &assurance_root,
            "metadata/source-assurance/000.json",
            "/limitations/0",
            serde_json::json!("mutated limitation"),
        );
        assert!(verify_nagoya_capsule(&assurance_root, None).is_err());

        let manifest_root = temporary.path().join("manifest");
        seal_nagoya_capsule(&result, &manifest_root).expect("seal manifest case");
        let mut manifest: CapsuleManifest = read_json(&manifest_root.join("capsule.json")).unwrap();
        manifest.subject_result_digest = digest(b"unrelated result");
        fs::write(
            manifest_root.join("capsule.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        assert!(verify_nagoya_capsule(&manifest_root, None).is_err());

        let policy_root = temporary.path().join("policy");
        seal_nagoya_capsule(&result, &policy_root).expect("seal policy case");
        let mut stronger = result
            .execution_receipt
            .verification_policy
            .clone()
            .expect("policy");
        stronger.minimum_artifact_count += 1;
        assert!(verify_nagoya_capsule(&policy_root, Some(&stronger)).is_err());

        let path_root = temporary.path().join("path");
        seal_nagoya_capsule(&result, &path_root).expect("seal path case");
        let mut manifest: CapsuleManifest = read_json(&path_root.join("capsule.json")).unwrap();
        manifest.entries[0].path = "../escape".into();
        fs::write(
            path_root.join("capsule.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        assert!(verify_nagoya_capsule(&path_root, None).is_err());
    }

    #[test]
    fn semantic_diff_ignores_runtime_identity_and_classifies_known_changes() {
        let old_result = run_ask_pipeline("名古屋市の人口密度を表示").expect("old run");
        let new_result = run_ask_pipeline("名古屋市の人口密度を表示").expect("new run");
        let temporary = tempfile::tempdir().expect("temporary directory");
        let old_root = temporary.path().join("old");
        let new_root = temporary.path().join("new");
        seal_nagoya_capsule(&old_result, &old_root).expect("old capsule");
        seal_nagoya_capsule(&new_result, &new_root).expect("new capsule");
        let equivalent = diff_capsules(&old_root, &new_root).expect("equivalent diff");
        assert!(
            equivalent.changes.is_empty(),
            "runtime-only diff: {:?}",
            equivalent.changes
        );

        mutate_json_subject(
            &new_root,
            ANALYSIS_PATH,
            "/features/0/population",
            serde_json::json!(123),
        );
        mutate_json_subject(
            &new_root,
            WORKFLOW_PATH,
            "/input_contracts/0/geo_contract/temporal/reference_period",
            serde_json::json!("2021"),
        );
        let changed = diff_capsules(&old_root, &new_root).expect("semantic diff");
        assert_eq!(changed.unclassified_changes, 0);
        assert!(changed.changes.iter().any(|change| {
            change.category == SemanticCategory::Result && change.path.ends_with("/population")
        }));
        assert!(changed.changes.iter().any(|change| {
            change.category == SemanticCategory::Contract
                && change.path.ends_with("/reference_period")
        }));
        let review = review_capsule_with_diff(&old_root, None, Some(changed.clone()))
            .expect("review with integrated diff");
        assert_eq!(review.semantic_diff, Some(changed));
    }

    #[test]
    fn approval_is_invalidated_by_capsule_or_diff_mutation() {
        let result = run_ask_pipeline("名古屋市の人口密度を表示").expect("north-star run");
        let temporary = tempfile::tempdir().expect("temporary directory");
        let root = temporary.path().join("capsule");
        seal_nagoya_capsule(&result, &root).expect("capsule");
        let empty_diff = diff_capsules(&root, &root).expect("empty diff");
        let approval = create_approval(
            &root,
            "reviewer@example.org",
            "2026-08-23T12:00:00+09:00",
            Some(&empty_diff),
        )
        .expect("approval");
        verify_approval(&root, &approval, Some(&empty_diff)).expect("current approval");

        let mut changed_diff = empty_diff.clone();
        changed_diff.new_result_digest = digest(b"changed diff subject");
        assert!(verify_approval(&root, &approval, Some(&changed_diff)).is_err());

        let mut html = read_file(&root.join(HTML_PATH)).unwrap();
        html.push(b' ');
        write_file(&root.join(HTML_PATH), &html).unwrap();
        refresh_manifest_entry(&root, HTML_PATH);
        assert!(verify_approval(&root, &approval, Some(&empty_diff)).is_err());
    }

    #[test]
    fn review_model_exposes_claims_contracts_artifacts_and_failures() {
        let result = run_ask_pipeline("名古屋市の人口密度を表示").expect("north-star run");
        let temporary = tempfile::tempdir().expect("temporary directory");
        let root = temporary.path().join("capsule");
        seal_nagoya_capsule(&result, &root).expect("capsule");
        let review = review_capsule(&root, None).expect("review model");
        assert_eq!(review.claims.len(), 5);
        assert_eq!(review.contracts.len(), 3);
        assert_eq!(review.artifacts.len(), 10);
        assert!(!review.sources.is_empty());
        assert_eq!(
            review.sources[0].assurance_level.as_deref(),
            Some("corroborated")
        );
        assert!(review.sources[0].assurance_digest.is_some());
        assert!(review.sources[0].limitations.len() >= 3);
        assert!(review.workflow_nodes.len() >= 10);
        assert_eq!(
            review.identities.result_digest,
            result.execution_receipt.result_digest
        );
        let trust_html = String::from_utf8(read_file(&root.join(TRUST_REPORT_PATH)).unwrap())
            .expect("trust report UTF-8");
        for identity in [
            &review.identities.result_digest,
            &review.identities.workflow_digest,
            &review.identities.policy_digest,
            &review.identities.verification_graph_digest,
        ] {
            assert!(trust_html.contains(identity));
        }
        assert!(review.integrity_error.is_none());
        assert_eq!(
            review.verification.as_ref().map(|value| value.trust.level),
            Some(TrustLevel::Verified)
        );

        let mut html = read_file(&root.join(HTML_PATH)).unwrap();
        html.push(b'!');
        write_file(&root.join(HTML_PATH), &html).unwrap();
        let broken = review_capsule(&root, None).expect("broken review remains inspectable");
        assert!(broken.verification.is_none());
        assert!(broken.integrity_error.is_some());
        assert_eq!(broken.claims.len(), 5);

        let failure = actionable_failure(
            &genegis_contract::TrustFailure {
                gate: genegis_contract::TrustGate::Verification,
                code: "check_failed".into(),
                subject: "density_oracle".into(),
                detail: "seeded density mismatch".into(),
            },
            &result.workflow,
        );
        assert_eq!(failure.affected_nodes, vec!["calculate-density"]);
        assert!(failure.remediation.contains("rerun"));
    }

    #[test]
    fn mutation_harness_has_zero_false_verified() {
        let result = run_ask_pipeline("名古屋市の人口密度を表示").expect("north-star run");
        let policy = result
            .execution_receipt
            .verification_policy
            .clone()
            .expect("policy");
        let temporary = tempfile::tempdir().expect("temporary directory");
        let cases = vec![
            (
                "population",
                ANALYSIS_PATH,
                "/features/0/population",
                serde_json::json!(1),
            ),
            (
                "area",
                ANALYSIS_PATH,
                "/features/0/area_km2",
                serde_json::json!(1.0),
            ),
            (
                "density",
                ANALYSIS_PATH,
                "/features/0/density_per_km2",
                serde_json::json!(1.0),
            ),
            (
                "ward-code",
                ANALYSIS_PATH,
                "/features/0/ward_code",
                serde_json::json!("99999"),
            ),
            (
                "ward-name",
                ANALYSIS_PATH,
                "/features/0/ward_name",
                serde_json::json!("mutation"),
            ),
            (
                "geometry",
                ANALYSIS_PATH,
                "/features/0/rings/0/coords/0/0",
                serde_json::json!(0.0),
            ),
            (
                "render-color",
                ANALYSIS_PATH,
                "/features/0/color/r",
                serde_json::json!(0.0),
            ),
            (
                "citation",
                ANALYSIS_PATH,
                "/citations/0/url",
                serde_json::json!("https://invalid.example"),
            ),
            (
                "result-crs",
                ANALYSIS_PATH,
                "/verification/crs",
                serde_json::json!("EPSG:3857"),
            ),
            (
                "result-unit",
                ANALYSIS_PATH,
                "/verification/density_unit",
                serde_json::json!("thousands/km2"),
            ),
            (
                "dag-parameter",
                WORKFLOW_PATH,
                "/steps/0/parameters/name",
                serde_json::json!("大阪市"),
            ),
            (
                "dag-edge",
                WORKFLOW_PATH,
                "/steps/1/depends_on",
                serde_json::json!([]),
            ),
            (
                "contract-time",
                WORKFLOW_PATH,
                "/input_contracts/0/geo_contract/temporal/reference_period",
                serde_json::json!("2021"),
            ),
            (
                "command-workflow",
                COMMAND_PATH,
                "/command/workflow_id",
                serde_json::json!("00000000-0000-0000-0000-000000000000"),
            ),
            (
                "command-digest",
                COMMAND_PATH,
                "/workflow_digest",
                serde_json::json!(digest(b"mutated workflow")),
            ),
            (
                "policy-id",
                POLICY_PATH,
                "/policy_id",
                serde_json::json!("weakened"),
            ),
            (
                "policy-artifacts",
                POLICY_PATH,
                "/minimum_artifact_count",
                serde_json::json!(0),
            ),
            (
                "graph-independence",
                GRAPH_PATH,
                "/nodes/0/verifier/independence",
                serde_json::json!("same_implementation"),
            ),
            (
                "graph-tolerance",
                GRAPH_PATH,
                "/nodes/3/tolerance/max_error_ppm",
                serde_json::json!(999999),
            ),
            (
                "stored-trust",
                RECEIPT_PATH,
                "/trust_assessment/level",
                serde_json::json!("attested"),
            ),
            (
                "source-checksum",
                RECEIPT_PATH,
                "/trust_evidence/sources/0/checksum_status",
                serde_json::json!("mismatch"),
            ),
            (
                "source-assurance-digest",
                RECEIPT_PATH,
                "/trust_evidence/sources/0/assurance_digest",
                serde_json::json!(digest(b"other assurance")),
            ),
            (
                "source-assurance-snapshot",
                RECEIPT_PATH,
                "/trust_evidence/sources/0/snapshot_digest",
                serde_json::json!(digest(b"other source snapshot")),
            ),
            (
                "check-outcome",
                RECEIPT_PATH,
                "/trust_evidence/checks/1/passed",
                serde_json::json!(false),
            ),
            (
                "replay-backend",
                RECEIPT_PATH,
                "/trust_evidence/replay/backend_identity",
                serde_json::json!("other:engine:1"),
            ),
            (
                "receipt-engine",
                RECEIPT_PATH,
                "/engine/version",
                serde_json::json!("9.9.9"),
            ),
            (
                "receipt-graph",
                RECEIPT_PATH,
                "/verification_graph_digest",
                serde_json::json!(digest(b"other graph")),
            ),
            (
                "receipt-result",
                RECEIPT_PATH,
                "/result_digest",
                serde_json::json!(digest(b"other result")),
            ),
            (
                "legacy-flag",
                RECEIPT_PATH,
                "/verification_passed",
                serde_json::json!(false),
            ),
        ];
        let mut caught = Vec::new();
        let mut false_verified = Vec::new();
        for (index, (name, path, pointer, replacement)) in cases.iter().enumerate() {
            let root = temporary.path().join(format!("mutation-{index:02}"));
            seal_nagoya_capsule(&result, &root).expect("seal baseline");
            mutate_json_subject(&root, path, pointer, replacement.clone());
            record_mutation_result(
                name,
                verify_nagoya_capsule(&root, Some(&policy)),
                &mut caught,
                &mut false_verified,
            );
        }
        for (index, (name, path)) in [("html-bytes", HTML_PATH), ("png-bytes", PNG_PATH)]
            .iter()
            .enumerate()
        {
            let root = temporary.path().join(format!("artifact-mutation-{index}"));
            seal_nagoya_capsule(&result, &root).expect("seal artifact baseline");
            let mut bytes = read_file(&root.join(path)).unwrap();
            bytes[0] ^= 0x01;
            write_file(&root.join(path), &bytes).unwrap();
            refresh_manifest_entry(&root, path);
            record_mutation_result(
                name,
                verify_nagoya_capsule(&root, Some(&policy)),
                &mut caught,
                &mut false_verified,
            );
        }
        let total = cases.len() + 2;
        let score = caught.len() as f64 / total as f64;
        assert!(total >= 20);
        assert!(
            false_verified.is_empty(),
            "false verified: {false_verified:?}"
        );
        assert!(
            score >= 0.95,
            "mutation score {score:.3}; caught {caught:?}"
        );
    }

    fn record_mutation_result<'a>(
        name: &'a str,
        result: Result<CapsuleVerification, CapsuleError>,
        caught: &mut Vec<&'a str>,
        false_verified: &mut Vec<&'a str>,
    ) {
        match result {
            Err(_) => caught.push(name),
            Ok(report) if report.trust.level >= TrustLevel::Verified => false_verified.push(name),
            Ok(_) => caught.push(name),
        }
    }

    fn mutate_json_subject(
        root: &Path,
        relative: &str,
        pointer: &str,
        replacement: serde_json::Value,
    ) {
        let path = root.join(relative);
        let mut document: serde_json::Value = read_json(&path).expect("mutation JSON");
        *document.pointer_mut(pointer).expect("mutation pointer") = replacement;
        write_file(&path, &serde_json::to_vec_pretty(&document).unwrap()).unwrap();
        refresh_manifest_entry(root, relative);
    }

    fn refresh_manifest_entry(root: &Path, relative: &str) {
        let bytes = read_file(&root.join(relative)).unwrap();
        let manifest_path = root.join("capsule.json");
        let mut manifest: CapsuleManifest = read_json(&manifest_path).unwrap();
        let entry = manifest
            .entries
            .iter_mut()
            .find(|entry| entry.path == relative)
            .unwrap();
        entry.sha256 = digest(&bytes);
        entry.bytes = bytes.len() as u64;
        write_file(
            &manifest_path,
            &serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }
}
