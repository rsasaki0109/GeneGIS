use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::CollabError;

/// Named project branch for style / workflow experiments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectBranch {
    pub id: Uuid,
    pub name: String,
    pub parent: Option<String>,
    pub created_at: DateTime<Utc>,
    pub description: String,
}

impl ProjectBranch {
    pub fn main() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: "main".into(),
            parent: None,
            created_at: Utc::now(),
            description: "Default collaboration branch".into(),
        }
    }

    pub fn child(name: impl Into<String>, parent: impl Into<String>) -> Result<Self, CollabError> {
        let name = name.into();
        validate_branch_name(&name)?;
        Ok(Self {
            id: Uuid::new_v4(),
            name,
            parent: Some(parent.into()),
            created_at: Utc::now(),
            description: String::new(),
        })
    }
}

pub(crate) fn validate_branch_name(name: &str) -> Result<(), CollabError> {
    if name.is_empty() {
        return Err(CollabError::InvalidBranch(
            "branch name must not be empty".into(),
        ));
    }
    let valid = name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');
    if !valid {
        return Err(CollabError::InvalidBranch(
            "branch name must use ASCII letters, digits, hyphen, underscore".into(),
        ));
    }
    Ok(())
}
