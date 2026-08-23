//! GeoWorkflow IR — the intermediate representation for analysis and AI planning.
//!
//! AI generates workflows first; verified execution follows.

pub mod graph;
pub mod operation;
pub mod review;
pub mod step;

pub use graph::{
    external_stac_fetch_template, federated_asset_execution_template,
    federated_stac_search_template, local_cog_metadata_template,
    nagoya_geoparquet_density_template, nagoya_geoparquet_template,
    nagoya_population_density_template, remote_cog_metadata_template,
    remote_geoparquet_range_template, stac_endpoint_registry_template, Citation, GeoWorkflow,
    WorkflowInputContract, WorkflowValidationError,
};
pub use operation::{OperationDescriptor, OperationId};
pub use review::ReviewStatus;
pub use step::{
    WorkflowDataRef, WorkflowNodeId, WorkflowPortContract, WorkflowStep, WorkflowStepId,
};
