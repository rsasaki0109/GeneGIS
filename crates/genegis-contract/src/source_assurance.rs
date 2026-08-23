//! Evidence and policy for assessing the reliability limits of source data.
//!
//! Source assurance does not assert that a dataset is universally true. It
//! records which snapshot was assessed, who published it, which checks and
//! independent comparisons passed, and which limitations or disputes remain.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

/// Schema version emitted for [`SourceAssurance`].
pub const SOURCE_ASSURANCE_SCHEMA_VERSION: &str = "0.1.0";

fn default_schema_version() -> String {
    SOURCE_ASSURANCE_SCHEMA_VERSION.to_string()
}

/// Institutional relationship between a publisher and the represented data.
///
/// The variants are classifications rather than an implied universal ranking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityClass {
    /// The legally or administratively responsible primary publisher.
    PrimaryOfficial,
    /// A publisher formally delegated by the primary authority.
    DelegatedOfficial,
    /// A peer-reviewed research publication or maintained research repository.
    PeerReviewed,
    /// A community-maintained source with inspectable governance.
    CommunityMaintained,
    /// A commercial publisher operating under documented terms.
    Commercial,
    /// The authority relationship is not known.
    Unknown,
}

/// Kind of machine- or human-produced source-quality check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssuranceCheckKind {
    /// File or record structure conforms to the declared schema.
    Schema,
    /// Expected entities, cells, points, or time periods are present.
    Completeness,
    /// Spatial extent, coverage, or topology agrees with its contract.
    SpatialCoverage,
    /// Observation and release times are internally consistent.
    TemporalConsistency,
    /// Statistical or geometric anomaly detection was performed.
    AnomalyDetection,
    /// Values were compared with a separately identified source.
    CrossSourceAgreement,
}

/// One content-addressed check applied to a source snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssuranceCheck {
    /// Stable check identity.
    pub check_id: String,
    /// Semantic category of the check.
    pub kind: AssuranceCheckKind,
    /// Whether the check passed.
    pub passed: bool,
    /// Digest of the check inputs and output.
    pub evidence_digest: String,
    /// Verifier implementation/build identity.
    pub verifier_identity: String,
}

/// Independence between the assessed source and a corroborating source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CorroborationIndependence {
    /// A mirror or derivative of the same publication.
    SamePublication,
    /// A distinct dataset from the same publishing organization.
    SameOrganization,
    /// A dataset produced by an independently governed publisher.
    IndependentPublisher,
    /// A separately measured or surveyed authoritative source.
    IndependentMeasurement,
}

impl CorroborationIndependence {
    fn is_independent(self) -> bool {
        matches!(
            self,
            Self::IndependentPublisher | Self::IndependentMeasurement
        )
    }
}

/// Evidence comparing the assessed source with another immutable snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorroborationEvidence {
    /// Stable identifier of the corroborating source.
    pub source_id: String,
    /// Content digest of the exact corroborating snapshot.
    pub snapshot_digest: String,
    /// Governance or measurement independence.
    pub independence: CorroborationIndependence,
    /// Whether the comparison agreed within its declared tolerance.
    pub agrees: bool,
    /// Content digest of the comparison evidence.
    pub evidence_digest: String,
}

/// Lifecycle state of a challenge to source data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisputeStatus {
    /// The challenge has not been resolved.
    Open,
    /// The publisher or reviewer acknowledged the issue and work is pending.
    Acknowledged,
    /// Evidence records how the challenge was resolved.
    Resolved,
}

/// Machine-readable challenge, correction notice, or reviewer objection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisputeRecord {
    /// Stable dispute identifier.
    pub dispute_id: String,
    /// Current lifecycle state.
    pub status: DisputeStatus,
    /// Concise description of the challenged claim.
    pub summary: String,
    /// Optional digest of the resolution or supporting evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_digest: Option<String>,
}

/// Quantified uncertainty declared for the source as a whole.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceUncertainty {
    /// Estimation or publication method.
    pub method: String,
    /// Relative uncertainty in parts per million, when quantified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative_ppm: Option<u64>,
    /// Scope to which the estimate applies.
    pub scope: String,
}

/// Evidence dossier for one immutable source snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceAssurance {
    /// Version of this evidence document.
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    /// Stable identifier shared with source admission evidence.
    pub source_id: String,
    /// SHA-256 identity of the exact bytes or canonical record collection.
    pub snapshot_digest: String,
    /// Identified publishing organization or responsible person.
    pub publisher: String,
    /// Relationship between publisher and subject matter.
    pub authority_class: AuthorityClass,
    /// Publisher-declared release timestamp or date, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    /// Time at which this dossier was evaluated.
    pub assessed_at: String,
    /// Age observed at assessment time, in whole days, when publication time
    /// is known. `None` remains unknown and never means freshly published.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_age_days: Option<u32>,
    /// Content-addressed validation and anomaly checks.
    #[serde(default)]
    pub checks: Vec<AssuranceCheck>,
    /// Comparisons with other immutable sources.
    #[serde(default)]
    pub corroborations: Vec<CorroborationEvidence>,
    /// Declared or independently estimated uncertainty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uncertainty: Option<SourceUncertainty>,
    /// Challenges and correction records that must remain visible.
    #[serde(default)]
    pub disputes: Vec<DisputeRecord>,
    /// Known exclusions, caveats, and non-claims.
    #[serde(default)]
    pub limitations: Vec<String>,
}

impl SourceAssurance {
    /// Construct an identified, otherwise unevaluated source dossier.
    pub fn new(
        source_id: impl Into<String>,
        snapshot_digest: impl Into<String>,
        publisher: impl Into<String>,
        authority_class: AuthorityClass,
    ) -> Self {
        Self {
            schema_version: default_schema_version(),
            source_id: source_id.into(),
            snapshot_digest: snapshot_digest.into(),
            publisher: publisher.into(),
            authority_class,
            published_at: None,
            assessed_at: String::new(),
            observed_age_days: None,
            checks: Vec::new(),
            corroborations: Vec::new(),
            uncertainty: None,
            disputes: Vec::new(),
            limitations: Vec::new(),
        }
    }

    /// Return a deterministic digest covering every assurance claim and limit.
    pub fn digest(&self) -> Result<String, serde_json::Error> {
        let bytes = serde_json::to_vec(self)?;
        Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
    }
}

/// Policy requirements for releasing a source-backed result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssurancePolicy {
    /// Authority relationships accepted for this use case.
    #[serde(default)]
    pub accepted_authority_classes: BTreeSet<AuthorityClass>,
    /// Maximum source age at assessment time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_age_days: Option<u32>,
    /// Check categories that must have a passing observation.
    #[serde(default)]
    pub required_checks: BTreeSet<AssuranceCheckKind>,
    /// Minimum agreeing comparisons from independently governed/measured data.
    #[serde(default)]
    pub minimum_independent_corroborations: u32,
    /// Require a documented uncertainty scope and method.
    #[serde(default)]
    pub require_uncertainty: bool,
    /// Require at least one explicit limitation/non-claim.
    #[serde(default)]
    pub require_limitations: bool,
    /// Permit acknowledged or open disputes for this use case.
    #[serde(default)]
    pub allow_unresolved_disputes: bool,
}

impl AssurancePolicy {
    /// Evaluate an assurance dossier without inferring truth from reputation.
    pub fn assess(&self, assurance: &SourceAssurance) -> AssuranceReport {
        assess(self, assurance)
    }
}

/// Highest assurance stage justified by the supplied evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssuranceLevel {
    /// Required identity or document structure is missing.
    Unassessed,
    /// Snapshot and publisher are identified, but use-case checks are incomplete.
    Identified,
    /// Required checks, freshness, uncertainty, and dispute policy passed.
    Checked,
    /// Checked plus the required number of independent comparisons passed.
    Corroborated,
}

/// One fail-closed reason preventing stronger source assurance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssuranceFailure {
    /// Stable machine-readable failure code.
    pub code: String,
    /// Source, check, corroboration, or dispute identifier.
    pub subject: String,
    /// Human-readable explanation that avoids claiming universal truth.
    pub detail: String,
}

/// Policy-derived source assurance result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssuranceReport {
    /// Highest evidence stage reached.
    pub level: AssuranceLevel,
    /// Whether every policy condition passed.
    pub passed: bool,
    /// Digest binding the assessed assurance dossier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assurance_digest: Option<String>,
    /// Structured reasons why policy did not pass.
    pub failures: Vec<AssuranceFailure>,
    /// Explicit reminder of the scope of this result.
    pub scope_statement: String,
}

fn assess(policy: &AssurancePolicy, assurance: &SourceAssurance) -> AssuranceReport {
    let mut identity_failures = Vec::new();
    if assurance.schema_version != SOURCE_ASSURANCE_SCHEMA_VERSION {
        push_failure(
            &mut identity_failures,
            "unsupported_assurance_schema",
            &assurance.schema_version,
            "source assurance schema version is not supported",
        );
    }
    for (code, subject, value) in [
        ("missing_source_id", "source", assurance.source_id.as_str()),
        (
            "invalid_snapshot_digest",
            assurance.source_id.as_str(),
            assurance.snapshot_digest.as_str(),
        ),
        (
            "missing_publisher",
            assurance.source_id.as_str(),
            assurance.publisher.as_str(),
        ),
        (
            "missing_assessed_at",
            assurance.source_id.as_str(),
            assurance.assessed_at.as_str(),
        ),
    ] {
        let valid = if code == "invalid_snapshot_digest" {
            valid_digest(value)
        } else {
            !value.trim().is_empty()
        };
        if !valid {
            push_failure(
                &mut identity_failures,
                code,
                subject,
                "required source identity evidence is missing or malformed",
            );
        }
    }
    if assurance
        .published_at
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        push_failure(
            &mut identity_failures,
            "invalid_published_at",
            &assurance.source_id,
            "published_at is present but empty",
        );
    }
    let assurance_digest = assurance.digest().ok();
    if !identity_failures.is_empty() || assurance_digest.is_none() {
        return report(
            AssuranceLevel::Unassessed,
            false,
            assurance_digest,
            identity_failures,
        );
    }

    let mut failures = Vec::new();
    if !policy.accepted_authority_classes.is_empty()
        && !policy
            .accepted_authority_classes
            .contains(&assurance.authority_class)
    {
        push_failure(
            &mut failures,
            "authority_not_accepted",
            &assurance.publisher,
            "publisher authority class is not accepted for this use case",
        );
    }
    if let Some(maximum) = policy.max_age_days {
        match (
            assurance.published_at.as_deref(),
            assurance.observed_age_days,
        ) {
            (Some(_), Some(age)) if age > maximum => push_failure(
                &mut failures,
                "source_stale",
                &assurance.source_id,
                &format!("observed age {age} days exceeds policy maximum {maximum} days"),
            ),
            (Some(_), Some(_)) => {}
            _ => push_failure(
                &mut failures,
                "source_freshness_unknown",
                &assurance.source_id,
                "policy requires freshness but publication date or observed age is unknown",
            ),
        }
    }

    let mut check_ids = BTreeSet::new();
    for check in &assurance.checks {
        if check.check_id.trim().is_empty() || !check_ids.insert(&check.check_id) {
            push_failure(
                &mut failures,
                "invalid_assurance_check",
                &check.check_id,
                "check identifier is empty or duplicated",
            );
        }
        if check.verifier_identity.trim().is_empty() || !valid_digest(&check.evidence_digest) {
            push_failure(
                &mut failures,
                "check_evidence_not_replayable",
                &check.check_id,
                "check lacks verifier identity or a valid evidence digest",
            );
        }
    }
    for required in &policy.required_checks {
        match assurance
            .checks
            .iter()
            .find(|check| check.kind == *required)
        {
            None => push_failure(
                &mut failures,
                "required_assurance_check_missing",
                &format!("{required:?}"),
                "required source-assurance check is missing",
            ),
            Some(check) if !check.passed => push_failure(
                &mut failures,
                "required_assurance_check_failed",
                &check.check_id,
                "required source-assurance check failed",
            ),
            Some(_) => {}
        }
    }
    if policy.require_uncertainty {
        match assurance.uncertainty.as_ref() {
            Some(uncertainty)
                if !uncertainty.method.trim().is_empty()
                    && !uncertainty.scope.trim().is_empty() => {}
            _ => push_failure(
                &mut failures,
                "uncertainty_not_documented",
                &assurance.source_id,
                "policy requires uncertainty method and scope",
            ),
        }
    }
    if policy.require_limitations
        && !assurance
            .limitations
            .iter()
            .any(|limit| !limit.trim().is_empty())
    {
        push_failure(
            &mut failures,
            "limitations_not_documented",
            &assurance.source_id,
            "policy requires an explicit limitation or non-claim",
        );
    }
    if !policy.allow_unresolved_disputes {
        for dispute in assurance
            .disputes
            .iter()
            .filter(|dispute| dispute.status != DisputeStatus::Resolved)
        {
            push_failure(
                &mut failures,
                "unresolved_source_dispute",
                &dispute.dispute_id,
                "an unresolved source challenge prevents assurance policy from passing",
            );
        }
    }
    if !failures.is_empty() {
        return report(
            AssuranceLevel::Identified,
            false,
            assurance_digest,
            failures,
        );
    }

    let independent = assurance
        .corroborations
        .iter()
        .filter(|item| {
            item.independence.is_independent()
                && item.agrees
                && valid_digest(&item.snapshot_digest)
                && valid_digest(&item.evidence_digest)
                && item.source_id != assurance.source_id
        })
        .count() as u32;
    if independent < policy.minimum_independent_corroborations {
        push_failure(
            &mut failures,
            "insufficient_independent_corroboration",
            &assurance.source_id,
            &format!(
                "{independent} independent agreeing sources; policy requires {}",
                policy.minimum_independent_corroborations
            ),
        );
        return report(AssuranceLevel::Checked, false, assurance_digest, failures);
    }

    report(
        if policy.minimum_independent_corroborations > 0 {
            AssuranceLevel::Corroborated
        } else {
            AssuranceLevel::Checked
        },
        true,
        assurance_digest,
        Vec::new(),
    )
}

fn report(
    level: AssuranceLevel,
    passed: bool,
    assurance_digest: Option<String>,
    failures: Vec<AssuranceFailure>,
) -> AssuranceReport {
    AssuranceReport {
        level,
        passed,
        assurance_digest,
        failures,
        scope_statement: "This assessment verifies declared evidence against a use-case policy; it does not prove universal truth.".into(),
    }
}

fn push_failure(failures: &mut Vec<AssuranceFailure>, code: &str, subject: &str, detail: &str) {
    failures.push(AssuranceFailure {
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

    const DIGEST_A: &str =
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const DIGEST_B: &str =
        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const DIGEST_C: &str =
        "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

    fn policy() -> AssurancePolicy {
        AssurancePolicy {
            accepted_authority_classes: BTreeSet::from([AuthorityClass::PrimaryOfficial]),
            max_age_days: Some(365),
            required_checks: BTreeSet::from([
                AssuranceCheckKind::Schema,
                AssuranceCheckKind::Completeness,
                AssuranceCheckKind::AnomalyDetection,
            ]),
            minimum_independent_corroborations: 1,
            require_uncertainty: true,
            require_limitations: true,
            allow_unresolved_disputes: false,
        }
    }

    fn dossier() -> SourceAssurance {
        SourceAssurance {
            schema_version: SOURCE_ASSURANCE_SCHEMA_VERSION.into(),
            source_id: "nagoya.population.2020".into(),
            snapshot_digest: DIGEST_A.into(),
            publisher: "Nagoya City".into(),
            authority_class: AuthorityClass::PrimaryOfficial,
            published_at: Some("2021-11-30".into()),
            assessed_at: "2022-01-01T00:00:00Z".into(),
            observed_age_days: Some(32),
            checks: [
                ("schema", AssuranceCheckKind::Schema),
                ("complete", AssuranceCheckKind::Completeness),
                ("outliers", AssuranceCheckKind::AnomalyDetection),
            ]
            .into_iter()
            .map(|(check_id, kind)| AssuranceCheck {
                check_id: check_id.into(),
                kind,
                passed: true,
                evidence_digest: DIGEST_B.into(),
                verifier_identity: "genegis-source-check/0.1.0".into(),
            })
            .collect(),
            corroborations: vec![CorroborationEvidence {
                source_id: "estat.population.2020".into(),
                snapshot_digest: DIGEST_C.into(),
                independence: CorroborationIndependence::IndependentPublisher,
                agrees: true,
                evidence_digest: DIGEST_B.into(),
            }],
            uncertainty: Some(SourceUncertainty {
                method: "publisher methodology".into(),
                relative_ppm: None,
                scope: "resident population by ward".into(),
            }),
            disputes: Vec::new(),
            limitations: vec!["Census reference date; not a live population estimate.".into()],
        }
    }

    #[test]
    fn corroborated_assurance_passes_without_claiming_truth() {
        let report = policy().assess(&dossier());
        assert!(report.passed);
        assert_eq!(report.level, AssuranceLevel::Corroborated);
        assert!(report
            .scope_statement
            .contains("does not prove universal truth"));
        assert!(report.assurance_digest.as_deref().is_some_and(valid_digest));
    }

    #[test]
    fn stale_failed_checks_and_disputes_fail_closed() {
        let mut source = dossier();
        source.observed_age_days = Some(366);
        source.checks[1].passed = false;
        source.disputes.push(DisputeRecord {
            dispute_id: "ward-total-challenge".into(),
            status: DisputeStatus::Open,
            summary: "Published total is disputed.".into(),
            evidence_digest: None,
        });
        let report = policy().assess(&source);
        assert!(!report.passed);
        assert_eq!(report.level, AssuranceLevel::Identified);
        let codes = report
            .failures
            .iter()
            .map(|failure| failure.code.as_str())
            .collect::<BTreeSet<_>>();
        assert!(codes.contains("source_stale"));
        assert!(codes.contains("required_assurance_check_failed"));
        assert!(codes.contains("unresolved_source_dispute"));
    }

    #[test]
    fn unknown_publication_age_never_satisfies_freshness_policy() {
        let mut source = dossier();
        source.published_at = None;
        source.observed_age_days = None;
        let report = policy().assess(&source);
        assert!(!report.passed);
        assert!(report
            .failures
            .iter()
            .any(|failure| failure.code == "source_freshness_unknown"));
    }

    #[test]
    fn mirror_does_not_count_as_independent_corroboration() {
        let mut source = dossier();
        source.corroborations[0].independence = CorroborationIndependence::SamePublication;
        let report = policy().assess(&source);
        assert!(!report.passed);
        assert_eq!(report.level, AssuranceLevel::Checked);
        assert_eq!(
            report.failures[0].code,
            "insufficient_independent_corroboration"
        );
    }

    #[test]
    fn assurance_digest_changes_when_a_limitation_is_removed() {
        let original = dossier();
        let mut changed = original.clone();
        changed.limitations.clear();
        assert_ne!(original.digest().unwrap(), changed.digest().unwrap());
    }
}
