//! GeneGIS collaboration — map comments, project branches, CRDT-ready document export.

pub mod branch;
pub mod comment;
pub mod crdt;
pub mod document;
pub mod error;
pub mod remote;
pub mod session;

pub use branch::ProjectBranch;
pub use comment::MapComment;
pub use crdt::{CollabApiPayload, CollabCrdt, CollabUpload};
pub use document::{CollabDocument, COLLAB_SCHEMA_VERSION};
pub use error::CollabError;
pub use remote::{pull_session, push_session, DEFAULT_SERVER_URL};
pub use session::CollabSession;
