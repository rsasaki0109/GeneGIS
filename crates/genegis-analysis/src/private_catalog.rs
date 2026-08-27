//! Command + Workflow admission for access-bound private catalog federation.

use std::sync::Mutex;

use genegis_catalog::{
    admit_private_federation, CatalogAccessContext, FederatedCatalogPolicy,
    PrivateFederationAdmission, StacEndpoint,
};
use genegis_core::{
    Command, CommandBus, CommandEnvelope, CommandOrigin, InputSnapshot, Project, WorkflowDigest,
    WorkflowExecution, WorkflowExecutionContext, WorkflowExecutionError, WorkflowExecutionEvent,
    WorkflowExecutor,
};
use genegis_crs::{ChecksumVerification, SourceSnapshot};
use genegis_workflow::{private_federated_catalog_template, GeoWorkflow};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::AnalysisError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateCatalogAdmissionReceipt {
    pub command_id: String,
    pub workflow_digest: WorkflowDigest,
    pub admission: PrivateFederationAdmission,
}

struct PrivateCatalogExecutor {
    endpoints: Vec<StacEndpoint>,
    policy: FederatedCatalogPolicy,
    context: CatalogAccessContext,
    admission: Mutex<Option<PrivateFederationAdmission>>,
}

impl WorkflowExecutor for PrivateCatalogExecutor {
    fn execute(
        &self,
        _workflow: &GeoWorkflow,
        context: &WorkflowExecutionContext,
    ) -> Result<WorkflowExecution, WorkflowExecutionError> {
        let admission = admit_private_federation(&self.endpoints, &self.policy, &self.context)
            .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        let result_digest = admission.admission_digest.clone();
        *self.admission.lock().map_err(|_| {
            WorkflowExecutionError::Failed("catalog admission lock poisoned".into())
        })? = Some(admission.clone());
        Ok(WorkflowExecution {
            result_digest,
            output: serde_json::json!({
                "admitted_count": admission.admitted.len(),
                "denied_count": admission.denied_count,
            }),
            evidence: serde_json::to_value(&admission)
                .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?,
            events: vec![WorkflowExecutionEvent {
                kind: "private_catalog_federation_admitted".into(),
                source_uri: Some("catalog-policy://federation".into()),
                observed_at: context.command_timestamp,
                details: serde_json::json!({
                    "command_id": context.command_id,
                    "workflow_digest": context.workflow_digest,
                }),
            }],
        })
    }
}

pub fn admit_private_catalog_workflow(
    endpoints: Vec<StacEndpoint>,
    policy: FederatedCatalogPolicy,
    context: CatalogAccessContext,
) -> Result<PrivateCatalogAdmissionReceipt, AnalysisError> {
    let policy_digest = digest(&policy)?;
    let mut source = SourceSnapshot::new("catalog-policy://federation");
    source.checksum = Some(policy_digest.clone());
    source.observed_checksum = Some(policy_digest);
    source.checksum_status = ChecksumVerification::Verified;
    let workflow = private_federated_catalog_template(source.clone());
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
    .with_input_snapshot(InputSnapshot::new("catalog-policy", source));
    let command_id = envelope.id.to_string();
    let executor = PrivateCatalogExecutor {
        endpoints,
        policy,
        context,
        admission: Mutex::new(None),
    };
    let mut project = Project::new("Private federated catalogs");
    let mut bus = CommandBus::new(project.clone());
    bus.register_workflow(workflow)
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    let execution = bus
        .apply_with_executor(&mut project, envelope, &executor)
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    let admission = executor
        .admission
        .into_inner()
        .map_err(|_| AnalysisError::Message("catalog admission lock poisoned".into()))?
        .ok_or_else(|| AnalysisError::Message("executor returned no catalog admission".into()))?;
    if execution.result_digest.as_deref() != Some(admission.admission_digest.as_str()) {
        return Err(AnalysisError::Message(
            "CommandBus and catalog admission digests differ".into(),
        ));
    }
    Ok(PrivateCatalogAdmissionReceipt {
        command_id,
        workflow_digest,
        admission,
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

    use genegis_catalog::{CatalogAccessRule, CatalogVisibility};

    use super::*;

    #[test]
    fn admission_is_bound_to_command_and_workflow() {
        let endpoint = StacEndpoint::new("public", "fixture://public-stac");
        let policy = FederatedCatalogPolicy {
            schema_version: "0.1.0".into(),
            policy_id: "catalog-policy".into(),
            policy_version: "1".into(),
            rules: BTreeMap::from([(
                "public".into(),
                CatalogAccessRule {
                    endpoint_id: "public".into(),
                    visibility: CatalogVisibility::Public,
                    organization_id: None,
                    allowed_subjects: BTreeSet::new(),
                    allowed_roles: BTreeSet::new(),
                },
            )]),
        };
        let receipt = admit_private_catalog_workflow(
            vec![endpoint],
            policy,
            CatalogAccessContext {
                subject_id: "visitor".into(),
                organization_id: None,
                roles: BTreeSet::new(),
            },
        )
        .expect("admission");
        assert_eq!(receipt.admission.admitted.len(), 1);
        assert!(receipt.workflow_digest.as_str().starts_with("sha256:"));
    }
}
