//! Command + Workflow boundary for signed plugin registry mutations.

use std::sync::Mutex;

use genegis_core::{
    Command, CommandBus, CommandEnvelope, CommandOrigin, InputSnapshot, Project, WorkflowDigest,
    WorkflowExecution, WorkflowExecutionContext, WorkflowExecutionError, WorkflowExecutionEvent,
    WorkflowExecutor,
};
use genegis_crs::{ChecksumVerification, SourceSnapshot};
use genegis_plugin_api::PluginManifest;
use genegis_workflow::{plugin_registry_operation_template, GeoWorkflow};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{PluginHostError, PluginRegistry, PluginReleaseSignature, PluginRevocation};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "operation")]
pub enum PluginRegistryOperation {
    Publish {
        manifest: PluginManifest,
        signature: PluginReleaseSignature,
        published_at: String,
    },
    Revoke {
        revocation: PluginRevocation,
    },
}

impl PluginRegistryOperation {
    fn name(&self) -> &'static str {
        match self {
            Self::Publish { .. } => "publish",
            Self::Revoke { .. } => "revoke",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRegistryOperationReceipt {
    pub command_id: String,
    pub workflow_digest: WorkflowDigest,
    pub registry_digest: String,
    pub registry: PluginRegistry,
}

struct RegistryExecutor {
    registry: Mutex<Option<PluginRegistry>>,
    operation: PluginRegistryOperation,
}

impl WorkflowExecutor for RegistryExecutor {
    fn execute(
        &self,
        _workflow: &GeoWorkflow,
        context: &WorkflowExecutionContext,
    ) -> Result<WorkflowExecution, WorkflowExecutionError> {
        let mut registry = self
            .registry
            .lock()
            .map_err(|_| WorkflowExecutionError::Failed("plugin registry lock poisoned".into()))?
            .take()
            .ok_or_else(|| WorkflowExecutionError::Failed("plugin registry is absent".into()))?;
        match &self.operation {
            PluginRegistryOperation::Publish {
                manifest,
                signature,
                published_at,
            } => {
                registry
                    .publish(manifest.clone(), signature.clone(), published_at)
                    .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
            }
            PluginRegistryOperation::Revoke { revocation } => {
                registry
                    .revoke(revocation.clone())
                    .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?
            }
        }
        let result_digest =
            digest(&registry).map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        *self.registry.lock().map_err(|_| {
            WorkflowExecutionError::Failed("plugin registry lock poisoned".into())
        })? = Some(registry.clone());
        Ok(WorkflowExecution {
            result_digest,
            output: serde_json::json!({"operation": self.operation.name()}),
            evidence: serde_json::to_value(&registry)
                .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?,
            events: vec![WorkflowExecutionEvent {
                kind: "plugin_registry_updated".into(),
                source_uri: Some("plugin-registry://v1".into()),
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

pub fn execute_plugin_registry_operation(
    registry: PluginRegistry,
    operation: PluginRegistryOperation,
) -> Result<PluginRegistryOperationReceipt, PluginHostError> {
    let state_digest = digest(&registry)?;
    let mut source = SourceSnapshot::new("plugin-registry://v1");
    source.checksum = Some(state_digest.clone());
    source.observed_checksum = Some(state_digest);
    source.checksum_status = ChecksumVerification::Verified;
    let workflow = plugin_registry_operation_template(source.clone(), operation.name());
    let workflow_digest = WorkflowDigest::new(
        workflow
            .stable_digest()
            .map_err(|error| PluginHostError::Bundle(error.to_string()))?,
    );
    let envelope = CommandEnvelope::new(
        CommandOrigin::Ui,
        Command::RunWorkflow {
            workflow_id: workflow.id,
        },
    )
    .with_workflow_digest(workflow_digest.clone())
    .with_source_snapshot(source.clone())
    .with_input_snapshot(InputSnapshot::new("plugin-registry", source));
    let command_id = envelope.id.to_string();
    let executor = RegistryExecutor {
        registry: Mutex::new(Some(registry)),
        operation,
    };
    let mut project = Project::new("SDK v1 plugin registry");
    let mut bus = CommandBus::new(project.clone());
    bus.register_workflow(workflow)
        .map_err(|error| PluginHostError::Bundle(error.to_string()))?;
    let execution = bus
        .apply_with_executor(&mut project, envelope, &executor)
        .map_err(|error| PluginHostError::Bundle(error.to_string()))?;
    let registry = executor
        .registry
        .into_inner()
        .map_err(|_| PluginHostError::Bundle("plugin registry lock poisoned".into()))?
        .ok_or_else(|| PluginHostError::Bundle("executor returned no plugin registry".into()))?;
    let registry_digest = digest(&registry)?;
    if execution.result_digest.as_deref() != Some(registry_digest.as_str()) {
        return Err(PluginHostError::Bundle(
            "CommandBus and plugin registry digests differ".into(),
        ));
    }
    Ok(PluginRegistryOperationReceipt {
        command_id,
        workflow_digest,
        registry_digest,
        registry,
    })
}

fn digest<T: Serialize>(value: &T) -> Result<String, PluginHostError> {
    let bytes =
        serde_json::to_vec(value).map_err(|error| PluginHostError::Bundle(error.to_string()))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use base64::{engine::general_purpose::STANDARD, Engine};
    use ed25519_dalek::SigningKey;
    use genegis_plugin_api::demo_manifest;

    use crate::{sign_plugin_release, PluginRegistryPolicy};

    use super::*;

    #[test]
    fn publication_is_command_and_workflow_bound() {
        let seed = [4_u8; 32];
        let key = SigningKey::from_bytes(&seed);
        let registry = PluginRegistry::new(PluginRegistryPolicy {
            schema_version: "1.0.0".into(),
            trusted_signers: BTreeMap::from([(
                "release".into(),
                STANDARD.encode(key.verifying_key().as_bytes()),
            )]),
        })
        .expect("registry");
        let manifest = demo_manifest();
        let signature = sign_plugin_release(&manifest, "release", &seed).expect("signature");
        let receipt = execute_plugin_registry_operation(
            registry,
            PluginRegistryOperation::Publish {
                manifest,
                signature,
                published_at: "2026-08-26T10:00:00Z".into(),
            },
        )
        .expect("publish workflow");
        assert_eq!(receipt.registry.entries.len(), 1);
        assert!(receipt.workflow_digest.as_str().starts_with("sha256:"));
    }
}
