use std::collections::BTreeMap;

use automerge::transaction::Transactable;
use automerge::{AutoCommit, ObjType, ReadDoc, ROOT};
use base64::Engine;
use genegis_core::Project;
use serde::{Deserialize, Serialize};

use crate::branch::ProjectBranch;
use crate::comment::MapComment;
use crate::document::{CollabDocument, COLLAB_SCHEMA_VERSION};
use crate::error::CollabError;

const KEY_SCHEMA_VERSION: &str = "schema_version";
const KEY_ACTIVE_BRANCH: &str = "active_branch";
const KEY_PROJECT_JSON: &str = "project_json";
const KEY_COMMENTS: &str = "comments";
const KEY_BRANCHES: &str = "branches";

/// Automerge-backed collaboration document (Phase 5 beta).
#[derive(Debug)]
pub struct CollabCrdt {
    doc: AutoCommit,
}

impl CollabCrdt {
    pub fn from_document(document: &CollabDocument) -> Result<Self, CollabError> {
        let mut crdt = Self {
            doc: AutoCommit::new(),
        };
        crdt.apply_document(document)?;
        Ok(crdt)
    }

    pub fn load(bytes: &[u8]) -> Result<Self, CollabError> {
        AutoCommit::load(bytes)
            .map(|doc| Self { doc })
            .map_err(|err| CollabError::Automerge(err.to_string()))
    }

    pub fn save(&mut self) -> Vec<u8> {
        self.doc.save()
    }

    pub fn merge(&mut self, other: &mut Self) -> Result<(), CollabError> {
        self.doc
            .merge(&mut other.doc)
            .map(|_| ())
            .map_err(|err| CollabError::Automerge(err.to_string()))
    }

    pub fn project_document(&self) -> Result<CollabDocument, CollabError> {
        let schema_version = read_u64(&self.doc, &ROOT, KEY_SCHEMA_VERSION)?
            .unwrap_or(COLLAB_SCHEMA_VERSION as u64) as u32;
        let active_branch = read_string(&self.doc, &ROOT, KEY_ACTIVE_BRANCH)?
            .unwrap_or_else(|| "main".into());
        let project_json = read_string(&self.doc, &ROOT, KEY_PROJECT_JSON)?
            .ok_or_else(|| CollabError::Automerge("missing project_json".into()))?;
        let project: Project =
            serde_json::from_str(&project_json).map_err(|err| CollabError::Json(err.to_string()))?;

        Ok(CollabDocument {
            schema_version,
            active_branch,
            project,
            comments: self.read_comments()?,
            branches: self.read_branches()?,
            automerge_snapshot: None,
        })
    }

    pub fn apply_document(&mut self, document: &CollabDocument) -> Result<(), CollabError> {
        self.doc
            .put(ROOT, KEY_SCHEMA_VERSION, document.schema_version as i64)
            .map_err(|err| CollabError::Automerge(err.to_string()))?;
        self.doc
            .put(ROOT, KEY_ACTIVE_BRANCH, document.active_branch.as_str())
            .map_err(|err| CollabError::Automerge(err.to_string()))?;
        let project_json = serde_json::to_string(&document.project)
            .map_err(|err| CollabError::Json(err.to_string()))?;
        self.doc
            .put(ROOT, KEY_PROJECT_JSON, project_json.as_str())
            .map_err(|err| CollabError::Automerge(err.to_string()))?;

        let comments = ensure_map(&mut self.doc, ROOT, KEY_COMMENTS)?;
        for comment in &document.comments {
            write_comment(&mut self.doc, &comments, comment)?;
        }

        let branches = ensure_map(&mut self.doc, ROOT, KEY_BRANCHES)?;
        for branch in &document.branches {
            write_branch(&mut self.doc, &branches, branch)?;
        }

        Ok(())
    }

    pub fn add_comment(&mut self, comment: MapComment) -> Result<(), CollabError> {
        comment.validate()?;
        let comments = ensure_map(&mut self.doc, ROOT, KEY_COMMENTS)?;
        write_comment(&mut self.doc, &comments, &comment)
    }

    pub fn create_branch(&mut self, branch: ProjectBranch) -> Result<(), CollabError> {
        let branches = ensure_map(&mut self.doc, ROOT, KEY_BRANCHES)?;
        write_branch(&mut self.doc, &branches, &branch)
    }

    pub fn set_active_branch(&mut self, name: &str) -> Result<(), CollabError> {
        self.doc
            .put(ROOT, KEY_ACTIVE_BRANCH, name)
            .map_err(|err| CollabError::Automerge(err.to_string()))
    }

    pub fn snapshot_base64(&mut self) -> String {
        base64::engine::general_purpose::STANDARD.encode(self.save())
    }

    pub fn merge_snapshot_base64(&mut self, encoded: &str) -> Result<(), CollabError> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|err| CollabError::Automerge(err.to_string()))?;
        let mut other = Self::load(&bytes)?;
        self.merge(&mut other)
    }

    fn read_comments(&self) -> Result<Vec<MapComment>, CollabError> {
        let Some(comments) = child_map(&self.doc, ROOT, KEY_COMMENTS)? else {
            return Ok(Vec::new());
        };

        let mut entries = BTreeMap::new();
        for key in self.doc.keys(&comments) {
            let Some(json) = read_string(&self.doc, &comments, &key)? else {
                continue;
            };
            let comment: MapComment =
                serde_json::from_str(&json).map_err(|err| CollabError::Json(err.to_string()))?;
            entries.insert(comment.id, comment);
        }

        Ok(entries.into_values().collect())
    }

    fn read_branches(&self) -> Result<Vec<ProjectBranch>, CollabError> {
        let Some(branches) = child_map(&self.doc, ROOT, KEY_BRANCHES)? else {
            return Ok(vec![ProjectBranch::main()]);
        };

        let mut entries = BTreeMap::new();
        for key in self.doc.keys(&branches) {
            let Some(json) = read_string(&self.doc, &branches, &key)? else {
                continue;
            };
            let branch: ProjectBranch =
                serde_json::from_str(&json).map_err(|err| CollabError::Json(err.to_string()))?;
            entries.insert(branch.name.clone(), branch);
        }

        if entries.is_empty() {
            return Ok(vec![ProjectBranch::main()]);
        }

        Ok(entries.into_values().collect())
    }
}

fn ensure_map(
    doc: &mut AutoCommit,
    parent: automerge::ObjId,
    key: &str,
) -> Result<automerge::ObjId, CollabError> {
    match doc
        .get(&parent, key)
        .map_err(|err| CollabError::Automerge(err.to_string()))?
    {
        Some((automerge::Value::Object(ObjType::Map), obj)) => Ok(obj),
        _ => doc
            .put_object(&parent, key, ObjType::Map)
            .map_err(|err| CollabError::Automerge(err.to_string())),
    }
}

fn write_comment(
    doc: &mut AutoCommit,
    comments: &automerge::ObjId,
    comment: &MapComment,
) -> Result<(), CollabError> {
    let json = serde_json::to_string(comment).map_err(|err| CollabError::Json(err.to_string()))?;
    doc.put(comments, comment.id.to_string(), json.as_str())
        .map_err(|err| CollabError::Automerge(err.to_string()))
}

fn write_branch(
    doc: &mut AutoCommit,
    branches: &automerge::ObjId,
    branch: &ProjectBranch,
) -> Result<(), CollabError> {
    let json = serde_json::to_string(branch).map_err(|err| CollabError::Json(err.to_string()))?;
    doc.put(branches, branch.name.as_str(), json.as_str())
        .map_err(|err| CollabError::Automerge(err.to_string()))
}

fn child_map(
    doc: &AutoCommit,
    parent: automerge::ObjId,
    key: &str,
) -> Result<Option<automerge::ObjId>, CollabError> {
    match doc
        .get(&parent, key)
        .map_err(|err| CollabError::Automerge(err.to_string()))?
    {
        Some((automerge::Value::Object(ObjType::Map), obj)) => Ok(Some(obj)),
        Some(_) => Err(CollabError::Automerge(format!("{key} must be a map"))),
        None => Ok(None),
    }
}

fn read_string(
    doc: &AutoCommit,
    obj: &automerge::ObjId,
    key: &str,
) -> Result<Option<String>, CollabError> {
    match doc
        .get(obj, key)
        .map_err(|err| CollabError::Automerge(err.to_string()))?
    {
        Some((automerge::Value::Scalar(value), _)) => {
            if let Some(text) = value.to_str() {
                return Ok(Some(text.to_string()));
            }
            if let Some(number) = value.to_i64() {
                return Ok(Some(number.to_string()));
            }
            Err(CollabError::Automerge(format!("{key} must be a string or integer")))
        }
        Some(_) => Err(CollabError::Automerge(format!("{key} must be a scalar"))),
        None => Ok(None),
    }
}

fn read_u64(
    doc: &AutoCommit,
    obj: &automerge::ObjId,
    key: &str,
) -> Result<Option<u64>, CollabError> {
    let Some(text) = read_string(doc, obj, key)? else {
        return Ok(None);
    };
    text.parse::<u64>()
        .map(Some)
        .map_err(|err| CollabError::Automerge(err.to_string()))
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CollabUpload {
    pub session: String,
    #[serde(default)]
    pub automerge_snapshot: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CollabApiPayload {
    pub ok: bool,
    pub summary: serde_json::Value,
    pub comments: serde_json::Value,
    pub session: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub automerge_snapshot: Option<String>,
}

impl CollabApiPayload {
    pub fn from_session(
        session: &crate::CollabSession,
        include_snapshot: bool,
    ) -> Result<Self, CollabError> {
        let mut session = session.clone();
        Ok(Self {
            ok: true,
            summary: session.summary_json()?,
            comments: session.comments_json()?,
            session: Some(session.export_json()?),
            automerge_snapshot: if include_snapshot {
                Some(session.snapshot_base64()?)
            } else {
                None
            },
        })
    }

    pub fn error(message: &str) -> Self {
        Self {
            ok: false,
            summary: serde_json::json!({ "error": message }),
            comments: serde_json::json!([]),
            session: None,
            automerge_snapshot: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CollabSession;

    #[test]
    fn merges_concurrent_comments() {
        let base = CollabCrdt::from_document(&CollabDocument::new(Project::new("Merge test")))
            .expect("seed");
        let saved = {
            let mut base = base;
            base.save()
        };

        let mut client_a = CollabCrdt::load(&saved).expect("load a");
        client_a
            .add_comment(MapComment::new("alice", "Check ward 中区"))
            .expect("comment a");

        let mut client_b = CollabCrdt::load(&saved).expect("load b");
        client_b
            .add_comment(MapComment::new("bob", "Verify density legend"))
            .expect("comment b");

        client_a.merge(&mut client_b).expect("merge");
        let document = client_a.project_document().expect("project");
        assert_eq!(document.comments.len(), 2);
    }

    #[test]
    fn round_trips_document_projection() {
        let session = CollabSession::demo_nagoya();
        let document = session.document().expect("document");
        let crdt = CollabCrdt::from_document(&document).expect("crdt");
        let projected = crdt.project_document().expect("project");
        assert_eq!(projected.comments.len(), document.comments.len());
        assert_eq!(projected.active_branch, document.active_branch);
    }
}
