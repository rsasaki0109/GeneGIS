use genegis_core::Project;
use serde::{Deserialize, Serialize};

use crate::branch::ProjectBranch;
use crate::comment::MapComment;
use crate::error::CollabError;

/// Collab API schema version (CRDT-ready JSON envelope).
pub const COLLAB_SCHEMA_VERSION: u32 = 1;

/// Serializable collaboration state bundled with a core project snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollabDocument {
    pub schema_version: u32,
    pub active_branch: String,
    pub project: Project,
    pub comments: Vec<MapComment>,
    pub branches: Vec<ProjectBranch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automerge_snapshot: Option<String>,
}

impl CollabDocument {
    pub fn new(project: Project) -> Self {
        Self {
            schema_version: COLLAB_SCHEMA_VERSION,
            active_branch: "main".into(),
            project,
            comments: Vec::new(),
            branches: vec![ProjectBranch::main()],
            automerge_snapshot: None,
        }
    }

    pub fn from_json(json: &str) -> Result<Self, CollabError> {
        serde_json::from_str(json).map_err(|err| CollabError::Json(err.to_string()))
    }

    pub fn to_json_pretty(&self) -> Result<String, CollabError> {
        serde_json::to_string_pretty(self).map_err(|err| CollabError::Json(err.to_string()))
    }

    pub fn summary_json(&self) -> serde_json::Value {
        serde_json::json!({
            "schema_version": self.schema_version,
            "active_branch": self.active_branch,
            "project_name": self.project.workspace().name,
            "comment_count": self.comments.len(),
            "branch_count": self.branches.len(),
            "provenance_count": self.project.workspace().provenance.entries.len(),
            "branches": self.branches.iter().map(|b| &b.name).collect::<Vec<_>>(),
        })
    }
}
