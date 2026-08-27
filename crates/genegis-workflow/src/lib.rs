//! GeoWorkflow IR — the intermediate representation for analysis and AI planning.
//!
//! AI generates workflows first; verified execution follows.

pub mod composer;
pub mod graph;
pub mod operation;
pub mod review;
pub mod step;

pub use composer::{
    reviewed_workflow_templates, AnalysisTemplateCategory, ComposerCommand, ComposerError,
    ComposerEvent, ReviewedWorkflowTemplate, WorkflowComposer,
};
pub use graph::{
    alert_acknowledgement_template, city_scene_stream_template, copc_change_detect_template,
    dashboard_export_template, desktop_layer_bridge_template, external_stac_fetch_template,
    federated_asset_execution_template, federated_stac_search_template, geocoding_template,
    live_feed_ingest_template, local_cog_metadata_template, nagoya_evacuation_template,
    nagoya_flood_exposure_template, nagoya_geoparquet_density_template, nagoya_geoparquet_template,
    nagoya_population_density_template, nagoya_xmin_city_template, narrative_project_view_template,
    ogc_service_read_template, operational_dashboard_view_template,
    organization_governance_template, performance_matrix_evaluation_template,
    plugin_registry_operation_template, private_federated_catalog_template,
    remote_cog_metadata_template, remote_geoparquet_range_template, scenario_branch_template,
    scenario_comparison_template, scenario_merge_template, scene3d_copc_lod1_template,
    sentinel_ndvi_timeseries_template, solution_pack_admission_template,
    stac_endpoint_registry_template, verified_alert_evaluation_template, Citation,
    DesktopLayerBridgeInput, GeoWorkflow, WorkflowInputContract, WorkflowValidationError,
};
pub use operation::{OperationDescriptor, OperationId};
pub use review::ReviewStatus;
pub use step::{
    WorkflowDataRef, WorkflowNodeId, WorkflowPortContract, WorkflowStep, WorkflowStepId,
};
