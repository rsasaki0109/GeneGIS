//! Command + Workflow orchestration for organization governance operations.

use std::sync::Mutex;

use genegis_collab::{
    AuditExport, GovernanceDecision, GovernanceState, GovernedAction, RetentionDisposition,
    RetentionRecord,
};
use genegis_core::{
    Command, CommandBus, CommandEnvelope, CommandOrigin, InputSnapshot, Project, WorkflowDigest,
    WorkflowExecution, WorkflowExecutionContext, WorkflowExecutionError, WorkflowExecutionEvent,
    WorkflowExecutor,
};
use genegis_crs::{ChecksumVerification, SourceSnapshot};
use genegis_workflow::{organization_governance_template, GeoWorkflow};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::AnalysisError;

/// One policy-governed organization operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "operation")]
pub enum GovernanceOperation {
    Authorize {
        actor_id: String,
        project_id: String,
        action: GovernedAction,
        resource_digest: String,
        occurred_at: String,
    },
    Approve {
        approval_id: String,
        approver_id: String,
        occurred_at: String,
    },
    PlanRetention {
        records: Vec<RetentionRecord>,
        as_of: String,
    },
    ExportAudit {
        actor_id: String,
        exported_at: String,
    },
}

impl GovernanceOperation {
    fn name(&self) -> &'static str {
        match self {
            Self::Authorize { .. } => "authorize",
            Self::Approve { .. } => "approve",
            Self::PlanRetention { .. } => "plan_retention",
            Self::ExportAudit { .. } => "export_audit",
        }
    }
}

/// Typed operation output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum GovernanceOperationOutput {
    Decision(GovernanceDecision),
    RetentionPlan(Vec<RetentionDisposition>),
    AuditExport(AuditExport),
}

/// Receipted state transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GovernanceOperationReceipt {
    pub command_id: String,
    pub workflow_digest: WorkflowDigest,
    pub result_digest: String,
    pub state_digest: String,
    pub state: GovernanceState,
    pub output: GovernanceOperationOutput,
}

struct GovernanceExecutor {
    state: Mutex<Option<GovernanceState>>,
    operation: GovernanceOperation,
    output: Mutex<Option<GovernanceOperationOutput>>,
}

impl WorkflowExecutor for GovernanceExecutor {
    fn execute(
        &self,
        _workflow: &GeoWorkflow,
        context: &WorkflowExecutionContext,
    ) -> Result<WorkflowExecution, WorkflowExecutionError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| WorkflowExecutionError::Failed("governance state lock poisoned".into()))?
            .take()
            .ok_or_else(|| WorkflowExecutionError::Failed("governance state is absent".into()))?;
        let output = match &self.operation {
            GovernanceOperation::Authorize {
                actor_id,
                project_id,
                action,
                resource_digest,
                occurred_at,
            } => state
                .authorize(actor_id, project_id, *action, resource_digest, occurred_at)
                .map(GovernanceOperationOutput::Decision),
            GovernanceOperation::Approve {
                approval_id,
                approver_id,
                occurred_at,
            } => state
                .approve(approval_id, approver_id, occurred_at)
                .map(GovernanceOperationOutput::Decision),
            GovernanceOperation::PlanRetention { records, as_of } => state
                .retention_plan(records, as_of)
                .map(GovernanceOperationOutput::RetentionPlan),
            GovernanceOperation::ExportAudit {
                actor_id,
                exported_at,
            } => state
                .export_audit(actor_id, exported_at)
                .map(GovernanceOperationOutput::AuditExport),
        }
        .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        let result_digest = digest(&(state.clone(), output.clone()))
            .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        *self.state.lock().map_err(|_| {
            WorkflowExecutionError::Failed("governance state lock poisoned".into())
        })? = Some(state);
        *self.output.lock().map_err(|_| {
            WorkflowExecutionError::Failed("governance output lock poisoned".into())
        })? = Some(output.clone());
        Ok(WorkflowExecution {
            result_digest,
            output: serde_json::json!({"operation": self.operation.name()}),
            evidence: serde_json::to_value(output)
                .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?,
            events: vec![WorkflowExecutionEvent {
                kind: "organization_governance_applied".into(),
                source_uri: Some("governance://organization-policy".into()),
                observed_at: context.command_timestamp,
                details: serde_json::json!({
                    "command_id": context.command_id,
                    "workflow_digest": context.workflow_digest,
                    "operation": self.operation.name(),
                }),
            }],
        })
    }
}

/// Execute one governance state transition exclusively through CommandBus.
pub fn execute_governance_operation(
    state: GovernanceState,
    operation: GovernanceOperation,
) -> Result<GovernanceOperationReceipt, AnalysisError> {
    let state_digest_before = digest(&state)?;
    let mut source = SourceSnapshot::new("governance://organization-policy");
    source.checksum = Some(state_digest_before.clone());
    source.observed_checksum = Some(state_digest_before);
    source.checksum_status = ChecksumVerification::Verified;
    let workflow = organization_governance_template(source.clone(), operation.name());
    let workflow_digest = WorkflowDigest::new(
        workflow
            .stable_digest()
            .map_err(|error| AnalysisError::Message(error.to_string()))?,
    );
    let envelope = CommandEnvelope::new(
        CommandOrigin::Ui,
        Command::RunWorkflow {
            workflow_id: workflow.id,
        },
    )
    .with_workflow_digest(workflow_digest.clone())
    .with_source_snapshot(source.clone())
    .with_input_snapshot(InputSnapshot::new("organization-governance", source));
    let command_id = envelope.id.to_string();
    let executor = GovernanceExecutor {
        state: Mutex::new(Some(state)),
        operation,
        output: Mutex::new(None),
    };
    let mut project = Project::new("Organization governance");
    let mut bus = CommandBus::new(project.clone());
    bus.register_workflow(workflow)
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    let execution = bus
        .apply_with_executor(&mut project, envelope, &executor)
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    let state = executor
        .state
        .into_inner()
        .map_err(|_| AnalysisError::Message("governance state lock poisoned".into()))?
        .ok_or_else(|| AnalysisError::Message("governance executor returned no state".into()))?;
    let output = executor
        .output
        .into_inner()
        .map_err(|_| AnalysisError::Message("governance output lock poisoned".into()))?
        .ok_or_else(|| AnalysisError::Message("governance executor returned no output".into()))?;
    let result_digest = digest(&(state.clone(), output.clone()))?;
    if execution.result_digest.as_deref() != Some(result_digest.as_str()) {
        return Err(AnalysisError::Message(
            "CommandBus and governance result digests differ".into(),
        ));
    }
    Ok(GovernanceOperationReceipt {
        command_id,
        workflow_digest,
        result_digest,
        state_digest: digest(&state)?,
        state,
        output,
    })
}

fn digest<T: Serialize>(value: &T) -> Result<String, AnalysisError> {
    let bytes =
        serde_json::to_vec(value).map_err(|error| AnalysisError::Message(error.to_string()))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use genegis_collab::{
        GovernanceCapability, OrganizationMember, OrganizationPolicy, OrganizationProject,
        OrganizationRole, RetentionPolicy,
    };

    use super::*;

    #[test]
    fn governance_transition_is_command_and_workflow_bound() {
        let role = OrganizationRole {
            id: "analyst".into(),
            capabilities: BTreeSet::from([GovernanceCapability::Execute]),
        };
        let mut state = GovernanceState::new(OrganizationPolicy {
            schema_version: "0.1.0".into(),
            organization_id: "org".into(),
            policy_version: "1".into(),
            roles: BTreeMap::from([("analyst".into(), role)]),
            approval_thresholds: BTreeMap::new(),
            retention: RetentionPolicy {
                minimum_days: 30,
                maximum_days: 365,
                protected_classes: BTreeSet::new(),
            },
        })
        .expect("state");
        state
            .add_member(OrganizationMember {
                subject_id: "alice".into(),
                role_id: "analyst".into(),
                active: true,
            })
            .expect("member");
        state
            .add_project(OrganizationProject {
                project_id: "p".into(),
                organization_id: "org".into(),
                classification: "internal".into(),
            })
            .expect("project");
        let receipt = execute_governance_operation(
            state,
            GovernanceOperation::Authorize {
                actor_id: "alice".into(),
                project_id: "p".into(),
                action: GovernedAction::ExecuteWorkflow,
                resource_digest: format!("sha256:{}", "c".repeat(64)),
                occurred_at: "2026-08-26T10:00:00Z".into(),
            },
        )
        .expect("execute");
        assert!(matches!(
            receipt.output,
            GovernanceOperationOutput::Decision(GovernanceDecision::Authorized { .. })
        ));
        assert_eq!(receipt.state.audit_events.len(), 1);
    }
}
