use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
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
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ProvenanceStore {
    pub entries: Vec<ProvenanceEntry>,
}

impl ProvenanceStore {
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
        });
    }
}
