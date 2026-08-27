//! GeneGIS collaboration — map comments, project branches, CRDT-ready document export.

pub mod branch;
pub mod comment;
pub mod crdt;
pub mod document;
pub mod error;
pub mod governance;
pub mod remote;
pub mod session;

pub use branch::ProjectBranch;
pub use comment::MapComment;
pub use crdt::{CollabApiPayload, CollabCrdt, CollabUpload};
pub use document::{CollabDocument, COLLAB_SCHEMA_VERSION};
pub use error::CollabError;
pub use governance::{
    AuditEvent, AuditExport, GovernanceCapability, GovernanceDecision, GovernanceError,
    GovernanceState, GovernedAction, OrganizationMember, OrganizationPolicy, OrganizationProject,
    OrganizationRole, PendingApproval, RetentionDisposition, RetentionPolicy, RetentionRecord,
};
pub use remote::{pull_session, push_session, DEFAULT_SERVER_URL};
pub use session::CollabSession;
