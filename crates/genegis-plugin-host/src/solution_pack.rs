//! Portable domain packs composed from reviewed workflows and signed SDK v1 plugins.

use std::collections::BTreeSet;
use std::sync::Mutex;

use genegis_core::{
    Command, CommandBus, CommandEnvelope, CommandOrigin, InputSnapshot, Project, WorkflowDigest,
    WorkflowExecution, WorkflowExecutionContext, WorkflowExecutionError, WorkflowExecutionEvent,
    WorkflowExecutor,
};
use genegis_crs::{ChecksumVerification, SourceSnapshot};
use genegis_plugin_api::PluginCapability;
use genegis_workflow::{
    reviewed_workflow_templates, solution_pack_admission_template, GeoWorkflow,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{PluginHostError, PluginRegistry};

/// First-party domain families implemented without core forks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SolutionDomain {
    Urban,
    Environment,
    Disaster,
    Mobility,
    Infrastructure,
}

/// Exact active SDK v1 plugin release required by a pack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolutionPluginRequirement {
    pub plugin_id: String,
    pub version: String,
    pub capabilities: Vec<PluginCapability>,
}

/// Authoring form loaded from `packs/*/pack.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolutionPackDraft {
    pub schema_version: String,
    pub id: String,
    pub version: String,
    pub domain: SolutionDomain,
    pub title: String,
    pub license: String,
    pub template_ids: Vec<String>,
    pub plugin_requirements: Vec<SolutionPluginRequirement>,
    pub open_state_formats: BTreeSet<String>,
}

/// Sealed pack identity admitted by the host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolutionPackManifest {
    #[serde(flatten)]
    pub draft: SolutionPackDraft,
    pub pack_digest: String,
}

/// Command-backed pack admission receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolutionPackAdmissionReceipt {
    pub command_id: String,
    pub workflow_digest: WorkflowDigest,
    pub pack: SolutionPackManifest,
}

/// Seal a structurally valid pack before registry admission.
pub fn seal_solution_pack(
    draft: SolutionPackDraft,
) -> Result<SolutionPackManifest, PluginHostError> {
    validate_draft(&draft)?;
    Ok(SolutionPackManifest {
        pack_digest: digest(&draft)?,
        draft,
    })
}

/// Verify reviewed template IDs and active signed plugin releases.
pub fn verify_solution_pack(
    pack: &SolutionPackManifest,
    registry: &PluginRegistry,
) -> Result<(), PluginHostError> {
    validate_draft(&pack.draft)?;
    if digest(&pack.draft)? != pack.pack_digest {
        return Err(PluginHostError::Bundle(
            "solution pack digest mismatch".into(),
        ));
    }
    let reviewed = reviewed_workflow_templates()
        .into_iter()
        .map(|template| template.id)
        .collect::<BTreeSet<_>>();
    if pack
        .draft
        .template_ids
        .iter()
        .any(|id| !reviewed.contains(id))
    {
        return Err(PluginHostError::Bundle(
            "solution pack references an unreviewed workflow template".into(),
        ));
    }
    for requirement in &pack.draft.plugin_requirements {
        let entry = registry.resolve(&requirement.plugin_id, &requirement.version)?;
        if requirement
            .capabilities
            .iter()
            .any(|capability| !entry.manifest.capabilities.contains(capability))
        {
            return Err(PluginHostError::Bundle(
                "solution pack plugin lacks a required capability".into(),
            ));
        }
    }
    Ok(())
}

struct PackExecutor {
    pack: SolutionPackManifest,
    registry: PluginRegistry,
    accepted: Mutex<bool>,
}

impl WorkflowExecutor for PackExecutor {
    fn execute(
        &self,
        _workflow: &GeoWorkflow,
        context: &WorkflowExecutionContext,
    ) -> Result<WorkflowExecution, WorkflowExecutionError> {
        verify_solution_pack(&self.pack, &self.registry)
            .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        *self
            .accepted
            .lock()
            .map_err(|_| WorkflowExecutionError::Failed("solution pack lock poisoned".into()))? =
            true;
        Ok(WorkflowExecution {
            result_digest: self.pack.pack_digest.clone(),
            output: serde_json::json!({
                "pack_id": self.pack.draft.id,
                "domain": self.pack.draft.domain,
                "templates": self.pack.draft.template_ids.len(),
                "plugins": self.pack.draft.plugin_requirements.len(),
            }),
            evidence: serde_json::to_value(&self.pack)
                .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?,
            events: vec![WorkflowExecutionEvent {
                kind: "solution_pack_admitted".into(),
                source_uri: Some(format!("solution-pack://{}", self.pack.draft.id)),
                observed_at: context.command_timestamp,
                details: serde_json::json!({
                    "command_id": context.command_id,
                    "workflow_digest": context.workflow_digest,
                }),
            }],
        })
    }
}

/// Admit one pack exclusively through Command + Workflow Graph.
pub fn admit_solution_pack_workflow(
    pack: SolutionPackManifest,
    registry: PluginRegistry,
) -> Result<SolutionPackAdmissionReceipt, PluginHostError> {
    let mut source = SourceSnapshot::new(format!("solution-pack://{}", pack.draft.id));
    source.checksum = Some(pack.pack_digest.clone());
    source.observed_checksum = Some(pack.pack_digest.clone());
    source.checksum_status = ChecksumVerification::Verified;
    let workflow = solution_pack_admission_template(source.clone());
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
    .with_input_snapshot(InputSnapshot::new("solution-pack", source));
    let command_id = envelope.id.to_string();
    let executor = PackExecutor {
        pack: pack.clone(),
        registry,
        accepted: Mutex::new(false),
    };
    let mut project = Project::new("Domain solution pack admission");
    let mut bus = CommandBus::new(project.clone());
    bus.register_workflow(workflow)
        .map_err(|error| PluginHostError::Bundle(error.to_string()))?;
    let execution = bus
        .apply_with_executor(&mut project, envelope, &executor)
        .map_err(|error| PluginHostError::Bundle(error.to_string()))?;
    if !executor
        .accepted
        .into_inner()
        .map_err(|_| PluginHostError::Bundle("solution pack lock poisoned".into()))?
        || execution.result_digest.as_deref() != Some(pack.pack_digest.as_str())
    {
        return Err(PluginHostError::Bundle(
            "solution pack admission receipt mismatch".into(),
        ));
    }
    Ok(SolutionPackAdmissionReceipt {
        command_id,
        workflow_digest,
        pack,
    })
}

fn validate_draft(draft: &SolutionPackDraft) -> Result<(), PluginHostError> {
    let required_formats = BTreeSet::from([
        "genegis-project-json".into(),
        "workflow-json".into(),
        "result-capsule".into(),
    ]);
    let template_ids = draft.template_ids.iter().collect::<BTreeSet<_>>();
    let plugin_ids = draft
        .plugin_requirements
        .iter()
        .map(|item| (&item.plugin_id, &item.version))
        .collect::<BTreeSet<_>>();
    if draft.schema_version != "1.0.0"
        || draft.id.trim().is_empty()
        || draft.title.trim().is_empty()
        || draft.license.trim().is_empty()
        || !semver(&draft.version)
        || draft.template_ids.is_empty()
        || draft.plugin_requirements.is_empty()
        || template_ids.len() != draft.template_ids.len()
        || plugin_ids.len() != draft.plugin_requirements.len()
        || !required_formats.is_subset(&draft.open_state_formats)
        || draft.plugin_requirements.iter().any(|item| {
            item.plugin_id.trim().is_empty()
                || !semver(&item.version)
                || item.capabilities.is_empty()
        })
    {
        return Err(PluginHostError::Bundle(
            "solution pack draft is invalid or introduces non-portable state".into(),
        ));
    }
    Ok(())
}

fn semver(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
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
    use genegis_plugin_api::{PluginManifest, PLUGIN_API_VERSION};

    use crate::{sign_plugin_release, PluginRegistryPolicy};

    use super::*;

    #[test]
    fn five_domain_packs_use_only_reviewed_templates_and_signed_plugins() {
        let drafts: Vec<SolutionPackDraft> = [
            include_str!("../../../packs/urban/pack.json"),
            include_str!("../../../packs/environment/pack.json"),
            include_str!("../../../packs/disaster/pack.json"),
            include_str!("../../../packs/mobility/pack.json"),
            include_str!("../../../packs/infrastructure/pack.json"),
        ]
        .into_iter()
        .map(|json| serde_json::from_str(json).expect("pack JSON"))
        .collect();
        let seed = [5_u8; 32];
        let signing = SigningKey::from_bytes(&seed);
        let mut registry = PluginRegistry::new(PluginRegistryPolicy {
            schema_version: "1.0.0".into(),
            trusted_signers: BTreeMap::from([(
                "first-party".into(),
                STANDARD.encode(signing.verifying_key().as_bytes()),
            )]),
        })
        .expect("registry");
        for requirement in drafts
            .iter()
            .flat_map(|draft| draft.plugin_requirements.iter())
        {
            let manifest = PluginManifest {
                id: requirement.plugin_id.clone(),
                name: requirement.plugin_id.clone(),
                version: requirement.version.clone(),
                api_version: PLUGIN_API_VERSION.into(),
                description: "solution pack fixture plugin".into(),
                author: "GeneGIS".into(),
                capabilities: requirement.capabilities.clone(),
                artifact_digest: format!("sha256:{}", "0".repeat(64)),
                wasm: None,
            };
            let signature = sign_plugin_release(&manifest, "first-party", &seed).expect("sign");
            registry
                .publish(manifest, signature, "2026-08-26T10:00:00Z")
                .expect("publish");
        }
        let mut domains = BTreeSet::new();
        for draft in drafts {
            domains.insert(draft.domain);
            let pack = seal_solution_pack(draft).expect("seal");
            admit_solution_pack_workflow(pack, registry.clone()).expect("admit");
        }
        assert_eq!(domains.len(), 5);
    }

    #[test]
    fn rejects_unreviewed_template_and_revoked_plugin() {
        let mut draft: SolutionPackDraft =
            serde_json::from_str(include_str!("../../../packs/urban/pack.json")).expect("pack");
        draft.template_ids.push("core-fork".into());
        let pack = seal_solution_pack(draft).expect("structurally seal");
        let registry = PluginRegistry {
            schema_version: "1.0.0".into(),
            policy: crate::PluginRegistryPolicy {
                schema_version: "1.0.0".into(),
                trusted_signers: BTreeMap::new(),
            },
            entries: BTreeMap::new(),
        };
        assert!(verify_solution_pack(&pack, &registry).is_err());
    }
}
