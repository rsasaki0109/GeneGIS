use crate::{AssurancePolicy, CompatibilityStatus, SourceAssurance, GEO_CONTRACT_SCHEMA_VERSION};
use genegis_crs::ChecksumVerification;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Schema version for [`VerificationPolicy`].
pub const VERIFICATION_POLICY_SCHEMA_VERSION: &str = "0.1.0";

fn default_policy_schema_version() -> String {
    VERIFICATION_POLICY_SCHEMA_VERSION.to_string()
}

/// Monotonic trust state derived from policy and evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    /// Output may be inspected, but required replay/verification evidence is incomplete.
    Exploratory,
    /// Inputs, workflow, backend, and result artifacts have stable identities.
    Replayable,
    /// Replay evidence plus every required semantic and independent check passed.
    Verified,
    /// A recognized signer attested the verified evidence and artifact subjects.
    Attested,
}

/// Relationship between a verifier and the executor/claim being checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndependenceClass {
    /// The executor implementation checked its own output.
    SameImplementation,
    /// A different algorithm in the same engine checked the output.
    DifferentAlgorithmSameEngine,
    /// A separately implemented engine checked the output.
    DifferentEngine,
    /// An authoritative external publication supplied an oracle value.
    AuthoritativeExternalOracle,
    /// A domain invariant or conservation rule checked the result.
    DomainInvariant,
}

/// Requirement for one independently checked release claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckRequirement {
    /// Stable check identifier.
    pub check_id: String,
    /// Accepted verifier/executor relationships.
    pub accepted_independence: BTreeSet<IndependenceClass>,
    /// Maximum accepted numeric error, when the check reports one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_error_ppm: Option<u64>,
}

/// Versioned policy used to derive a trust level.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationPolicy {
    /// Policy document schema version.
    #[serde(default = "default_policy_schema_version")]
    pub schema_version: String,
    /// Stable policy identifier/version chosen by a reviewer or organization.
    pub policy_id: String,
    /// GeoContract schema accepted by the policy.
    pub geo_contract_schema_version: String,
    /// Contract identifiers that must be valid and compatible.
    #[serde(default)]
    pub required_contracts: BTreeSet<String>,
    /// Required independent verification claims.
    #[serde(default)]
    pub required_checks: Vec<CheckRequirement>,
    /// Require every admitted source checksum to have been observed and verified.
    #[serde(default)]
    pub require_verified_sources: bool,
    /// Require a stable source version for every admitted source.
    #[serde(default)]
    pub require_source_version: bool,
    /// Require a declared license for every admitted source.
    #[serde(default)]
    pub require_license: bool,
    /// Optional use-case policy for authority, freshness, corroboration, and limitations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_assurance: Option<AssurancePolicy>,
    /// Minimum number of content-addressed result artifacts.
    #[serde(default)]
    pub minimum_artifact_count: u32,
    /// Signers accepted for the `attested` level.
    #[serde(default)]
    pub accepted_attesters: BTreeSet<String>,
}

impl VerificationPolicy {
    /// Construct an empty fail-closed policy for the current schema versions.
    pub fn new(policy_id: impl Into<String>) -> Self {
        Self {
            schema_version: default_policy_schema_version(),
            policy_id: policy_id.into(),
            geo_contract_schema_version: GEO_CONTRACT_SCHEMA_VERSION.into(),
            required_contracts: BTreeSet::new(),
            required_checks: Vec::new(),
            require_verified_sources: true,
            require_source_version: true,
            require_license: true,
            source_assurance: None,
            minimum_artifact_count: 1,
            accepted_attesters: BTreeSet::new(),
        }
    }

    /// Derive the highest trust level justified by the supplied evidence.
    pub fn assess(&self, evidence: &TrustEvidence) -> TrustAssessment {
        assess(self, evidence)
    }
}

/// Identity evidence required for deterministic replay.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayEvidence {
    /// Stable canonical workflow digest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_digest: Option<String>,
    /// Digest of actual output values and verification evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_digest: Option<String>,
    /// Backend/build/container identity that executed the workflow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend_identity: Option<String>,
    /// Content digests of released artifacts.
    #[serde(default)]
    pub artifact_digests: Vec<String>,
}

/// Validation result for one required GeoContract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractEvidence {
    /// Stable contract identifier.
    pub contract_id: String,
    /// Schema version of the observed contract.
    pub schema_version: String,
    /// Whether structural and semantic validation passed.
    pub valid: bool,
    /// Directional provider-to-requirement compatibility result.
    pub compatibility: CompatibilityStatus,
}

/// Admission evidence for one input source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceEvidence {
    /// Stable source identifier used in failure messages.
    pub source_id: String,
    /// Result of comparing expected and observed bytes.
    pub checksum_status: ChecksumVerification,
    /// Whether a stable release/version is present.
    pub source_version_present: bool,
    /// Whether license/usage terms are present.
    pub license_present: bool,
    /// Digest of the exact source snapshot represented by this evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_digest: Option<String>,
    /// Evidence dossier assessing authority, freshness, checks, and known limits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assurance: Option<SourceAssurance>,
    /// Digest of the complete assurance dossier, binding it independently of
    /// the enclosing receipt serialization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assurance_digest: Option<String>,
}

/// Outcome and provenance for one verification claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationEvidence {
    /// Stable check identifier.
    pub check_id: String,
    /// Whether the check passed.
    pub passed: bool,
    /// Relationship between verifier and executor.
    pub independence: IndependenceClass,
    /// Observed error in parts per million, when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_error_ppm: Option<u64>,
    /// Content digest of the check inputs/output or oracle document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_digest: Option<String>,
}

/// Verified signature/issuer information for an analysis attestation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttestationEvidence {
    /// Claimed signer identity.
    pub signer: String,
    /// Whether signature and subject integrity verification passed.
    pub signature_verified: bool,
}

/// Evidence supplied to a [`VerificationPolicy`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustEvidence {
    /// Replay and artifact identities.
    pub replay: ReplayEvidence,
    /// Contract validation/compatibility observations.
    #[serde(default)]
    pub contracts: Vec<ContractEvidence>,
    /// Input source admission observations.
    #[serde(default)]
    pub sources: Vec<SourceEvidence>,
    /// Independent verification observations.
    #[serde(default)]
    pub checks: Vec<VerificationEvidence>,
    /// Optional signed attestation evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attestation: Option<AttestationEvidence>,
}

/// Gate at which trust derivation stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustGate {
    /// Stable replay identity requirements.
    Replay,
    /// Contract/source/independent verification requirements.
    Verification,
    /// Signature and signer policy requirements.
    Attestation,
}

/// One structured reason why a higher trust level was not reached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustFailure {
    /// Gate that emitted the failure.
    pub gate: TrustGate,
    /// Stable machine-readable failure code.
    pub code: String,
    /// Affected contract, source, check, artifact, or signer.
    pub subject: String,
    /// Human-readable failure detail.
    pub detail: String,
}

/// Policy-derived trust result with actionable failures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustAssessment {
    /// Highest trust level justified by evidence.
    pub level: TrustLevel,
    /// Policy identifier used for derivation.
    pub policy_id: String,
    /// Structured reasons why the next level was not reached.
    pub failures: Vec<TrustFailure>,
}

fn assess(policy: &VerificationPolicy, evidence: &TrustEvidence) -> TrustAssessment {
    let mut failures = validate_policy(policy);
    failures.extend(replay_failures(policy, &evidence.replay));
    if !failures.is_empty() {
        return TrustAssessment {
            level: TrustLevel::Exploratory,
            policy_id: policy.policy_id.clone(),
            failures,
        };
    }

    failures.extend(verification_failures(policy, evidence));
    if !failures.is_empty() {
        return TrustAssessment {
            level: TrustLevel::Replayable,
            policy_id: policy.policy_id.clone(),
            failures,
        };
    }

    let attestation_failures = attestation_failures(policy, evidence.attestation.as_ref());
    if !attestation_failures.is_empty() {
        return TrustAssessment {
            level: TrustLevel::Verified,
            policy_id: policy.policy_id.clone(),
            failures: attestation_failures,
        };
    }

    TrustAssessment {
        level: TrustLevel::Attested,
        policy_id: policy.policy_id.clone(),
        failures: Vec::new(),
    }
}

fn validate_policy(policy: &VerificationPolicy) -> Vec<TrustFailure> {
    let mut failures = Vec::new();
    if policy.schema_version != VERIFICATION_POLICY_SCHEMA_VERSION {
        failures.push(failure(
            TrustGate::Replay,
            "unsupported_policy_schema",
            &policy.schema_version,
            "verification policy schema version is not supported",
        ));
    }
    if policy.policy_id.trim().is_empty() {
        failures.push(failure(
            TrustGate::Replay,
            "missing_policy_id",
            "policy",
            "policy identifier is empty",
        ));
    }
    if policy.geo_contract_schema_version != GEO_CONTRACT_SCHEMA_VERSION {
        failures.push(failure(
            TrustGate::Verification,
            "unsupported_contract_schema",
            &policy.geo_contract_schema_version,
            "policy requires an unsupported GeoContract schema",
        ));
    }
    let mut check_ids = BTreeSet::new();
    for check in &policy.required_checks {
        if check.check_id.trim().is_empty() || !check_ids.insert(&check.check_id) {
            failures.push(failure(
                TrustGate::Verification,
                "invalid_check_requirement",
                &check.check_id,
                "required check identifier is empty or duplicated",
            ));
        }
        if check.accepted_independence.is_empty() {
            failures.push(failure(
                TrustGate::Verification,
                "missing_independence_policy",
                &check.check_id,
                "required check has no accepted independence class",
            ));
        }
    }
    failures
}

fn replay_failures(policy: &VerificationPolicy, replay: &ReplayEvidence) -> Vec<TrustFailure> {
    let mut failures = Vec::new();
    for (code, subject, value) in [
        (
            "missing_workflow_digest",
            "workflow",
            replay.workflow_digest.as_deref(),
        ),
        (
            "missing_result_digest",
            "result",
            replay.result_digest.as_deref(),
        ),
        (
            "missing_backend_identity",
            "backend",
            replay.backend_identity.as_deref(),
        ),
    ] {
        if !value.is_some_and(valid_identity) {
            failures.push(failure(
                TrustGate::Replay,
                code,
                subject,
                "required stable replay identity is missing or malformed",
            ));
        }
    }
    let valid_artifacts = replay
        .artifact_digests
        .iter()
        .filter(|digest| valid_digest(digest))
        .count();
    if valid_artifacts < policy.minimum_artifact_count as usize {
        failures.push(failure(
            TrustGate::Replay,
            "insufficient_artifact_digests",
            "artifacts",
            &format!(
                "{} valid artifact digests; policy requires {}",
                valid_artifacts, policy.minimum_artifact_count
            ),
        ));
    }
    failures
}

fn verification_failures(
    policy: &VerificationPolicy,
    evidence: &TrustEvidence,
) -> Vec<TrustFailure> {
    let mut failures = Vec::new();
    for required in &policy.required_contracts {
        match evidence
            .contracts
            .iter()
            .find(|contract| &contract.contract_id == required)
        {
            None => failures.push(failure(
                TrustGate::Verification,
                "missing_contract_evidence",
                required,
                "required GeoContract evidence is missing",
            )),
            Some(contract)
                if !contract.valid
                    || contract.schema_version != policy.geo_contract_schema_version
                    || contract.compatibility != CompatibilityStatus::Compatible =>
            {
                failures.push(failure(
                    TrustGate::Verification,
                    "contract_not_compatible",
                    required,
                    "required GeoContract is invalid, unsupported, or not fully compatible",
                ));
            }
            Some(_) => {}
        }
    }

    for source in &evidence.sources {
        if policy.require_verified_sources
            && source.checksum_status != ChecksumVerification::Verified
        {
            failures.push(failure(
                TrustGate::Verification,
                "source_not_verified",
                &source.source_id,
                "source bytes were not verified against their snapshot identity",
            ));
        }
        if policy.require_source_version && !source.source_version_present {
            failures.push(failure(
                TrustGate::Verification,
                "missing_source_version",
                &source.source_id,
                "stable source version is missing",
            ));
        }
        if policy.require_license && !source.license_present {
            failures.push(failure(
                TrustGate::Verification,
                "missing_source_license",
                &source.source_id,
                "source license is missing",
            ));
        }
        if let Some(assurance) = source.assurance.as_ref() {
            if source.snapshot_digest.as_deref() != Some(assurance.snapshot_digest.as_str()) {
                failures.push(failure(
                    TrustGate::Verification,
                    "source_assurance_snapshot_mismatch",
                    &source.source_id,
                    "source assurance snapshot digest differs from source admission evidence",
                ));
            }
            match assurance.digest() {
                Ok(digest) if source.assurance_digest.as_deref() == Some(digest.as_str()) => {}
                _ => failures.push(failure(
                    TrustGate::Verification,
                    "source_assurance_digest_mismatch",
                    &source.source_id,
                    "source assurance dossier is not bound by its computed digest",
                )),
            }
        }
        if let Some(assurance_policy) = policy.source_assurance.as_ref() {
            match source.assurance.as_ref() {
                None => failures.push(failure(
                    TrustGate::Verification,
                    "missing_source_assurance",
                    &source.source_id,
                    "policy requires a source assurance dossier",
                )),
                Some(assurance) if assurance.source_id != source.source_id => {
                    failures.push(failure(
                        TrustGate::Verification,
                        "source_assurance_identity_mismatch",
                        &source.source_id,
                        "source assurance dossier identifies a different source",
                    ));
                }
                Some(assurance) => {
                    let report = assurance_policy.assess(assurance);
                    if !report.passed {
                        failures.extend(report.failures.into_iter().map(|assurance_failure| {
                            failure(
                                TrustGate::Verification,
                                &format!("source_assurance_{}", assurance_failure.code),
                                &assurance_failure.subject,
                                &assurance_failure.detail,
                            )
                        }));
                    }
                }
            }
        }
    }
    if (policy.require_verified_sources || policy.require_source_version || policy.require_license)
        && evidence.sources.is_empty()
    {
        failures.push(failure(
            TrustGate::Verification,
            "missing_source_evidence",
            "sources",
            "policy requires source evidence but none was supplied",
        ));
    }

    for requirement in &policy.required_checks {
        match evidence
            .checks
            .iter()
            .find(|check| check.check_id == requirement.check_id)
        {
            None => failures.push(failure(
                TrustGate::Verification,
                "missing_check_evidence",
                &requirement.check_id,
                "required verification check is missing",
            )),
            Some(check) if !check.passed => failures.push(failure(
                TrustGate::Verification,
                "check_failed",
                &requirement.check_id,
                "required verification check failed",
            )),
            Some(check)
                if !requirement
                    .accepted_independence
                    .contains(&check.independence) =>
            {
                failures.push(failure(
                    TrustGate::Verification,
                    "insufficient_verifier_independence",
                    &requirement.check_id,
                    "verifier independence class is not accepted by policy",
                ));
            }
            Some(check) if !check.evidence_digest.as_deref().is_some_and(valid_digest) => {
                failures.push(failure(
                    TrustGate::Verification,
                    "missing_check_digest",
                    &requirement.check_id,
                    "check evidence is not content-addressed",
                ));
            }
            Some(check) => {
                if let Some(maximum) = requirement.max_error_ppm {
                    match check.observed_error_ppm {
                        Some(observed) if observed <= maximum => {}
                        Some(observed) => failures.push(failure(
                            TrustGate::Verification,
                            "check_tolerance_exceeded",
                            &requirement.check_id,
                            &format!("observed {observed} ppm exceeds {maximum} ppm"),
                        )),
                        None => failures.push(failure(
                            TrustGate::Verification,
                            "missing_check_error",
                            &requirement.check_id,
                            "policy requires a numeric error observation",
                        )),
                    }
                }
            }
        }
    }
    failures
}

fn attestation_failures(
    policy: &VerificationPolicy,
    attestation: Option<&AttestationEvidence>,
) -> Vec<TrustFailure> {
    if policy.accepted_attesters.is_empty() {
        return vec![failure(
            TrustGate::Attestation,
            "attestation_not_required",
            "attestation",
            "policy does not identify an accepted attester",
        )];
    }
    match attestation {
        None => vec![failure(
            TrustGate::Attestation,
            "missing_attestation",
            "attestation",
            "no signed attestation was supplied",
        )],
        Some(attestation) if !attestation.signature_verified => vec![failure(
            TrustGate::Attestation,
            "invalid_attestation_signature",
            &attestation.signer,
            "attestation signature or subject integrity did not verify",
        )],
        Some(attestation) if !policy.accepted_attesters.contains(&attestation.signer) => {
            vec![failure(
                TrustGate::Attestation,
                "unaccepted_attester",
                &attestation.signer,
                "attestation signer is not accepted by policy",
            )]
        }
        Some(_) => Vec::new(),
    }
}

fn valid_identity(value: &str) -> bool {
    !value.trim().is_empty() && (valid_digest(value) || value.contains(':'))
}

fn valid_digest(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn failure(gate: TrustGate, code: &str, subject: &str, detail: &str) -> TrustFailure {
    TrustFailure {
        gate,
        code: code.into(),
        subject: subject.into(),
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn policy() -> VerificationPolicy {
        let mut policy = VerificationPolicy::new("nagoya-release-v1");
        policy.required_contracts.insert("nagoya-density".into());
        policy.required_checks.push(CheckRequirement {
            check_id: "area-oracle".into(),
            accepted_independence: BTreeSet::from([
                IndependenceClass::AuthoritativeExternalOracle,
                IndependenceClass::DifferentEngine,
            ]),
            max_error_ppm: Some(5_000),
        });
        policy
    }

    fn evidence() -> TrustEvidence {
        TrustEvidence {
            replay: ReplayEvidence {
                workflow_digest: Some(DIGEST.into()),
                result_digest: Some(DIGEST.into()),
                backend_identity: Some("genegis:native:0.1.0".into()),
                artifact_digests: vec![DIGEST.into()],
            },
            contracts: vec![ContractEvidence {
                contract_id: "nagoya-density".into(),
                schema_version: GEO_CONTRACT_SCHEMA_VERSION.into(),
                valid: true,
                compatibility: CompatibilityStatus::Compatible,
            }],
            sources: vec![SourceEvidence {
                source_id: "nagoya-2020".into(),
                checksum_status: ChecksumVerification::Verified,
                source_version_present: true,
                license_present: true,
                snapshot_digest: Some(DIGEST.into()),
                assurance: None,
                assurance_digest: None,
            }],
            checks: vec![VerificationEvidence {
                check_id: "area-oracle".into(),
                passed: true,
                independence: IndependenceClass::AuthoritativeExternalOracle,
                observed_error_ppm: Some(100),
                evidence_digest: Some(DIGEST.into()),
            }],
            attestation: None,
        }
    }

    #[test]
    fn derives_verified_then_attested_monotonically() {
        let mut policy = policy();
        let mut evidence = evidence();
        let assessment = policy.assess(&evidence);
        assert_eq!(assessment.level, TrustLevel::Verified);
        assert_eq!(assessment.failures[0].code, "attestation_not_required");

        policy.accepted_attesters.insert("city-review-board".into());
        assert_eq!(policy.assess(&evidence).level, TrustLevel::Verified);
        evidence.attestation = Some(AttestationEvidence {
            signer: "city-review-board".into(),
            signature_verified: true,
        });
        assert_eq!(policy.assess(&evidence).level, TrustLevel::Attested);
    }

    #[test]
    fn missing_replay_identity_stays_exploratory() {
        let mut evidence = evidence();
        evidence.replay.workflow_digest = None;
        let assessment = policy().assess(&evidence);
        assert_eq!(assessment.level, TrustLevel::Exploratory);
        assert!(assessment
            .failures
            .iter()
            .any(|failure| failure.code == "missing_workflow_digest"));
    }

    #[test]
    fn semantic_or_source_failure_stays_replayable() {
        let mut missing_contract = evidence();
        missing_contract.contracts.clear();
        assert_eq!(
            policy().assess(&missing_contract).level,
            TrustLevel::Replayable
        );

        let mut unknown_source = evidence();
        unknown_source.sources[0].checksum_status = ChecksumVerification::Declared;
        assert_eq!(
            policy().assess(&unknown_source).level,
            TrustLevel::Replayable
        );

        let mut incompatible = evidence();
        incompatible.contracts[0].compatibility = CompatibilityStatus::Indeterminate;
        assert_eq!(policy().assess(&incompatible).level, TrustLevel::Replayable);
    }

    #[test]
    fn required_source_assurance_is_part_of_verified_trust() {
        let mut policy = policy();
        policy.source_assurance = Some(AssurancePolicy {
            accepted_authority_classes: BTreeSet::from([crate::AuthorityClass::PrimaryOfficial]),
            max_age_days: Some(365),
            required_checks: BTreeSet::new(),
            minimum_independent_corroborations: 0,
            require_uncertainty: false,
            require_limitations: true,
            allow_unresolved_disputes: false,
        });

        let missing = policy.assess(&evidence());
        assert_eq!(missing.level, TrustLevel::Replayable);
        assert!(missing
            .failures
            .iter()
            .any(|failure| failure.code == "missing_source_assurance"));

        let mut admitted = evidence();
        let mut assurance = SourceAssurance::new(
            "nagoya-2020",
            DIGEST,
            "Nagoya City",
            crate::AuthorityClass::PrimaryOfficial,
        );
        assurance.published_at = Some("2021-11-30".into());
        assurance.assessed_at = "2022-01-01T00:00:00Z".into();
        assurance.observed_age_days = Some(32);
        assurance.limitations = vec!["Reference-date population, not a live estimate.".into()];
        admitted.sources[0].assurance_digest = Some(assurance.digest().unwrap());
        admitted.sources[0].assurance = Some(assurance);
        assert_eq!(policy.assess(&admitted).level, TrustLevel::Verified);
    }

    #[test]
    fn self_verification_and_tolerance_failure_never_verify() {
        let mut self_checked = evidence();
        self_checked.checks[0].independence = IndependenceClass::SameImplementation;
        let assessment = policy().assess(&self_checked);
        assert_eq!(assessment.level, TrustLevel::Replayable);
        assert!(assessment
            .failures
            .iter()
            .any(|failure| failure.code == "insufficient_verifier_independence"));

        let mut outside_tolerance = evidence();
        outside_tolerance.checks[0].observed_error_ppm = Some(5_001);
        assert_eq!(
            policy().assess(&outside_tolerance).level,
            TrustLevel::Replayable
        );
    }

    #[test]
    fn invalid_signature_or_signer_cannot_attest() {
        let mut policy = policy();
        policy.accepted_attesters.insert("trusted".into());
        let mut evidence = evidence();
        evidence.attestation = Some(AttestationEvidence {
            signer: "untrusted".into(),
            signature_verified: true,
        });
        assert_eq!(policy.assess(&evidence).level, TrustLevel::Verified);

        evidence.attestation = Some(AttestationEvidence {
            signer: "trusted".into(),
            signature_verified: false,
        });
        assert_eq!(policy.assess(&evidence).level, TrustLevel::Verified);
    }

    #[test]
    fn malformed_policy_fails_closed_and_round_trips() {
        let mut malformed = policy();
        malformed.schema_version = "999".into();
        assert_eq!(malformed.assess(&evidence()).level, TrustLevel::Exploratory);

        let json = serde_json::to_value(policy()).expect("serialize policy");
        let decoded: VerificationPolicy = serde_json::from_value(json).expect("decode policy");
        assert_eq!(decoded, policy());
    }
}
