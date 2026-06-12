use genegis_core::Project;
use uuid::Uuid;

use crate::branch::{validate_branch_name, ProjectBranch};
use crate::comment::MapComment;
use crate::crdt::CollabCrdt;
use crate::document::CollabDocument;
use crate::error::CollabError;

/// In-memory collaboration session backed by Automerge CRDT (Phase 5 beta).
#[derive(Debug)]
pub struct CollabSession {
    crdt: CollabCrdt,
}

impl CollabSession {
    pub fn new(project: Project) -> Self {
        Self::from_document(CollabDocument::new(project)).expect("seed collab session")
    }

    /// Demo session seeded for the Nagoya north-star workbench.
    pub fn demo_nagoya() -> Self {
        let mut session = Self::new(Project::new("Nagoya density review"));
        session
            .add_comment(
                MapComment::new("reviewer", "Verify 中区 ward boundary alignment")
                    .with_map_anchor(136.906, 35.168),
            )
            .expect("demo comment");
        session
    }

    pub fn from_crdt(crdt: CollabCrdt) -> Self {
        Self { crdt }
    }

    pub fn from_document(document: CollabDocument) -> Result<Self, CollabError> {
        Ok(Self {
            crdt: CollabCrdt::from_document(&document)?,
        })
    }

    pub fn from_snapshot(bytes: &[u8]) -> Result<Self, CollabError> {
        Ok(Self {
            crdt: CollabCrdt::load(bytes)?,
        })
    }

    pub fn crdt(&self) -> &CollabCrdt {
        &self.crdt
    }

    pub fn crdt_mut(&mut self) -> &mut CollabCrdt {
        &mut self.crdt
    }

    pub fn document(&self) -> Result<CollabDocument, CollabError> {
        self.crdt.project_document()
    }

    pub fn comments(&self) -> Result<Vec<MapComment>, CollabError> {
        Ok(self.document()?.comments)
    }

    pub fn branches(&self) -> Result<Vec<ProjectBranch>, CollabError> {
        Ok(self.document()?.branches)
    }

    pub fn active_branch(&self) -> Result<String, CollabError> {
        Ok(self.document()?.active_branch)
    }

    pub fn add_comment(&mut self, comment: MapComment) -> Result<MapComment, CollabError> {
        comment.validate()?;
        self.crdt.add_comment(comment.clone())?;
        Ok(comment)
    }

    pub fn add_agent_comment(
        &mut self,
        run_id: Uuid,
        step_id: Uuid,
        author: impl Into<String>,
        body: impl Into<String>,
    ) -> Result<MapComment, CollabError> {
        let comment = MapComment::new(author, body).with_agent_context(run_id, step_id);
        self.add_comment(comment)
    }

    pub fn create_branch(
        &mut self,
        name: impl Into<String>,
        from: Option<&str>,
    ) -> Result<ProjectBranch, CollabError> {
        let name = name.into();
        validate_branch_name(&name)?;
        let document = self.document()?;
        if document.branches.iter().any(|branch| branch.name == name) {
            return Err(CollabError::InvalidBranch(format!(
                "branch already exists: {name}"
            )));
        }

        let parent_name = from.unwrap_or(&document.active_branch);
        if !document
            .branches
            .iter()
            .any(|branch| branch.name == parent_name)
        {
            return Err(CollabError::BranchNotFound(parent_name.into()));
        }

        let branch = ProjectBranch::child(name, parent_name)?;
        self.crdt.create_branch(branch.clone())?;
        Ok(branch)
    }

    pub fn merge_snapshot_base64(&mut self, encoded: &str) -> Result<(), CollabError> {
        self.crdt.merge_snapshot_base64(encoded)
    }

    pub fn merge_session(&mut self, other: &mut CollabSession) -> Result<(), CollabError> {
        self.crdt.merge(&mut other.crdt)
    }

    pub fn merge_json(&mut self, json: &str) -> Result<(), CollabError> {
        let incoming = CollabDocument::from_json(json)?;
        if let Some(snapshot) = incoming.automerge_snapshot {
            self.merge_snapshot_base64(&snapshot)?;
        }
        for comment in incoming.comments {
            self.crdt.add_comment(comment)?;
        }
        Ok(())
    }

    pub fn export_json(&self) -> Result<String, CollabError> {
        self.document()?.to_json_pretty()
    }

    pub fn export_json_with_snapshot(&mut self) -> Result<String, CollabError> {
        let mut document = self.document()?;
        document.automerge_snapshot = Some(self.snapshot_base64()?);
        document.to_json_pretty()
    }

    pub fn import_json(json: &str) -> Result<Self, CollabError> {
        let document = CollabDocument::from_json(json)?;
        if let Some(snapshot) = document.automerge_snapshot.clone() {
            let mut session = Self::from_document(document)?;
            session.merge_snapshot_base64(&snapshot)?;
            Ok(session)
        } else {
            Self::from_document(document)
        }
    }

    pub fn snapshot_bytes(&mut self) -> Vec<u8> {
        self.crdt.save()
    }

    pub fn snapshot_base64(&mut self) -> Result<String, CollabError> {
        Ok(self.crdt.snapshot_base64())
    }

    pub fn summary_json(&self) -> Result<serde_json::Value, CollabError> {
        Ok(self.document()?.summary_json())
    }

    pub fn comments_json(&self) -> Result<serde_json::Value, CollabError> {
        Ok(serde_json::json!(self.document()?.comments))
    }
}

impl Clone for CollabSession {
    fn clone(&self) -> Self {
        let document = self.document().expect("collab document");
        Self::from_document(document).expect("clone collab session")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_comment_and_branch() {
        let mut session = CollabSession::demo_nagoya();
        assert_eq!(session.comments().expect("comments").len(), 1);

        session
            .create_branch("experiment-style", Some("main"))
            .expect("branch");
        assert_eq!(session.branches().expect("branches").len(), 2);
    }

    #[test]
    fn round_trips_json_export() {
        let session = CollabSession::demo_nagoya();
        let json = session.export_json().expect("export");
        let restored = CollabSession::import_json(&json).expect("import");
        assert_eq!(
            restored.comments().expect("comments").len(),
            session.comments().expect("comments").len()
        );
    }

    #[test]
    fn merges_incoming_json_comments() {
        let mut server = CollabSession::demo_nagoya();
        let mut client = CollabSession::from_document(CollabDocument::new(Project::new(
            "Nagoya density review",
        )))
        .expect("client");
        client
            .add_comment(MapComment::new("cli", "CLI-side review note"))
            .expect("comment");

        server
            .merge_json(&client.export_json().expect("json"))
            .expect("merge");
        assert_eq!(server.comments().expect("comments").len(), 2);
    }
}
