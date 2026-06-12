//! GeneGIS collaboration — map comments, project branches, CRDT-ready document export.

pub mod branch;
pub mod comment;
pub mod document;
pub mod error;
pub mod session;

pub use branch::ProjectBranch;
pub use comment::MapComment;
pub use document::{CollabDocument, COLLAB_SCHEMA_VERSION};
pub use error::CollabError;
pub use session::CollabSession;
