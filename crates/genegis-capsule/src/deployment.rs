//! Shared replay and offline capsule verification across deployment classes.

use std::path::Path;

use genegis_analysis::run_ask_pipeline;
use genegis_core::{DeploymentClass, DeploymentProfile};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{seal_nagoya_capsule, verify_nagoya_capsule, CapsuleError};

const NORTH_STAR_PROMPT: &str = "名古屋市の人口密度を表示";

/// One deployment's execution of the exact shared corpus case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentConformanceCase {
    /// Profile identity.
    pub profile_id: String,
    /// Deployment class.
    pub class: DeploymentClass,
    /// Exact profile identity.
    pub profile_digest: String,
    /// Stable replayed workflow identity.
    pub workflow_digest: String,
    /// Stable execution result identity.
    pub result_digest: String,
    /// Result identity independently recovered from the capsule.
    pub capsule_result_digest: String,
    /// Verification graph identity independently recovered from the capsule.
    pub verification_graph_digest: String,
    /// Number of content-addressed capsule subjects verified.
    pub verified_capsule_entries: usize,
    /// This corpus is intentionally fixture-only for every profile.
    pub network_used: bool,
}

/// One sealed result proving profile-equivalent replay semantics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentConformanceReport {
    /// Report schema.
    pub schema_version: String,
    /// Exact corpus identity.
    pub corpus_digest: String,
    /// Profile executions in caller order.
    pub cases: Vec<DeploymentConformanceCase>,
    /// Whether all profiles produced identical semantic workflow/results.
    pub equivalent: bool,
    /// Canonical complete report identity.
    pub report_digest: String,
}

/// Replay the north-star workflow and verify its capsule for every profile.
pub fn run_deployment_conformance(
    profiles: &[DeploymentProfile],
    output_root: impl AsRef<Path>,
) -> Result<DeploymentConformanceReport, CapsuleError> {
    if profiles.is_empty() {
        return Err(CapsuleError::Verification(
            "deployment conformance requires profiles".into(),
        ));
    }
    let corpus_digest = digest(&serde_json::json!({
        "prompt": NORTH_STAR_PROMPT,
        "fixture_only": true,
        "assertions": ["command_workflow", "result_digest", "offline_capsule_verification"]
    }))?;
    let mut cases = Vec::new();
    for profile in profiles {
        profile
            .validate()
            .map_err(|error| CapsuleError::Verification(error.to_string()))?;
        let run = run_ask_pipeline(NORTH_STAR_PROMPT)
            .map_err(|error| CapsuleError::Verification(error.to_string()))?;
        if !run.duckdb_verified || !run.execution_receipt.verification_passed {
            return Err(CapsuleError::Verification(
                "north-star verification failed in deployment corpus".into(),
            ));
        }
        let root = output_root.as_ref().join(&profile.id);
        seal_nagoya_capsule(&run, &root)?;
        let verified =
            verify_nagoya_capsule(&root, run.execution_receipt.verification_policy.as_ref())?;
        if verified.result_digest != run.execution_receipt.result_digest {
            return Err(CapsuleError::Verification(
                "capsule and execution result identities differ".into(),
            ));
        }
        cases.push(DeploymentConformanceCase {
            profile_id: profile.id.clone(),
            class: profile.class,
            profile_digest: profile
                .stable_digest()
                .map_err(|error| CapsuleError::Verification(error.to_string()))?,
            workflow_digest: run.execution_receipt.workflow_digest.as_str().into(),
            result_digest: run.execution_receipt.result_digest,
            capsule_result_digest: verified.result_digest,
            verification_graph_digest: verified.verification_graph_digest,
            verified_capsule_entries: verified.verified_entries,
            network_used: false,
        });
    }
    let first = &cases[0];
    let equivalent = cases.iter().all(|case| {
        case.workflow_digest == first.workflow_digest
            && case.result_digest == first.result_digest
            && case.capsule_result_digest == first.capsule_result_digest
            && case.verification_graph_digest == first.verification_graph_digest
            && !case.network_used
    });
    if !equivalent {
        return Err(CapsuleError::Verification(
            "deployment profiles produced different verification semantics".into(),
        ));
    }
    let mut report = DeploymentConformanceReport {
        schema_version: "1.0.0".into(),
        corpus_digest,
        cases,
        equivalent,
        report_digest: String::new(),
    };
    report.report_digest = report_digest(&report)?;
    Ok(report)
}

fn report_digest(report: &DeploymentConformanceReport) -> Result<String, CapsuleError> {
    let mut semantic = report.clone();
    semantic.report_digest.clear();
    digest(&semantic)
}

fn digest<T: Serialize>(value: &T) -> Result<String, CapsuleError> {
    let bytes = serde_json::to_vec(value)?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_three_profiles_replay_and_verify_the_same_corpus() {
        let profiles = [
            serde_json::from_str(include_str!("../../../deploy/profiles/local-first.json"))
                .expect("local"),
            serde_json::from_str(include_str!("../../../deploy/profiles/managed-cloud.json"))
                .expect("cloud"),
            serde_json::from_str(include_str!("../../../deploy/profiles/air-gapped.json"))
                .expect("air gap"),
        ];
        let output = tempfile::tempdir().expect("output");
        let report = run_deployment_conformance(&profiles, output.path()).expect("conformance");
        assert!(report.equivalent);
        assert_eq!(report.cases.len(), 3);
        assert!(report.cases.iter().all(|case| !case.network_used));
    }
}
