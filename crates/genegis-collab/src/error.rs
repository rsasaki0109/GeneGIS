use thiserror::Error;

/// Errors raised by collaboration document IO.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CollabError {
    /// JSON parse or serialize failed.
    #[error("collab JSON error: {0}")]
    Json(String),
    /// Requested branch was not found.
    #[error("branch not found: {0}")]
    BranchNotFound(String),
    /// Branch name is invalid or already exists.
    #[error("invalid branch: {0}")]
    InvalidBranch(String),
    /// Comment validation failed.
    #[error("invalid comment: {0}")]
    InvalidComment(String),
}
