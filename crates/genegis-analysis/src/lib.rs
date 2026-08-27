//! Spatial analysis engine — workflow execution and operators.

pub mod accessibility;
pub mod change;
pub mod city_scene;
pub mod dashboard;
pub mod district3d;
pub mod error;
pub mod evacuation;
pub mod export;
pub mod flood;
pub mod geocoding;
pub mod governance;
pub mod gpu_acceptance;
pub mod live_dashboard;
pub mod live_feed;
pub mod nagoya;
pub mod ndvi;
pub mod ogc_service;
pub mod operational_dashboard;
pub mod performance_matrix;
pub mod pipeline;
pub mod preview;
pub mod private_catalog;
pub mod result;
pub mod scenario;
pub mod showcase;
pub mod temporal_playback;
pub mod verified_alert;
pub mod zone_index;

pub use showcase::{render_usecase_frames, ShowcaseFrame};

pub use accessibility::{
    run_nagoya_accessibility, run_nagoya_accessibility_with_threshold, AccessibilityAnalysis,
    AccessibilityFeature, DEFAULT_THRESHOLD_MINUTES,
};
pub use change::{
    run_pointcloud_change_detection, ChangeClassSummary, ChangeDetectionAnalysis,
    CHANGE_CELL_SIZE_M, CONTROL_AREA,
};
pub use city_scene::{plan_city_scene_workflow, CitySceneWorkflowResult};
pub use dashboard::{export_dashboard_pmtiles, DashboardExportOptions, DashboardExportReport};
pub use district3d::{render_district3d_frames, District3dFrame};
pub use error::AnalysisError;
pub use evacuation::{
    run_nagoya_evacuation_access, run_nagoya_evacuation_access_with_penalty, EvacuationAnalysis,
    EvacuationFeature, DEFAULT_DEPTH_SPEED_PENALTY_PER_M,
};
pub use export::{export_html_map, export_map_svg, export_png_map, ExportError};
pub use flood::{
    export_flood_html_map, run_nagoya_flood_exposure, run_nagoya_flood_exposure_with_options,
    FloodExposureAnalysis, FloodExposureFeature, FloodZone, DEFAULT_SAMPLES_PER_AXIS,
};
pub use geocoding::{execute_geocoding_workflow, GeocodingWorkflowResult};
pub use governance::{
    execute_governance_operation, GovernanceOperation, GovernanceOperationOutput,
    GovernanceOperationReceipt,
};
pub use gpu_acceptance::{
    run_gpu_scene_acceptance_workflow, verify_gpu_scene_acceptance_receipt, GpuAcceptanceVerdict,
    GpuSceneAcceptanceReceipt, GpuSceneAcceptanceRequest,
};
pub use live_dashboard::{
    build_scene3d_dashboard, canonical_scene_result_digest, CategoryCount, DashboardWidget,
    HistogramBin, LiveDashboard, LiveDashboardError,
};
pub use live_feed::{execute_live_feed_workflow, LiveFeedWorkflowResult};
pub use nagoya::{
    canonical_nagoya_execution_digest, default_nagoya_data_path, default_nagoya_dataset_id,
    nagoya_population_density_workflow_for_dataset, run_nagoya_population_density,
    run_nagoya_population_density_for_dataset, run_nagoya_population_density_from_catalog,
    run_nagoya_population_density_geoparquet, verify_nagoya_analysis, NagoyaArtifactDigests,
    NagoyaExecutionOutput, NagoyaWorkflowExecutor,
};
pub use ndvi::{
    run_nagoya_ndvi_timeseries, NdviEpochSummary, NdviFeature, NdviTimeseriesAnalysis,
    CHANGE_THRESHOLD_NDVI,
};
pub use ogc_service::{execute_ogc_workflow, OgcWorkflowResult};
pub use operational_dashboard::{
    compose_operational_dashboard, seal_operational_dashboard, verify_operational_dashboard,
    OperationalAlertHistoryEntry, OperationalDashboardDraft, OperationalDashboardError,
    OperationalDashboardReceipt, OperationalDashboardView, OperationalMapLayer, OperationalStatus,
    OPERATIONAL_DASHBOARD_SCHEMA_VERSION,
};
pub use performance_matrix::{
    evaluate_performance_matrix_workflow, PerformanceMatrixWorkflowReceipt,
};
pub use pipeline::{
    build_accessibility_ask_result, build_ask_result, build_ask_result_from_dispatch,
    build_change_ask_result, build_dashboard_ask_result, build_evacuation_ask_result,
    build_flood_ask_result, build_geoparquet_ask_result, build_ndvi_ask_result,
    build_remote_cog_ask_result, build_stac_collection_ask_result, execute_from_plan,
    execute_from_plan_with_origin, execute_workflow_for_plan,
    execute_workflow_for_plan_with_origin, execution_receipt_for_workflow,
    execution_receipt_for_workflow_with_checks, execution_receipt_for_workflow_with_executor,
    run_analysis_for_plan, run_ask_pipeline, run_ask_pipeline_with_config,
    run_ask_pipeline_with_config_and_origin, verify_analysis_densities, verify_dashboard_export,
    verify_evacuation_analysis, verify_executed_workflow, verify_geoparquet_features,
    verify_ndvi_timeseries_analysis, verify_remote_cog_metadata, verify_stac_collection,
    AskPipelineResult, ExecutedWorkflow, ExecutedWorkflowOutput, NagoyaDispatch,
};
pub use preview::{
    attach_scene3d_gpu_evidence, cog_raster_preview_map, launch_scene3d_preview,
    nagoya_choropleth_map, spawn_cog_gpu_preview, spawn_gpu_preview_for_workflow,
    spawn_nagoya_gpu_preview, Scene3dLaunchReceipt,
};
pub use private_catalog::{admit_private_catalog_workflow, PrivateCatalogAdmissionReceipt};
pub use result::{
    canonical_analysis_result_digest, AnalysisResult, DensityFeature, EngineIdentity,
    ExecutionReceipt, VerificationCheck, VerificationReport,
};
pub use scenario::{
    compare_scenarios, create_scenario_branch, merge_reviewed_scenario, ScenarioApproval,
    ScenarioAssumption, ScenarioBranch, ScenarioBranchDraft, ScenarioBranchReceipt,
    ScenarioComparison, ScenarioComparisonReceipt, ScenarioError, ScenarioMergeCommit,
    ScenarioMergeReceipt, ScenarioSemanticChange, ScenarioSpatialOutcome, SCENARIO_SCHEMA_VERSION,
};
pub use temporal_playback::{
    build_ndvi_temporal_playback, TemporalEpochLayer, TemporalFeatureValue, TemporalPlayback,
    TileEncodingBudget, TileEncodingReceipt, TEMPORAL_PLAYBACK_SCHEMA_VERSION, TEMPORAL_TILE_ZOOM,
};
pub use verified_alert::{
    acknowledge_verified_alert, evaluate_verified_alert, verify_alert_record, AlertAcknowledgement,
    AlertAcknowledgementReceipt, AlertComparison, AlertMetric, AlertTriggeringWindow,
    AlertVerificationCheck, VerifiedAlertError, VerifiedAlertEvaluation, VerifiedAlertPolicy,
    VerifiedAlertRecord, VerifiedAlertRule, VERIFIED_ALERT_SCHEMA_VERSION,
};
