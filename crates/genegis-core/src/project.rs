use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::Workspace;

/// On-disk workspace manifest (`.genegis/project.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectManifest {
    pub schema_version: u32,
    pub workspace: Workspace,
}

impl ProjectManifest {
    pub const CURRENT_SCHEMA_VERSION: u32 = 1;

    pub fn new(workspace: Workspace) -> Self {
        Self {
            schema_version: Self::CURRENT_SCHEMA_VERSION,
            workspace,
        }
    }
}

/// In-memory project handle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub manifest: ProjectManifest,
    pub path: Option<String>,
}

impl Project {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            manifest: ProjectManifest::new(Workspace::new(name)),
            path: None,
        }
    }

    pub fn workspace(&self) -> &Workspace {
        &self.manifest.workspace
    }

    pub fn workspace_mut(&mut self) -> &mut Workspace {
        &mut self.manifest.workspace
    }

    /// Return a canonical JSON representation of the semantic project state.
    ///
    /// Runtime timestamps and the optional in-memory project path are omitted
    /// because they are execution details rather than project state. Object
    /// keys are sorted recursively so the representation is stable across
    /// serde implementations and persistence round-trips.
    pub fn canonical_state_json(&self) -> serde_json::Value {
        let mut value = serde_json::to_value(self).expect("Project is serializable");
        strip_runtime_fields(&mut value);
        value
    }

    /// Return the SHA-256 digest of the canonical semantic project state.
    pub fn state_digest(&self) -> String {
        let canonical = canonical_json(&self.canonical_state_json());
        format!("sha256:{:x}", Sha256::digest(canonical.as_bytes()))
    }
}

fn strip_runtime_fields(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for field in [
                "created_at",
                "updated_at",
                "timestamp",
                "command_timestamp",
                "observed_at",
                "retrieved_at",
                "path",
            ] {
                map.remove(field);
            }
            for value in map.values_mut() {
                strip_runtime_fields(value);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                strip_runtime_fields(value);
            }
        }
        _ => {}
    }
}

fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by_key(|(key, _)| *key);
            let mut output = String::from("{");
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(&serde_json::to_string(key).expect("JSON key serialization"));
                output.push(':');
                output.push_str(&canonical_json(value));
            }
            output.push('}');
            output
        }
        serde_json::Value::Array(values) => {
            let values = values.iter().map(canonical_json).collect::<Vec<_>>();
            format!("[{}]", values.join(","))
        }
        _ => serde_json::to_string(value).expect("JSON scalar serialization"),
    }
}

/// Append-only provenance record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceEntry {
    pub id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub actor: String,
    pub action: String,
    pub target: String,
    pub details: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_run_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ProvenanceStore {
    pub entries: Vec<ProvenanceEntry>,
}

impl ProvenanceStore {
    pub fn record_workflow(
        &mut self,
        workflow_id: impl Into<String>,
        actor: impl Into<String>,
        action: impl Into<String>,
        target: impl Into<String>,
        details: serde_json::Value,
    ) {
        self.entries.push(ProvenanceEntry {
            id: Uuid::new_v4(),
            timestamp: Utc::now(),
            actor: actor.into(),
            action: action.into(),
            target: target.into(),
            details,
            agent_run_id: None,
            workflow_id: Some(workflow_id.into()),
        });
    }

    pub fn record(
        &mut self,
        actor: impl Into<String>,
        action: impl Into<String>,
        target: impl Into<String>,
        details: serde_json::Value,
    ) {
        self.entries.push(ProvenanceEntry {
            id: Uuid::new_v4(),
            timestamp: Utc::now(),
            actor: actor.into(),
            action: action.into(),
            target: target.into(),
            details,
            agent_run_id: None,
            workflow_id: None,
        });
    }

    pub fn record_agent_run(
        &mut self,
        run_id: Uuid,
        workflow_id: impl Into<String>,
        actor: impl Into<String>,
        action: impl Into<String>,
        details: serde_json::Value,
    ) {
        let workflow_id = workflow_id.into();
        self.entries.push(ProvenanceEntry {
            id: Uuid::new_v4(),
            timestamp: Utc::now(),
            actor: actor.into(),
            action: action.into(),
            target: workflow_id.clone(),
            details,
            agent_run_id: Some(run_id),
            workflow_id: Some(workflow_id),
        });
    }
}
