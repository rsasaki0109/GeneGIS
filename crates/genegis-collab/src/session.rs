use genegis_core::Project;

use crate::branch::{validate_branch_name, ProjectBranch};
use crate::comment::MapComment;
use crate::document::CollabDocument;
use crate::error::CollabError;

/// In-memory collaboration session (Phase 5 alpha — file export today, CRDT sync later).
#[derive(Debug, Clone)]
pub struct CollabSession {
    document: CollabDocument,
}

impl CollabSession {
    pub fn new(project: Project) -> Self {
        Self {
            document: CollabDocument::new(project),
        }
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

    pub fn document(&self) -> &CollabDocument {
        &self.document
    }

    pub fn comments(&self) -> &[MapComment] {
        &self.document.comments
    }

    pub fn branches(&self) -> &[ProjectBranch] {
        &self.document.branches
    }

    pub fn active_branch(&self) -> &str {
        &self.document.active_branch
    }

    pub fn add_comment(&mut self, comment: MapComment) -> Result<&MapComment, CollabError> {
        comment.validate()?;
        self.document.comments.push(comment);
        Ok(self.document.comments.last().expect("comment"))
    }

    pub fn create_branch(
        &mut self,
        name: impl Into<String>,
        from: Option<&str>,
    ) -> Result<&ProjectBranch, CollabError> {
        let name = name.into();
        validate_branch_name(&name)?;
        if self.document.branches.iter().any(|branch| branch.name == name) {
            return Err(CollabError::InvalidBranch(format!(
                "branch already exists: {name}"
            )));
        }

        let parent_name = from.unwrap_or(&self.document.active_branch);
        if !self
            .document
            .branches
            .iter()
            .any(|branch| branch.name == parent_name)
        {
            return Err(CollabError::BranchNotFound(parent_name.into()));
        }

        let branch = ProjectBranch::child(name, parent_name)?;
        self.document.branches.push(branch);
        Ok(self.document.branches.last().expect("branch"))
    }

    pub fn export_json(&self) -> Result<String, CollabError> {
        self.document.to_json_pretty()
    }

    pub fn import_json(json: &str) -> Result<Self, CollabError> {
        Ok(Self {
            document: CollabDocument::from_json(json)?,
        })
    }

    pub fn summary_json(&self) -> serde_json::Value {
        self.document.summary_json()
    }

    pub fn comments_json(&self) -> serde_json::Value {
        serde_json::json!(self.document.comments)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_comment_and_branch() {
        let mut session = CollabSession::demo_nagoya();
        assert_eq!(session.comments().len(), 1);

        session
            .create_branch("experiment-style", Some("main"))
            .expect("branch");
        assert_eq!(session.branches().len(), 2);
    }

    #[test]
    fn round_trips_json_export() {
        let session = CollabSession::demo_nagoya();
        let json = session.export_json().expect("export");
        let restored = CollabSession::import_json(&json).expect("import");
        assert_eq!(restored.comments().len(), session.comments().len());
    }
}
