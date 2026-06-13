//! GeoWorkflow IR — the intermediate representation for analysis and AI planning.
//!
//! AI generates workflows first; verified execution follows.

pub mod graph;
pub mod operation;
pub mod review;
pub mod step;

pub use graph::{
    local_cog_metadata_template, nagoya_geoparquet_template, nagoya_population_density_template,
    remote_cog_metadata_template, Citation, GeoWorkflow,
};
pub use operation::{OperationDescriptor, OperationId};
pub use review::ReviewStatus;
pub use step::{WorkflowStep, WorkflowStepId};
