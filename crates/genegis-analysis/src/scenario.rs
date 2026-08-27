//! Digest-bound scenario branching, semantic comparison, approval, and merge.

use std::{collections::BTreeMap, sync::Mutex};

use chrono::DateTime;
use genegis_core::{
    Command, CommandBus, CommandEnvelope, CommandOrigin, InputSnapshot, Project, WorkflowDigest,
    WorkflowExecution, WorkflowExecutionContext, WorkflowExecutionError, WorkflowExecutionEvent,
    WorkflowExecutor,
};
use genegis_crs::{ChecksumVerification, SourceSnapshot};
use genegis_workflow::{
    scenario_branch_template, scenario_comparison_template, scenario_merge_template, GeoWorkflow,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Scenario document schema version.
pub const SCENARIO_SCHEMA_VERSION: &str = "0.1.0";

/// One typed and unit-bearing scenario assumption.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioAssumption {
    /// JSON scalar/object value.
    pub value: serde_json::Value,
    /// Semantic unit, or `dimensionless`.
    pub unit: String,
}

/// One spatial outcome summarized by stable area identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioSpatialOutcome {
    /// Outcome unit.
    pub unit: String,
    /// Numeric value by stable area/feature identity.
    pub by_area: BTreeMap<String, f64>,
}

/// Unsealed scenario branch request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioBranchDraft {
    /// Stable branch identity.
    pub branch_id: String,
    /// Project state from which the scenario diverged.
    pub base_project_digest: String,
    /// Exact workflow that produced outcomes.
    pub workflow_digest: String,
    /// Exact scenario analytical result.
    pub result_digest: String,
    /// Typed changed assumptions.
    pub assumptions: BTreeMap<String, ScenarioAssumption>,
    /// Spatial outcomes available for comparison.
    pub outcomes: BTreeMap<String, ScenarioSpatialOutcome>,
}

/// Sealed scenario branch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioBranch {
    /// Schema version.
    pub schema_version: String,
    /// Stable branch identity.
    pub branch_id: String,
    /// Common base project identity.
    pub base_project_digest: String,
    /// Outcome workflow identity.
    pub workflow_digest: String,
    /// Outcome result identity.
    pub result_digest: String,
    /// Typed assumptions.
    pub assumptions: BTreeMap<String, ScenarioAssumption>,
    /// Spatial outcomes.
    pub outcomes: BTreeMap<String, ScenarioSpatialOutcome>,
    /// Canonical branch identity.
    pub branch_digest: String,
}

/// One semantic assumption or spatial outcome change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioSemanticChange {
    /// `assumption` or `outcome`.
    pub category: String,
    /// Stable JSON-like semantic path.
    pub path: String,
    /// Base value, absent when added.
    pub base: Option<serde_json::Value>,
    /// Scenario value, absent when removed.
    pub scenario: Option<serde_json::Value>,
}

/// Sealed semantic comparison.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioComparison {
    /// Common base project identity.
    pub base_project_digest: String,
    /// Base branch identity.
    pub base_branch_digest: String,
    /// Compared branch identity.
    pub scenario_branch_digest: String,
    /// Deterministically ordered semantic changes.
    pub changes: Vec<ScenarioSemanticChange>,
    /// Canonical diff identity.
    pub diff_digest: String,
}

/// Human approval bound to one exact scenario diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioApproval {
    /// Reviewer identity.
    pub reviewer: String,
    /// RFC 3339 approval time.
    pub approved_at: String,
    /// Exact reviewed diff.
    pub diff_digest: String,
    /// Exact target branch.
    pub scenario_branch_digest: String,
}

/// Sealed merge commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioMergeCommit {
    /// Base project identity.
    pub base_project_digest: String,
    /// Selected branch identity.
    pub scenario_branch_digest: String,
    /// Reviewed semantic diff identity.
    pub diff_digest: String,
    /// Reviewer identity.
    pub reviewer: String,
    /// Approval time.
    pub approved_at: String,
    /// New merged project state identity.
    pub merged_project_digest: String,
    /// Canonical merge commit identity.
    pub merge_digest: String,
}

/// Command/workflow receipt for branch creation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioBranchReceipt {
    /// Applied command identity.
    pub command_id: String,
    /// Exact branch workflow identity.
    pub workflow_digest: WorkflowDigest,
    /// Sealed branch.
    pub branch: ScenarioBranch,
}

/// Command/workflow receipt for comparison.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioComparisonReceipt {
    /// Applied command identity.
    pub command_id: String,
    /// Exact comparison workflow identity.
    pub workflow_digest: WorkflowDigest,
    /// Sealed comparison.
    pub comparison: ScenarioComparison,
}

/// Command/workflow receipt for reviewed merge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioMergeReceipt {
    /// Applied command identity.
    pub command_id: String,
    /// Exact merge workflow identity.
    pub workflow_digest: WorkflowDigest,
    /// Sealed merge commit.
    pub commit: ScenarioMergeCommit,
}

/// Fail-closed scenario error.
#[derive(Debug, Error)]
pub enum ScenarioError {
    /// Branch, diff, approval, or merge contract is invalid.
    #[error("invalid scenario: {0}")]
    Invalid(String),
    /// Canonical serialization failed.
    #[error("scenario serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    /// Command/workflow execution failed.
    #[error("scenario workflow failed: {0}")]
    Workflow(String),
}

#[derive(Clone)]
enum ScenarioOperation {
    Branch(ScenarioBranchDraft),
    Compare(ScenarioBranch, ScenarioBranch),
    Merge(ScenarioBranch, ScenarioBranch, ScenarioApproval),
}

#[derive(Clone)]
enum ScenarioOutput {
    Branch(ScenarioBranch),
    Comparison(ScenarioComparison),
    Merge(ScenarioMergeCommit),
}

struct ScenarioExecutor {
    operation: ScenarioOperation,
    output: Mutex<Option<ScenarioOutput>>,
}

impl WorkflowExecutor for ScenarioExecutor {
    fn execute(
        &self,
        _workflow: &GeoWorkflow,
        context: &WorkflowExecutionContext,
    ) -> Result<WorkflowExecution, WorkflowExecutionError> {
        let output = match &self.operation {
            ScenarioOperation::Branch(draft) => {
                seal_branch(draft.clone()).map(ScenarioOutput::Branch)
            }
            ScenarioOperation::Compare(base, scenario) => {
                compare(base, scenario).map(ScenarioOutput::Comparison)
            }
            ScenarioOperation::Merge(base, scenario, approval) => {
                merge(base, scenario, approval).map(ScenarioOutput::Merge)
            }
        }
        .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        let (kind, result_digest) = match &output {
            ScenarioOutput::Branch(branch) => {
                ("scenario_branch_created", branch.branch_digest.clone())
            }
            ScenarioOutput::Comparison(diff) => ("scenario_compared", diff.diff_digest.clone()),
            ScenarioOutput::Merge(commit) => ("scenario_merged", commit.merge_digest.clone()),
        };
        let evidence = match &output {
            ScenarioOutput::Branch(value) => serde_json::to_value(value),
            ScenarioOutput::Comparison(value) => serde_json::to_value(value),
            ScenarioOutput::Merge(value) => serde_json::to_value(value),
        }
        .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        *self
            .output
            .lock()
            .map_err(|_| WorkflowExecutionError::Failed("scenario lock poisoned".into()))? =
            Some(output);
        Ok(WorkflowExecution {
            result_digest,
            output: serde_json::json!({"operation": kind}),
            evidence,
            events: vec![WorkflowExecutionEvent {
                kind: kind.into(),
                source_uri: None,
                observed_at: context.command_timestamp,
                details: serde_json::json!({
                    "command_id": context.command_id,
                    "workflow_digest": context.workflow_digest,
                }),
            }],
        })
    }
}

/// Create a scenario branch through Command + Workflow.
pub fn create_scenario_branch(
    draft: ScenarioBranchDraft,
) -> Result<ScenarioBranchReceipt, ScenarioError> {
    validate_draft(&draft)?;
    let source = verified_source("project", &draft.base_project_digest);
    let workflow = scenario_branch_template(source.clone(), &draft.branch_id);
    let (command_id, workflow_digest, output) = execute_operation(
        workflow,
        source,
        "scenario-base",
        ScenarioOperation::Branch(draft),
    )?;
    let ScenarioOutput::Branch(branch) = output else {
        unreachable!()
    };
    Ok(ScenarioBranchReceipt {
        command_id,
        workflow_digest,
        branch,
    })
}

/// Compare assumptions and per-area outcomes through Command + Workflow.
pub fn compare_scenarios(
    base: ScenarioBranch,
    scenario: ScenarioBranch,
) -> Result<ScenarioComparisonReceipt, ScenarioError> {
    verify_branch(&base)?;
    verify_branch(&scenario)?;
    let source = verified_source("scenario", &base.branch_digest);
    let workflow = scenario_comparison_template(source.clone());
    let (command_id, workflow_digest, output) = execute_operation(
        workflow,
        source,
        "scenario-comparison",
        ScenarioOperation::Compare(base, scenario),
    )?;
    let ScenarioOutput::Comparison(comparison) = output else {
        unreachable!()
    };
    Ok(ScenarioComparisonReceipt {
        command_id,
        workflow_digest,
        comparison,
    })
}

/// Merge only the exact branch/diff approved by a reviewer.
pub fn merge_reviewed_scenario(
    base: ScenarioBranch,
    scenario: ScenarioBranch,
    approval: ScenarioApproval,
) -> Result<ScenarioMergeReceipt, ScenarioError> {
    let comparison = compare(&base, &scenario)?;
    if approval.reviewer.trim().is_empty()
        || DateTime::parse_from_rfc3339(&approval.approved_at).is_err()
        || approval.diff_digest != comparison.diff_digest
        || approval.scenario_branch_digest != scenario.branch_digest
    {
        return Err(ScenarioError::Invalid(
            "approval is stale or mismatched".into(),
        ));
    }
    let source = verified_source("scenario-diff", &comparison.diff_digest);
    let workflow = scenario_merge_template(source.clone(), &comparison.diff_digest);
    let (command_id, workflow_digest, output) = execute_operation(
        workflow,
        source,
        "scenario-merge",
        ScenarioOperation::Merge(base, scenario, approval),
    )?;
    let ScenarioOutput::Merge(commit) = output else {
        unreachable!()
    };
    Ok(ScenarioMergeReceipt {
        command_id,
        workflow_digest,
        commit,
    })
}

fn execute_operation(
    workflow: GeoWorkflow,
    source: SourceSnapshot,
    input: &str,
    operation: ScenarioOperation,
) -> Result<(String, WorkflowDigest, ScenarioOutput), ScenarioError> {
    let workflow_digest = WorkflowDigest::new(
        workflow
            .stable_digest()
            .map_err(|error| ScenarioError::Workflow(error.to_string()))?,
    );
    let envelope = CommandEnvelope::new(
        CommandOrigin::Ui,
        Command::RunWorkflow {
            workflow_id: workflow.id,
        },
    )
    .with_workflow_digest(workflow_digest.clone())
    .with_source_snapshot(source.clone())
    .with_input_snapshot(InputSnapshot::new(input, source));
    let command_id = envelope.id;
    let executor = ScenarioExecutor {
        operation,
        output: Mutex::new(None),
    };
    let mut project = Project::new("Scenario operation");
    let mut bus = CommandBus::new(project.clone());
    bus.register_workflow(workflow)
        .map_err(|error| ScenarioError::Workflow(error.to_string()))?;
    let execution = bus
        .apply_with_executor(&mut project, envelope, &executor)
        .map_err(|error| ScenarioError::Workflow(error.to_string()))?;
    let output = executor
        .output
        .into_inner()
        .map_err(|_| ScenarioError::Workflow("scenario lock poisoned".into()))?
        .ok_or_else(|| ScenarioError::Workflow("executor returned no scenario output".into()))?;
    let digest = match &output {
        ScenarioOutput::Branch(value) => &value.branch_digest,
        ScenarioOutput::Comparison(value) => &value.diff_digest,
        ScenarioOutput::Merge(value) => &value.merge_digest,
    };
    if execution.result_digest.as_deref() != Some(digest.as_str()) {
        return Err(ScenarioError::Workflow(
            "CommandBus scenario digest mismatch".into(),
        ));
    }
    Ok((command_id.to_string(), workflow_digest, output))
}

fn seal_branch(draft: ScenarioBranchDraft) -> Result<ScenarioBranch, ScenarioError> {
    validate_draft(&draft)?;
    let mut branch = ScenarioBranch {
        schema_version: SCENARIO_SCHEMA_VERSION.into(),
        branch_id: draft.branch_id,
        base_project_digest: draft.base_project_digest,
        workflow_digest: draft.workflow_digest,
        result_digest: draft.result_digest,
        assumptions: draft.assumptions,
        outcomes: draft.outcomes,
        branch_digest: String::new(),
    };
    branch.branch_digest = branch_digest(&branch)?;
    Ok(branch)
}

fn verify_branch(branch: &ScenarioBranch) -> Result<(), ScenarioError> {
    validate_draft(&ScenarioBranchDraft {
        branch_id: branch.branch_id.clone(),
        base_project_digest: branch.base_project_digest.clone(),
        workflow_digest: branch.workflow_digest.clone(),
        result_digest: branch.result_digest.clone(),
        assumptions: branch.assumptions.clone(),
        outcomes: branch.outcomes.clone(),
    })?;
    if branch.schema_version != SCENARIO_SCHEMA_VERSION
        || branch.branch_digest != branch_digest(branch)?
    {
        return Err(ScenarioError::Invalid("branch digest mismatch".into()));
    }
    Ok(())
}

fn compare(
    base: &ScenarioBranch,
    scenario: &ScenarioBranch,
) -> Result<ScenarioComparison, ScenarioError> {
    verify_branch(base)?;
    verify_branch(scenario)?;
    if base.base_project_digest != scenario.base_project_digest
        || base.branch_digest == scenario.branch_digest
    {
        return Err(ScenarioError::Invalid(
            "branches require same base and distinct identity".into(),
        ));
    }
    let mut changes = Vec::new();
    diff_maps(
        "assumption",
        &base.assumptions,
        &scenario.assumptions,
        &mut changes,
    )?;
    diff_maps("outcome", &base.outcomes, &scenario.outcomes, &mut changes)?;
    let mut comparison = ScenarioComparison {
        base_project_digest: base.base_project_digest.clone(),
        base_branch_digest: base.branch_digest.clone(),
        scenario_branch_digest: scenario.branch_digest.clone(),
        changes,
        diff_digest: String::new(),
    };
    comparison.diff_digest = digest(&serde_json::json!({
        "base_project_digest": comparison.base_project_digest,
        "base_branch_digest": comparison.base_branch_digest,
        "scenario_branch_digest": comparison.scenario_branch_digest,
        "changes": comparison.changes,
    }))?;
    Ok(comparison)
}

fn diff_maps<T: Serialize + PartialEq>(
    category: &str,
    base: &BTreeMap<String, T>,
    scenario: &BTreeMap<String, T>,
    changes: &mut Vec<ScenarioSemanticChange>,
) -> Result<(), serde_json::Error> {
    for key in base
        .keys()
        .chain(scenario.keys())
        .collect::<std::collections::BTreeSet<_>>()
    {
        if base.get(key) != scenario.get(key) {
            changes.push(ScenarioSemanticChange {
                category: category.into(),
                path: format!("/{category}s/{key}"),
                base: base.get(key).map(serde_json::to_value).transpose()?,
                scenario: scenario.get(key).map(serde_json::to_value).transpose()?,
            });
        }
    }
    Ok(())
}

fn merge(
    base: &ScenarioBranch,
    scenario: &ScenarioBranch,
    approval: &ScenarioApproval,
) -> Result<ScenarioMergeCommit, ScenarioError> {
    let comparison = compare(base, scenario)?;
    if approval.diff_digest != comparison.diff_digest
        || approval.scenario_branch_digest != scenario.branch_digest
    {
        return Err(ScenarioError::Invalid(
            "approval no longer matches semantic diff".into(),
        ));
    }
    let merged_project_digest = digest(&serde_json::json!({
        "base": base.base_project_digest, "selected_branch": scenario.branch_digest,
        "result": scenario.result_digest, "diff": comparison.diff_digest,
    }))?;
    let mut commit = ScenarioMergeCommit {
        base_project_digest: base.base_project_digest.clone(),
        scenario_branch_digest: scenario.branch_digest.clone(),
        diff_digest: comparison.diff_digest,
        reviewer: approval.reviewer.clone(),
        approved_at: approval.approved_at.clone(),
        merged_project_digest,
        merge_digest: String::new(),
    };
    commit.merge_digest = digest(&serde_json::json!({
        "base_project_digest": commit.base_project_digest, "scenario_branch_digest": commit.scenario_branch_digest,
        "diff_digest": commit.diff_digest, "reviewer": commit.reviewer, "approved_at": commit.approved_at,
        "merged_project_digest": commit.merged_project_digest,
    }))?;
    Ok(commit)
}

fn validate_draft(draft: &ScenarioBranchDraft) -> Result<(), ScenarioError> {
    if draft.branch_id.trim().is_empty()
        || draft.assumptions.is_empty()
        || draft.outcomes.is_empty()
    {
        return Err(ScenarioError::Invalid(
            "branch, assumptions, and outcomes are required".into(),
        ));
    }
    for value in [
        &draft.base_project_digest,
        &draft.workflow_digest,
        &draft.result_digest,
    ] {
        require_digest(value)?;
    }
    if draft
        .assumptions
        .iter()
        .any(|(key, value)| key.trim().is_empty() || value.unit.trim().is_empty())
        || draft.outcomes.iter().any(|(key, value)| {
            key.trim().is_empty()
                || value.unit.trim().is_empty()
                || value.by_area.is_empty()
                || value.by_area.values().any(|number| !number.is_finite())
        })
    {
        return Err(ScenarioError::Invalid(
            "assumption or outcome semantics are invalid".into(),
        ));
    }
    Ok(())
}

fn branch_digest(branch: &ScenarioBranch) -> Result<String, serde_json::Error> {
    let mut value = branch.clone();
    value.branch_digest.clear();
    digest(&value)
}

fn verified_source(kind: &str, value: &str) -> SourceSnapshot {
    let mut source = SourceSnapshot::new(format!("{kind}://{value}"));
    source.checksum = Some(value.into());
    source.observed_checksum = Some(value.into());
    source.checksum_status = ChecksumVerification::Verified;
    source
}

fn require_digest(value: &str) -> Result<(), ScenarioError> {
    if value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        Ok(())
    } else {
        Err(ScenarioError::Invalid("invalid SHA-256 identity".into()))
    }
}

fn digest(value: &impl Serialize) -> Result<String, serde_json::Error> {
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(value)?)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(value: &str) -> String {
        format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
    }

    fn draft(id: &str, speed: f64) -> ScenarioBranchDraft {
        ScenarioBranchDraft {
            branch_id: id.into(),
            base_project_digest: hash("project"),
            workflow_digest: hash("workflow"),
            result_digest: hash(&format!("result-{id}")),
            assumptions: [(
                "walking_speed".into(),
                ScenarioAssumption {
                    value: serde_json::json!(speed),
                    unit: "m/s".into(),
                },
            )]
            .into_iter()
            .collect(),
            outcomes: [(
                "reachable_population".into(),
                ScenarioSpatialOutcome {
                    unit: "persons".into(),
                    by_area: [("23101".into(), 1000.0 * speed)].into_iter().collect(),
                },
            )]
            .into_iter()
            .collect(),
        }
    }

    #[test]
    fn branches_compares_and_merges_only_reviewed_semantic_diff() {
        let base = create_scenario_branch(draft("base", 1.0)).expect("base");
        let scenario = create_scenario_branch(draft("faster", 1.2)).expect("scenario");
        let comparison =
            compare_scenarios(base.branch.clone(), scenario.branch.clone()).expect("compare");
        assert_eq!(comparison.comparison.changes.len(), 2);
        let approval = ScenarioApproval {
            reviewer: "reviewer-a".into(),
            approved_at: "2026-08-26T11:00:00Z".into(),
            diff_digest: comparison.comparison.diff_digest.clone(),
            scenario_branch_digest: scenario.branch.branch_digest.clone(),
        };
        let merged = merge_reviewed_scenario(
            base.branch.clone(),
            scenario.branch.clone(),
            approval.clone(),
        )
        .expect("merge");
        assert!(merged.commit.merge_digest.starts_with("sha256:"));
        let mut stale = approval;
        stale.diff_digest = hash("different-diff");
        assert!(merge_reviewed_scenario(base.branch, scenario.branch, stale).is_err());
    }

    #[test]
    fn rejects_cross_base_and_tampered_branch() {
        let base = create_scenario_branch(draft("base", 1.0))
            .expect("base")
            .branch;
        let mut other_draft = draft("other", 1.1);
        other_draft.base_project_digest = hash("other-project");
        let other = create_scenario_branch(other_draft).expect("other").branch;
        assert!(compare_scenarios(base.clone(), other).is_err());
        let mut tampered = base.clone();
        tampered
            .outcomes
            .get_mut("reachable_population")
            .expect("outcome")
            .by_area
            .insert("23101".into(), 9999.0);
        assert!(compare_scenarios(base, tampered).is_err());
    }
}
