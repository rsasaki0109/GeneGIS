//! Shared MVP ask → analyze → verify → export pipeline.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::{DateTime, Utc};
use genegis_ai::{extract_catalog_url, plan_with_config, PlanResult, PlannerConfig, WorkflowId};
use genegis_catalog::{alpha_catalog, fetch_stac_collection, DatasetRecord, StacCollection};
use genegis_contract::{
    CompatibilityStatus, ContractEvidence, ReplayEvidence, SourceEvidence, TrustEvidence,
    TrustLevel, VerificationGraph, VerificationPolicy,
};
use genegis_core::{
    Command, CommandBus, CommandEnvelope, CommandExecution, CommandOrigin, InputSnapshot, Project,
    ProvenanceEntry, ProvenanceStore, WorkflowDigest, WorkflowExecution, WorkflowExecutionContext,
    WorkflowExecutionError, WorkflowExecutionEvent, WorkflowExecutor,
};
use genegis_crs::{ChecksumVerification, CoordinateUnit, Crs, SourceMetadata};
use genegis_raster::CogInfo;
use genegis_vector::{
    geoparquet_summary, read_geoparquet_uri, verify_nagoya_geoparquet, VectorDataset,
};
use genegis_workflow::{
    nagoya_geoparquet_density_template, nagoya_population_density_template, GeoWorkflow,
    ReviewStatus,
};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::accessibility::{run_nagoya_accessibility, AccessibilityAnalysis};
use crate::change::{run_pointcloud_change_detection, ChangeDetectionAnalysis};
use crate::dashboard::{export_dashboard_pmtiles, DashboardExportOptions, DashboardExportReport};
use crate::error::AnalysisError;
use crate::evacuation::{run_nagoya_evacuation_access, EvacuationAnalysis};
use crate::export::{export_html_map, export_png_map};
use crate::flood::{export_flood_html_map, run_nagoya_flood_exposure, FloodExposureAnalysis};
use crate::nagoya::{
    canonical_nagoya_execution_digest, nagoya_population_density_workflow_for_dataset,
    nagoya_source_assurance, nagoya_verification_observations, verify_nagoya_analysis,
    NagoyaArtifactDigests, NagoyaExecutionOutput, NagoyaWorkflowExecutor,
};
use crate::ndvi::{run_nagoya_ndvi_timeseries, NdviTimeseriesAnalysis};
use crate::result::{
    AnalysisResult, EngineIdentity, ExecutionReceipt, VerificationCheck, VerificationReport,
};

/// Executed workflow payload for agent orchestration (Phase 8 alpha).
#[derive(Debug, Clone)]
pub enum ExecutedWorkflow {
    NagoyaDensity(AnalysisResult),
    CogMetadata(CogInfo),
    Geoparquet(VectorDataset),
    StacCollection(StacCollection),
    DashboardExport(DashboardExportReport),
    FloodExposure(FloodExposureAnalysis),
    Accessibility(AccessibilityAnalysis),
    Evacuation(EvacuationAnalysis),
    NdviTimeseries(NdviTimeseriesAnalysis),
    PointCloudChange(ChangeDetectionAnalysis),
}

/// Internal/public additive form retaining the single CommandExecution
/// payload needed to build an ask result without running the executor twice.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)] // payload variants mirror ExecutedWorkflow precedent
pub enum ExecutedWorkflowOutput {
    NagoyaDensity(NagoyaDispatch),
    CogMetadata(CogInfo),
    Geoparquet(VectorDataset),
    StacCollection(StacCollection),
    DashboardExport(DashboardExportReport),
    FloodExposure(FloodExposureAnalysis),
    Accessibility(AccessibilityAnalysis),
    Evacuation(EvacuationAnalysis),
    NdviTimeseries(NdviTimeseriesAnalysis),
    PointCloudChange(ChangeDetectionAnalysis),
}

#[derive(Debug, Clone)]
pub struct NagoyaDispatch {
    pub output: NagoyaExecutionOutput,
    pub command: CommandEnvelope,
    pub workflow: GeoWorkflow,
    pub execution: CommandExecution,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AskPipelineResult {
    pub prompt: String,
    pub workflow_id: String,
    pub confidence: f32,
    pub ambiguities: Vec<String>,
    pub workflow_steps: usize,
    pub verification: VerificationReport,
    /// Exact typed analysis output when this workflow produced one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis: Option<AnalysisResult>,
    pub summary: serde_json::Value,
    pub html: String,
    #[serde(skip)]
    pub png: Vec<u8>,
    pub png_base64: String,
    pub duckdb_verified: bool,
    pub dataset: DatasetRecord,
    pub stac_item: genegis_catalog::StacItem,
    /// Auditable command that authorized this execution.
    pub command: CommandEnvelope,
    /// Evidence-first receipt emitted by the shared command dispatcher.
    pub execution_receipt: ExecutionReceipt,
    /// Executed workflow graph referenced by `command`.
    pub workflow: GeoWorkflow,
    /// Append-only execution provenance, including CRS, units, and source identity.
    pub provenance: ProvenanceStore,
}

pub fn run_ask_pipeline(prompt: &str) -> Result<AskPipelineResult, AnalysisError> {
    run_ask_pipeline_with_config(prompt, &PlannerConfig::default())
}

pub fn run_ask_pipeline_with_config(
    prompt: &str,
    config: &PlannerConfig,
) -> Result<AskPipelineResult, AnalysisError> {
    run_ask_pipeline_with_config_and_origin(prompt, config, CommandOrigin::Ai)
}

/// AI/UI-compatible ask entrypoint with an explicit command origin. The
/// origin reaches the same dispatcher; it is not inferred after execution.
pub fn run_ask_pipeline_with_config_and_origin(
    prompt: &str,
    config: &PlannerConfig,
    origin: CommandOrigin,
) -> Result<AskPipelineResult, AnalysisError> {
    let plan =
        plan_with_config(prompt, config).map_err(|e| AnalysisError::Message(e.to_string()))?;
    execute_from_plan_with_origin(prompt, &plan, origin)
}

pub fn run_analysis_for_plan(
    plan: &PlanResult,
) -> Result<(AnalysisResult, DatasetRecord), AnalysisError> {
    match execute_workflow_for_plan(plan)? {
        (ExecutedWorkflow::NagoyaDensity(analysis), dataset) => Ok((analysis, dataset)),
        (ExecutedWorkflow::CogMetadata(_), _) => Err(AnalysisError::Message(
            "cog metadata workflow does not produce AnalysisResult; use execute_workflow_for_plan"
                .into(),
        )),
        (ExecutedWorkflow::Geoparquet(_), _) => Err(AnalysisError::Message(
            "geoparquet workflow does not produce AnalysisResult; use execute_workflow_for_plan"
                .into(),
        )),
        (ExecutedWorkflow::StacCollection(_), _) => Err(AnalysisError::Message(
            "stac collection workflow does not produce AnalysisResult; use execute_workflow_for_plan"
                .into(),
        )),
        (ExecutedWorkflow::DashboardExport(_), _) => Err(AnalysisError::Message(
            "dashboard export workflow does not produce AnalysisResult; use execute_workflow_for_plan"
                .into(),
        )),
        (ExecutedWorkflow::FloodExposure(_), _) => Err(AnalysisError::Message(
            "flood exposure workflow does not produce AnalysisResult; use execute_workflow_for_plan"
                .into(),
        )),
        (ExecutedWorkflow::Accessibility(_), _) => Err(AnalysisError::Message(
            "accessibility workflow does not produce AnalysisResult; use execute_workflow_for_plan"
                .into(),
        )),
        (ExecutedWorkflow::Evacuation(_), _) => Err(AnalysisError::Message(
            "evacuation workflow does not produce AnalysisResult; use execute_workflow_for_plan"
                .into(),
        )),
        (ExecutedWorkflow::NdviTimeseries(_), _) => Err(AnalysisError::Message(
            "ndvi workflow does not produce AnalysisResult; use execute_workflow_for_plan".into(),
        )),
        (ExecutedWorkflow::PointCloudChange(_), _) => Err(AnalysisError::Message(
            "pointcloud change workflow does not produce AnalysisResult; use execute_workflow_for_plan"
                .into(),
        )),
    }
}

pub fn execute_workflow_for_plan(
    plan: &PlanResult,
) -> Result<(ExecutedWorkflow, DatasetRecord), AnalysisError> {
    let (executed, dataset) = execute_workflow_for_plan_with_origin(plan, CommandOrigin::Ai)?;
    Ok((legacy_workflow(&executed), dataset))
}

/// Execute a plan through the shared dispatcher with an explicit command
/// origin. The Nagoya variant retains the actual CommandExecution payload so
/// downstream receipt/render assembly never invokes the executor again.
pub fn execute_workflow_for_plan_with_origin(
    plan: &PlanResult,
    origin: CommandOrigin,
) -> Result<(ExecutedWorkflowOutput, DatasetRecord), AnalysisError> {
    let catalog = alpha_catalog();
    let dataset_record = catalog
        .require(&plan.resolved.dataset_id)
        .map_err(|e| AnalysisError::Message(e.to_string()))?
        .clone();

    match plan.resolved.workflow_id {
        WorkflowId::NagoyaDensity | WorkflowId::NagoyaGeoparquetDensity => {
            validate_planned_nagoya_graph(plan)?;
            let execution = execute_nagoya_with_dispatch(&dataset_record, origin)?;
            Ok((
                ExecutedWorkflowOutput::NagoyaDensity(execution),
                dataset_record,
            ))
        }
        WorkflowId::DashboardExportDemo => {
            let output_path = std::path::Path::new(".genegis").join("dashboard.pmtiles");
            if let Some(parent) = output_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| AnalysisError::Message(error.to_string()))?;
            }
            let analysis = crate::nagoya::run_nagoya_population_density(&dataset_record.uri)
                .map_err(|error| AnalysisError::Message(error.to_string()))?;
            let report = export_dashboard_pmtiles(
                &analysis,
                output_path.to_str().ok_or_else(|| {
                    AnalysisError::Message("dashboard output path is not UTF-8".into())
                })?,
                &DashboardExportOptions::default(),
            )?;
            if !report.verification_passed {
                return Err(AnalysisError::Message(
                    "dashboard export verification failed; archive withheld".into(),
                ));
            }
            Ok((
                ExecutedWorkflowOutput::DashboardExport(report),
                dataset_record,
            ))
        }
        WorkflowId::NagoyaFloodExposure => {
            let analysis = run_nagoya_flood_exposure(
                genegis_catalog::nagoya_wards_geojson_path(),
                &dataset_record.uri,
            )?;
            if !analysis
                .verification
                .checks
                .iter()
                .all(|check| check.passed)
            {
                return Err(AnalysisError::Message(
                    "flood exposure verification failed; map withheld".into(),
                ));
            }
            Ok((
                ExecutedWorkflowOutput::FloodExposure(analysis),
                dataset_record,
            ))
        }
        WorkflowId::NagoyaXminCity => {
            let analysis = run_nagoya_accessibility(
                genegis_catalog::nagoya_wards_geojson_path(),
                &dataset_record.uri,
                genegis_catalog::nagoya_pois_path(),
            )?;
            if !analysis
                .verification
                .checks
                .iter()
                .all(|check| check.passed)
            {
                return Err(AnalysisError::Message(
                    "accessibility verification failed; map withheld".into(),
                ));
            }
            Ok((
                ExecutedWorkflowOutput::Accessibility(analysis),
                dataset_record,
            ))
        }
        WorkflowId::NagoyaEvacuationAccess => {
            let analysis = run_nagoya_evacuation_access(
                genegis_catalog::nagoya_wards_geojson_path(),
                genegis_catalog::nagoya_walk_network_path(),
                genegis_catalog::nagoya_flood_zones_path(),
                &dataset_record.uri,
            )?;
            if !analysis
                .verification
                .checks
                .iter()
                .all(|check| check.passed)
            {
                return Err(AnalysisError::Message(
                    "evacuation verification failed; map withheld".into(),
                ));
            }
            Ok((ExecutedWorkflowOutput::Evacuation(analysis), dataset_record))
        }
        WorkflowId::SentinelNdviTimeseries => {
            let analysis = run_nagoya_ndvi_timeseries(
                &dataset_record.uri,
                genegis_catalog::nagoya_wards_geojson_path(),
            )?;
            if !analysis
                .verification
                .checks
                .iter()
                .all(|check| check.passed)
            {
                return Err(AnalysisError::Message(
                    "NDVI verification failed; chart withheld".into(),
                ));
            }
            Ok((
                ExecutedWorkflowOutput::NdviTimeseries(analysis),
                dataset_record,
            ))
        }
        WorkflowId::CopcChangeDetect => {
            let analysis = run_pointcloud_change_detection(
                &dataset_record.uri,
                genegis_catalog::nagoya_pointcloud_epoch_b_path(),
            )?;
            if !analysis
                .verification
                .checks
                .iter()
                .all(|check| check.passed)
            {
                return Err(AnalysisError::Message(
                    "change-detection verification failed; report withheld".into(),
                ));
            }
            Ok((
                ExecutedWorkflowOutput::PointCloudChange(analysis),
                dataset_record,
            ))
        }
        WorkflowId::RemoteCogDemo | WorkflowId::LocalCogDemo => {
            let info = genegis_raster::read_cog_uri(&dataset_record.uri)
                .map_err(|err| AnalysisError::Message(err.to_string()))?;
            Ok((ExecutedWorkflowOutput::CogMetadata(info), dataset_record))
        }
        WorkflowId::NagoyaGeoparquet => {
            let dataset = read_geoparquet_uri(&dataset_record.uri)
                .map_err(|err| AnalysisError::Message(err.to_string()))?;
            Ok((ExecutedWorkflowOutput::Geoparquet(dataset), dataset_record))
        }
        WorkflowId::ExternalStacDemo => {
            let url = extract_catalog_url(&plan.intent.raw_prompt)
                .unwrap_or_else(|| dataset_record.uri.clone());
            let collection = fetch_stac_collection(&url)
                .map_err(|err| AnalysisError::Message(err.to_string()))?;
            Ok((
                ExecutedWorkflowOutput::StacCollection(collection),
                dataset_record,
            ))
        }
    }
}

/// Do not silently replace a planner-supplied Nagoya graph with the runtime
/// graph if the plan was modified after approval. Runtime source/CRS binding
/// is allowed below, but the authored operation/DAG contract must first match
/// the corresponding immutable north-star template.
fn validate_planned_nagoya_graph(plan: &PlanResult) -> Result<(), AnalysisError> {
    let expected = match plan.resolved.workflow_id {
        WorkflowId::NagoyaDensity => nagoya_population_density_template(),
        WorkflowId::NagoyaGeoparquetDensity => nagoya_geoparquet_density_template(),
        _ => return Ok(()),
    };
    plan.workflow.validate().map_err(|error| {
        AnalysisError::Message(format!("invalid planned workflow graph: {error}"))
    })?;
    let planned_digest = plan.workflow.stable_digest().map_err(|error| {
        AnalysisError::Message(format!("planned workflow digest failed: {error}"))
    })?;
    let expected_digest = expected.stable_digest().map_err(|error| {
        AnalysisError::Message(format!("north-star workflow digest failed: {error}"))
    })?;
    if planned_digest != expected_digest {
        return Err(AnalysisError::Message(
            "planned Nagoya workflow was modified after approval".into(),
        ));
    }
    Ok(())
}

fn legacy_workflow(executed: &ExecutedWorkflowOutput) -> ExecutedWorkflow {
    match executed {
        ExecutedWorkflowOutput::NagoyaDensity(dispatch) => {
            ExecutedWorkflow::NagoyaDensity(dispatch.output.analysis.clone())
        }
        ExecutedWorkflowOutput::CogMetadata(info) => ExecutedWorkflow::CogMetadata(info.clone()),
        ExecutedWorkflowOutput::Geoparquet(dataset) => {
            ExecutedWorkflow::Geoparquet(dataset.clone())
        }
        ExecutedWorkflowOutput::Evacuation(analysis) => {
            ExecutedWorkflow::Evacuation(analysis.clone())
        }
        ExecutedWorkflowOutput::NdviTimeseries(analysis) => {
            ExecutedWorkflow::NdviTimeseries(analysis.clone())
        }
        ExecutedWorkflowOutput::PointCloudChange(analysis) => {
            ExecutedWorkflow::PointCloudChange(analysis.clone())
        }
        ExecutedWorkflowOutput::StacCollection(collection) => {
            ExecutedWorkflow::StacCollection(collection.clone())
        }
        ExecutedWorkflowOutput::DashboardExport(report) => {
            ExecutedWorkflow::DashboardExport(report.clone())
        }
        ExecutedWorkflowOutput::FloodExposure(analysis) => {
            ExecutedWorkflow::FloodExposure(analysis.clone())
        }
        ExecutedWorkflowOutput::Accessibility(analysis) => {
            ExecutedWorkflow::Accessibility(analysis.clone())
        }
    }
}

/// Execute the north-star data plane through the same CommandBus boundary
/// used by receipts. The executor is not called until the graph digest and
/// every typed source/input snapshot has been validated by core.
fn execute_nagoya_with_dispatch(
    dataset: &DatasetRecord,
    origin: CommandOrigin,
) -> Result<NagoyaDispatch, AnalysisError> {
    let workflow = nagoya_population_density_workflow_for_dataset(dataset)?;
    let (command, _, _, _) = command_for_workflow(&workflow, dataset, origin)?;
    let executor = NagoyaWorkflowExecutor::new(dataset.id.clone());
    let mut project = Project::new("nagoya-execution");
    project.manifest.workspace.id = Uuid::nil();
    let mut bus = CommandBus::new(project.clone());
    let execution = bus
        .apply_with_workflow_executor(&mut project, command.clone(), workflow.clone(), &executor)
        .map_err(|error| AnalysisError::Message(format!("command dispatch failed: {error}")))?;
    let output = execution.output.clone().ok_or_else(|| {
        AnalysisError::Message("Nagoya executor returned no analysis output".into())
    })?;
    let output: NagoyaExecutionOutput = serde_json::from_value(output).map_err(|error| {
        AnalysisError::Message(format!("invalid Nagoya executor output: {error}"))
    })?;
    let evidence = execution
        .evidence
        .clone()
        .unwrap_or(serde_json::Value::Null);
    let canonical_digest = canonical_nagoya_execution_digest(&output, &evidence);
    if execution.result_digest.as_deref() != Some(canonical_digest.as_str()) {
        return Err(AnalysisError::Message(
            "Nagoya executor result digest does not match its output".into(),
        ));
    }
    Ok(NagoyaDispatch {
        output,
        command,
        workflow,
        execution,
    })
}

pub fn verify_executed_workflow(result: &ExecutedWorkflow) -> Result<bool, AnalysisError> {
    match result {
        ExecutedWorkflow::NagoyaDensity(analysis) => verify_analysis_densities(analysis),
        ExecutedWorkflow::CogMetadata(info) => verify_remote_cog_metadata(info),
        ExecutedWorkflow::Geoparquet(dataset) => verify_geoparquet_features(dataset),
        ExecutedWorkflow::StacCollection(collection) => verify_stac_collection(collection),
        ExecutedWorkflow::DashboardExport(report) => verify_dashboard_export(report),
        ExecutedWorkflow::FloodExposure(analysis) => verify_flood_exposure_analysis(analysis),
        ExecutedWorkflow::Accessibility(analysis) => verify_accessibility_analysis(analysis),
        ExecutedWorkflow::Evacuation(analysis) => verify_evacuation_analysis(analysis),
        ExecutedWorkflow::NdviTimeseries(analysis) => verify_ndvi_timeseries_analysis(analysis),
        ExecutedWorkflow::PointCloudChange(analysis) => verify_change_detection_analysis(analysis),
    }
}

/// Native route-sanity re-check for accessibility results.
pub fn verify_accessibility_analysis(
    analysis: &AccessibilityAnalysis,
) -> Result<bool, AnalysisError> {
    Ok(analysis
        .verification
        .checks
        .iter()
        .all(|check| check.passed)
        && analysis.features.iter().all(|feature| {
            feature.accessibility_score >= 0.0
                && feature.accessibility_score <= 1.0
                && feature.reachable_pois <= feature.poi_total
        }))
}

/// Native route-sanity re-check for evacuation results.
pub fn verify_evacuation_analysis(analysis: &EvacuationAnalysis) -> Result<bool, AnalysisError> {
    Ok(analysis
        .verification
        .checks
        .iter()
        .all(|check| check.passed)
        && analysis
            .features
            .iter()
            .all(|feature| feature.delay_minutes >= -1e-6))
}

/// Native re-check for NDVI time-series results.
pub fn verify_ndvi_timeseries_analysis(
    analysis: &NdviTimeseriesAnalysis,
) -> Result<bool, AnalysisError> {
    Ok(analysis
        .verification
        .checks
        .iter()
        .all(|check| check.passed)
        && analysis.epochs.len() >= 2
        && analysis.features.iter().all(|feature| {
            feature.mean_ndvi_per_epoch.len() == analysis.epochs.len()
                && feature
                    .mean_ndvi_per_epoch
                    .iter()
                    .all(|value| (-1.0..=1.0).contains(value))
        }))
}

/// Native re-check for point-cloud change detection.
pub fn verify_change_detection_analysis(
    analysis: &ChangeDetectionAnalysis,
) -> Result<bool, AnalysisError> {
    Ok(analysis
        .verification
        .checks
        .iter()
        .all(|check| check.passed)
        && analysis
            .summaries
            .iter()
            .all(|summary| summary.cell_count > 0))
}

/// Re-aggregate flood exposure rows in DuckDB as the independent verifier.
pub fn verify_flood_exposure_analysis(
    analysis: &FloodExposureAnalysis,
) -> Result<bool, AnalysisError> {
    let rows: Vec<(String, u64, u64, f64)> = analysis
        .features
        .iter()
        .map(|feature| {
            (
                feature.ward_name.clone(),
                feature.exposed_population,
                feature.population,
                feature.exposure_rate,
            )
        })
        .collect();
    let duckdb_ok = genegis_query::verify_flood_exposure(&rows)
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    Ok(duckdb_ok
        && analysis
            .verification
            .checks
            .iter()
            .all(|check| check.passed))
}

/// Fail-closed re-check of a dashboard export receipt plus its artifact.
pub fn verify_dashboard_export(report: &DashboardExportReport) -> Result<bool, AnalysisError> {
    let artifact_exists = std::fs::metadata(&report.output_path)
        .map(|meta| meta.len() == report.object_bytes && report.object_bytes > 0)
        .unwrap_or(false);
    Ok(report.verification_passed && !report.checks.is_empty() && artifact_exists)
}

pub fn verify_analysis_densities(analysis: &AnalysisResult) -> Result<bool, AnalysisError> {
    verify_nagoya_analysis(analysis)
}

pub fn verify_remote_cog_metadata(info: &CogInfo) -> Result<bool, AnalysisError> {
    let known_crs = Crs::parse(&info.crs)
        .ok()
        .and_then(|crs| crs.require_known().ok())
        .is_some();
    Ok(info.width > 0
        && info.height > 0
        && info.band_count >= 1
        && !info.crs.is_empty()
        && known_crs)
}

pub fn verify_geoparquet_features(dataset: &VectorDataset) -> Result<bool, AnalysisError> {
    let known_crs = Crs::parse(&dataset.crs)
        .ok()
        .and_then(|crs| crs.require_known().ok())
        .is_some();
    if !known_crs {
        return Ok(false);
    }
    verify_nagoya_geoparquet(dataset).map_err(|err| AnalysisError::Message(err.to_string()))
}

pub fn verify_stac_collection(collection: &StacCollection) -> Result<bool, AnalysisError> {
    Ok(!collection.id.is_empty() && collection.collection_type == "Collection")
}

/// Assemble the north-star ask result from the one CommandExecution returned
/// by the dispatcher. No analysis, verification, or export function is called
/// here: all three were completed by `NagoyaWorkflowExecutor` before this
/// method was reached.
pub fn build_ask_result_from_dispatch(
    prompt: &str,
    plan: &PlanResult,
    dispatch: NagoyaDispatch,
    dataset: DatasetRecord,
) -> Result<AskPipelineResult, AnalysisError> {
    if !dispatch.output.verification_passed {
        return Err(AnalysisError::Message(
            "workflow verification failed; render artifacts were withheld".into(),
        ));
    }
    // Build the receipt while the dispatch still owns the complete execution
    // record. Moving `output` first would make it impossible to prove that
    // the receipt was assembled from this exact CommandExecution.
    let expected_result_digest = canonical_nagoya_execution_digest(
        &dispatch.output,
        &dispatch
            .execution
            .evidence
            .clone()
            .unwrap_or(serde_json::Value::Null),
    );
    if dispatch.execution.result_digest.as_deref() != Some(expected_result_digest.as_str()) {
        return Err(AnalysisError::Message(
            "Nagoya dispatch output was modified after execution".into(),
        ));
    }
    let workflow = dispatch.output.analysis.workflow.clone();
    let (provenance, execution_receipt) = receipt_from_dispatch(
        &dispatch,
        &workflow,
        &dataset,
        &dispatch.output.analysis.verification.crs,
        &dispatch.output.analysis.verification.coordinate_unit,
        &dispatch.output.analysis.verification.density_unit,
        &dispatch.output.analysis.verification.area_method,
        dispatch.output.verification_passed,
        "duckdb_verify",
        &dispatch.output.analysis.verification.checks,
        &plan.mode,
    )?;
    let output = dispatch.output;
    let analysis = output.analysis;
    let png = STANDARD.decode(&output.png_base64).map_err(|error| {
        AnalysisError::Message(format!("invalid PNG artifact from executor: {error}"))
    })?;
    if output.verification_passed {
        let html_digest = format!("sha256:{:x}", Sha256::digest(output.html.as_bytes()));
        let png_digest = format!("sha256:{:x}", Sha256::digest(&png));
        if output.artifact_digests.html_sha256.as_deref() != Some(html_digest.as_str())
            || output.artifact_digests.png_sha256.as_deref() != Some(png_digest.as_str())
            || output.artifact_digests.html_bytes != output.html.len()
            || output.artifact_digests.png_bytes != png.len()
        {
            return Err(AnalysisError::Message(
                "Nagoya render artifact digest mismatch".into(),
            ));
        }
    }
    let png_base64 = output.png_base64;
    let mut summary = build_summary(&analysis, &dataset);
    if let Some(summary_object) = summary.as_object_mut() {
        summary_object.insert(
            "command".into(),
            serde_json::to_value(&dispatch.command)
                .map_err(|error| AnalysisError::Message(error.to_string()))?,
        );
        summary_object.insert(
            "workflow".into(),
            serde_json::to_value(&workflow)
                .map_err(|error| AnalysisError::Message(error.to_string()))?,
        );
        summary_object.insert(
            "provenance".into(),
            serde_json::to_value(&provenance)
                .map_err(|error| AnalysisError::Message(error.to_string()))?,
        );
        summary_object.insert(
            "execution_receipt".into(),
            serde_json::to_value(&execution_receipt)
                .map_err(|error| AnalysisError::Message(error.to_string()))?,
        );
    }
    Ok(AskPipelineResult {
        prompt: prompt.to_string(),
        workflow_id: plan.resolved.workflow_id.as_str().to_string(),
        confidence: plan.resolved.confidence,
        ambiguities: plan.resolved.ambiguities.clone(),
        workflow_steps: plan.workflow.steps.len(),
        verification: analysis.verification.clone(),
        analysis: Some(analysis.clone()),
        summary,
        html: output.html,
        png,
        png_base64,
        duckdb_verified: output.verification_passed,
        dataset: dataset.clone(),
        stac_item: dataset.to_stac_item(),
        command: dispatch.command,
        execution_receipt,
        workflow,
        provenance,
    })
}

fn receipt_from_dispatch(
    dispatch: &NagoyaDispatch,
    workflow: &GeoWorkflow,
    dataset: &DatasetRecord,
    crs: &str,
    coordinate_units: &str,
    units: &str,
    area_method: &str,
    verified: bool,
    verifier: &str,
    checks: &[VerificationCheck],
    planner_mode: &str,
) -> Result<(ProvenanceStore, ExecutionReceipt), AnalysisError> {
    let workflow_digest =
        dispatch.command.workflow_digest.clone().ok_or_else(|| {
            AnalysisError::Message("Nagoya command omitted workflow digest".into())
        })?;
    let actual_workflow_digest = WorkflowDigest::new(
        workflow
            .stable_digest()
            .map_err(|error| AnalysisError::Message(error.to_string()))?,
    );
    if workflow_digest != actual_workflow_digest {
        return Err(AnalysisError::Message(
            "Nagoya receipt workflow digest does not match its graph".into(),
        ));
    }
    let source_snapshots = dispatch.command.source_snapshots.clone();
    let input_snapshots = dispatch.command.input_snapshots.clone();
    let engine: EngineIdentity = Default::default();
    let result_digest =
        dispatch.execution.result_digest.clone().ok_or_else(|| {
            AnalysisError::Message("Nagoya execution omitted result digest".into())
        })?;
    let evidence = dispatch
        .execution
        .evidence
        .clone()
        .unwrap_or(serde_json::Value::Null);
    let verification_policy: VerificationPolicy = serde_json::from_value(
        evidence
            .get("verification_policy")
            .cloned()
            .ok_or_else(|| {
                AnalysisError::Message("execution omitted verification policy".into())
            })?,
    )
    .map_err(|error| AnalysisError::Message(format!("invalid verification policy: {error}")))?;
    let verification_graph: VerificationGraph =
        serde_json::from_value(evidence.get("verification_graph").cloned().ok_or_else(|| {
            AnalysisError::Message("execution omitted verification graph".into())
        })?)
        .map_err(|error| AnalysisError::Message(format!("invalid verification graph: {error}")))?;
    verification_graph
        .validate_against_policy(&verification_policy)
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    let verification_graph_digest = verification_graph
        .stable_digest()
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    if evidence
        .get("verification_graph_digest")
        .and_then(serde_json::Value::as_str)
        != Some(verification_graph_digest.as_str())
    {
        return Err(AnalysisError::Message(
            "execution verification graph digest mismatch".into(),
        ));
    }
    let trust_evidence = nagoya_trust_evidence(
        workflow,
        &dispatch.output.analysis,
        &source_snapshots,
        workflow_digest.as_str(),
        &result_digest,
        &engine,
        &dispatch.output.artifact_digests,
    );
    let trust_assessment = verification_policy.assess(&trust_evidence);
    let policy_verified = trust_assessment.level >= TrustLevel::Verified;
    if verified != policy_verified {
        return Err(AnalysisError::Message(format!(
            "legacy verification flag ({verified}) disagrees with policy-derived trust ({:?})",
            trust_assessment.level
        )));
    }
    let receipt = ExecutionReceipt {
        command_id: dispatch.command.id,
        command_timestamp: dispatch.command.timestamp,
        workflow_id: workflow.id,
        workflow_digest,
        source_snapshots: source_snapshots.clone(),
        input_snapshots: input_snapshots.clone(),
        crs: Some(Crs::parse(crs).map_err(|error| AnalysisError::Message(error.to_string()))?),
        coordinate_unit: coordinate_units.to_string(),
        value_unit: units.to_string(),
        area_method: area_method.to_string(),
        verifier: verifier.to_string(),
        verification_passed: verified,
        checks: checks.to_vec(),
        verification_policy: Some(verification_policy),
        verification_graph: Some(verification_graph),
        verification_graph_digest: Some(verification_graph_digest),
        trust_assessment: Some(trust_assessment),
        trust_evidence: Some(trust_evidence),
        evidence,
        retrieval_events: dispatch.execution.events.clone(),
        engine,
        state_digest: dispatch.execution.state_digest.clone(),
        result_digest,
    };
    let receipt_json = serde_json::to_value(&receipt)
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    let source = source_metadata(dataset);
    let mut provenance = ProvenanceStore::default();
    provenance.entries.push(ProvenanceEntry {
        id: dispatch.command.id,
        timestamp: dispatch.command.timestamp,
        actor: format!("{:?}", dispatch.command.origin).to_lowercase(),
        action: "execute_verified_workflow".into(),
        target: dataset.id.clone(),
        details: serde_json::json!({
            "command_id": dispatch.command.id,
            "command_timestamp": dispatch.command.timestamp,
            "planner_mode": planner_mode,
            "source_uri": dataset.uri,
            "source_id": dataset.id,
            "stac_item_id": dataset.id,
            "workflow_digest": receipt.workflow_digest,
            "crs": crs,
            "coordinate_units": coordinate_units,
            "units": units,
            "area_method": area_method,
            "area_unit": if area_method == "n/a" { "n/a" } else { "km²" },
            "license": dataset.license,
            "source": source,
            "source_version": source.source_version,
            "checksum": source.checksum,
            "expected_checksum": source.expected_checksum,
            "observed_checksum": source.observed_checksum,
            "checksum_status": source.checksum_status,
            "verifier": verifier,
            "verification_passed": verified,
            "checks": checks,
            "engine": receipt.engine,
            "state_digest": receipt.state_digest,
            "result_digest": receipt.result_digest,
            "execution_receipt": receipt_json,
        }),
        agent_run_id: None,
        workflow_id: Some(workflow.id.to_string()),
    });
    Ok((provenance, receipt))
}

fn nagoya_trust_evidence(
    workflow: &GeoWorkflow,
    analysis: &AnalysisResult,
    sources: &[SourceMetadata],
    workflow_digest: &str,
    result_digest: &str,
    engine: &EngineIdentity,
    artifacts: &NagoyaArtifactDigests,
) -> TrustEvidence {
    let mut contracts = workflow
        .input_contracts
        .iter()
        .filter_map(|input| input.geo_contract.as_ref())
        .map(contract_evidence)
        .collect::<Vec<_>>();
    contracts.extend(
        workflow
            .steps
            .iter()
            .flat_map(|step| step.output_contracts.iter())
            .map(|output| contract_evidence(&output.contract)),
    );
    contracts.sort_by(|left, right| left.contract_id.cmp(&right.contract_id));

    let mut artifact_digests = Vec::new();
    if let Some(digest) = &artifacts.html_sha256 {
        artifact_digests.push(digest.clone());
    }
    if let Some(digest) = &artifacts.png_sha256 {
        artifact_digests.push(digest.clone());
    }

    TrustEvidence {
        replay: ReplayEvidence {
            workflow_digest: Some(workflow_digest.into()),
            result_digest: Some(result_digest.into()),
            backend_identity: Some(format!(
                "{}:{}:{}",
                engine.name, engine.version, engine.build
            )),
            artifact_digests,
        },
        contracts,
        sources: sources
            .iter()
            .map(|source| {
                let source_id = source
                    .dataset_id
                    .clone()
                    .unwrap_or_else(|| source.uri.clone());
                let assurance = nagoya_source_assurance(source);
                let assurance_digest = assurance
                    .as_ref()
                    .and_then(|assurance| assurance.digest().ok());
                SourceEvidence {
                    source_id,
                    checksum_status: source.checksum_status,
                    source_version_present: source
                        .source_version
                        .as_ref()
                        .is_some_and(|version| !version.is_empty()),
                    license_present: source
                        .license
                        .as_deref()
                        .is_some_and(|license| !license.trim().is_empty()),
                    snapshot_digest: source
                        .observed_checksum
                        .clone()
                        .or_else(|| source.checksum.clone()),
                    assurance,
                    assurance_digest,
                }
            })
            .collect(),
        checks: nagoya_verification_observations(analysis),
        attestation: None,
    }
}

fn contract_evidence(contract: &genegis_contract::GeoContract) -> ContractEvidence {
    let valid = contract.validate().is_ok();
    ContractEvidence {
        contract_id: contract.id.clone(),
        schema_version: contract.schema_version.clone(),
        valid,
        compatibility: if valid {
            CompatibilityStatus::Compatible
        } else {
            CompatibilityStatus::Indeterminate
        },
    }
}

/// Legacy compatibility builder for callers that only have an `AnalysisResult`.
/// New ask/CLI/Agent paths must use [`build_ask_result_from_dispatch`], which
/// consumes the typed output from the already completed command execution and
/// therefore cannot dispatch the Nagoya executor a second time.
pub fn build_ask_result(
    prompt: &str,
    plan: &PlanResult,
    analysis: AnalysisResult,
    dataset: DatasetRecord,
    duckdb_verified: bool,
) -> Result<AskPipelineResult, AnalysisError> {
    let (html, png) = if duckdb_verified {
        (
            export_html_map(&analysis, "名古屋市 人口密度"),
            export_png_map(&analysis, "名古屋市 人口密度")?,
        )
    } else {
        (String::new(), Vec::new())
    };
    let png_base64 = STANDARD.encode(&png);
    let artifact_digests = NagoyaArtifactDigests {
        html_sha256: (!html.is_empty())
            .then(|| format!("sha256:{:x}", Sha256::digest(html.as_bytes()))),
        png_sha256: (!png.is_empty()).then(|| format!("sha256:{:x}", Sha256::digest(&png))),
        html_bytes: html.len(),
        png_bytes: png.len(),
    };
    let output = NagoyaExecutionOutput {
        analysis: analysis.clone(),
        verification_passed: duckdb_verified,
        html: html.clone(),
        png_base64: png_base64.clone(),
        artifact_digests,
    };
    let topological_order = analysis
        .workflow
        .topological_order()
        .map(|nodes| {
            nodes
                .into_iter()
                .map(|node| node.as_str().to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let evidence = serde_json::json!({
        "legacy_builder": true,
        "topological_order": topological_order,
        "verification": output.analysis.verification,
        "verification_passed": duckdb_verified,
        "output_fields": [
            "ward_code", "ward_name", "population", "area_km2",
            "density_per_km2", "rings", "color"
        ],
        "artifact_digests": output.artifact_digests,
    });
    let static_executor = StaticNagoyaOutputExecutor {
        result_digest: canonical_nagoya_execution_digest(&output, &evidence),
        output: serde_json::to_value(&output)
            .map_err(|error| AnalysisError::Message(error.to_string()))?,
        evidence: evidence.clone(),
    };
    let mut summary = build_summary(&analysis, &dataset);
    let (command, workflow, provenance, execution_receipt) = execution_receipt_with_executor(
        plan,
        analysis.workflow.clone(),
        &dataset,
        &analysis.verification.crs,
        &analysis.verification.coordinate_unit,
        &analysis.verification.density_unit,
        &analysis.verification.area_method,
        duckdb_verified,
        "duckdb_verify",
        &analysis.verification.checks,
        Some(&static_executor),
    )?;
    if let Some(summary_object) = summary.as_object_mut() {
        summary_object.insert(
            "command".into(),
            serde_json::to_value(&command)
                .map_err(|err| AnalysisError::Message(err.to_string()))?,
        );
        summary_object.insert(
            "workflow".into(),
            serde_json::to_value(&workflow)
                .map_err(|err| AnalysisError::Message(err.to_string()))?,
        );
        summary_object.insert(
            "provenance".into(),
            serde_json::to_value(&provenance)
                .map_err(|err| AnalysisError::Message(err.to_string()))?,
        );
        summary_object.insert(
            "execution_receipt".into(),
            serde_json::to_value(&execution_receipt)
                .map_err(|err| AnalysisError::Message(err.to_string()))?,
        );
    }

    Ok(AskPipelineResult {
        prompt: prompt.to_string(),
        workflow_id: plan.resolved.workflow_id.as_str().to_string(),
        confidence: plan.resolved.confidence,
        ambiguities: plan.resolved.ambiguities.clone(),
        workflow_steps: plan.workflow.steps.len(),
        verification: analysis.verification.clone(),
        analysis: Some(analysis.clone()),
        summary,
        html,
        png,
        png_base64,
        duckdb_verified,
        dataset: dataset.clone(),
        stac_item: dataset.to_stac_item(),
        command,
        execution_receipt,
        workflow,
        provenance,
    })
}

pub fn execute_from_plan(
    prompt: &str,
    plan: &PlanResult,
) -> Result<AskPipelineResult, AnalysisError> {
    execute_from_plan_with_origin(prompt, plan, CommandOrigin::Ai)
}

pub fn execute_from_plan_with_origin(
    prompt: &str,
    plan: &PlanResult,
    origin: CommandOrigin,
) -> Result<AskPipelineResult, AnalysisError> {
    let (executed, dataset) = execute_workflow_for_plan_with_origin(plan, origin)?;
    match executed {
        ExecutedWorkflowOutput::NagoyaDensity(dispatch) => {
            if !dispatch.output.verification_passed {
                return Err(AnalysisError::Message(
                    "workflow verification failed; render artifacts were withheld".into(),
                ));
            }
            build_ask_result_from_dispatch(prompt, plan, dispatch, dataset)
        }
        ExecutedWorkflowOutput::CogMetadata(info) => {
            let verified = verify_remote_cog_metadata(&info)?;
            if !verified {
                return Err(AnalysisError::Message(
                    "workflow verification failed".into(),
                ));
            }
            build_remote_cog_ask_result_with_origin(prompt, plan, info, dataset, verified, origin)
        }
        ExecutedWorkflowOutput::Geoparquet(vector) => {
            let verified = verify_geoparquet_features(&vector)?;
            if !verified {
                return Err(AnalysisError::Message(
                    "workflow verification failed".into(),
                ));
            }
            build_geoparquet_ask_result_with_origin(prompt, plan, vector, dataset, verified, origin)
        }
        ExecutedWorkflowOutput::DashboardExport(report) => {
            let verified = verify_dashboard_export(&report)?;
            if !verified {
                return Err(AnalysisError::Message(
                    "workflow verification failed".into(),
                ));
            }
            build_dashboard_ask_result_with_origin(prompt, plan, report, dataset, verified, origin)
        }
        ExecutedWorkflowOutput::FloodExposure(analysis) => {
            let verified = verify_flood_exposure_analysis(&analysis)?;
            if !verified {
                return Err(AnalysisError::Message(
                    "workflow verification failed".into(),
                ));
            }
            build_flood_ask_result_with_origin(prompt, plan, analysis, dataset, verified, origin)
        }
        ExecutedWorkflowOutput::Accessibility(analysis) => {
            let verified = verify_accessibility_analysis(&analysis)?;
            if !verified {
                return Err(AnalysisError::Message(
                    "workflow verification failed".into(),
                ));
            }
            build_accessibility_ask_result_with_origin(
                prompt, plan, analysis, dataset, verified, origin,
            )
        }
        ExecutedWorkflowOutput::Evacuation(analysis) => {
            let verified = verify_evacuation_analysis(&analysis)?;
            if !verified {
                return Err(AnalysisError::Message(
                    "workflow verification failed".into(),
                ));
            }
            build_evacuation_ask_result_with_origin(
                prompt, plan, analysis, dataset, verified, origin,
            )
        }
        ExecutedWorkflowOutput::NdviTimeseries(analysis) => {
            let verified = verify_ndvi_timeseries_analysis(&analysis)?;
            if !verified {
                return Err(AnalysisError::Message(
                    "workflow verification failed".into(),
                ));
            }
            build_ndvi_ask_result_with_origin(prompt, plan, analysis, dataset, verified, origin)
        }
        ExecutedWorkflowOutput::PointCloudChange(analysis) => {
            let verified = verify_change_detection_analysis(&analysis)?;
            if !verified {
                return Err(AnalysisError::Message(
                    "workflow verification failed".into(),
                ));
            }
            build_change_ask_result_with_origin(prompt, plan, analysis, dataset, verified, origin)
        }
        ExecutedWorkflowOutput::StacCollection(collection) => {
            let verified = verify_stac_collection(&collection)?;
            if !verified {
                return Err(AnalysisError::Message(
                    "workflow verification failed".into(),
                ));
            }
            build_stac_collection_ask_result_with_origin(
                prompt, plan, collection, dataset, verified, origin,
            )
        }
    }
}

pub fn build_remote_cog_ask_result(
    prompt: &str,
    plan: &PlanResult,
    info: CogInfo,
    dataset: DatasetRecord,
    verified: bool,
) -> Result<AskPipelineResult, AnalysisError> {
    build_remote_cog_ask_result_with_origin(
        prompt,
        plan,
        info,
        dataset,
        verified,
        CommandOrigin::Ai,
    )
}

fn build_remote_cog_ask_result_with_origin(
    prompt: &str,
    plan: &PlanResult,
    info: CogInfo,
    dataset: DatasetRecord,
    verified: bool,
    origin: CommandOrigin,
) -> Result<AskPipelineResult, AnalysisError> {
    let source = source_metadata(&dataset);
    let mut summary = serde_json::json!({
        "goal": plan.resolved.goal,
        "dataset": dataset.summary_json(),
        "cog": info.summary_json(),
        "verification_passed": verified && source.checksum_verified(),
        "read_mode": info.read_mode,
        "source": source,
    });
    let checks = source_checks(&source);

    let (command, workflow, provenance, execution_receipt) = execution_receipt_with_origin(
        plan,
        plan.workflow.clone(),
        &dataset,
        &info.crs,
        coordinate_unit_for_crs(&info.crs),
        "raster pixels",
        "n/a",
        verified,
        "cog_metadata_verify",
        origin,
        &checks,
    )?;
    add_receipt_to_summary(&mut summary, &execution_receipt)?;

    Ok(AskPipelineResult {
        prompt: prompt.to_string(),
        workflow_id: plan.resolved.workflow_id.as_str().to_string(),
        confidence: plan.resolved.confidence,
        ambiguities: plan.resolved.ambiguities.clone(),
        workflow_steps: plan.workflow.steps.len(),
        verification: VerificationReport {
            crs: info.crs.clone(),
            coordinate_unit: coordinate_unit_for_crs(&info.crs).into(),
            area_unit: "n/a".into(),
            area_method: "n/a".into(),
            density_unit: "n/a".into(),
            source: source.clone(),
            checks,
        },
        analysis: None,
        summary,
        html: String::new(),
        png: Vec::new(),
        png_base64: String::new(),
        duckdb_verified: verified,
        dataset: dataset.clone(),
        stac_item: dataset.to_stac_item(),
        command,
        execution_receipt,
        workflow,
        provenance,
    })
}

pub fn build_geoparquet_ask_result(
    prompt: &str,
    plan: &PlanResult,
    vector: VectorDataset,
    dataset: DatasetRecord,
    verified: bool,
) -> Result<AskPipelineResult, AnalysisError> {
    build_geoparquet_ask_result_with_origin(
        prompt,
        plan,
        vector,
        dataset,
        verified,
        CommandOrigin::Ai,
    )
}

fn build_geoparquet_ask_result_with_origin(
    prompt: &str,
    plan: &PlanResult,
    vector: VectorDataset,
    dataset: DatasetRecord,
    verified: bool,
    origin: CommandOrigin,
) -> Result<AskPipelineResult, AnalysisError> {
    let source = source_metadata(&dataset);
    let mut summary = serde_json::json!({
        "goal": plan.resolved.goal,
        "dataset": dataset.summary_json(),
        "geoparquet": geoparquet_summary(&vector),
        "verification_passed": verified && source.checksum_verified(),
        "source": source,
    });
    let checks = source_checks(&source);

    let (command, workflow, provenance, execution_receipt) = execution_receipt_with_origin(
        plan,
        plan.workflow.clone(),
        &dataset,
        &vector.crs,
        coordinate_unit_for_crs(&vector.crs),
        "declared by attributes",
        "n/a",
        verified,
        "geoparquet_feature_verify",
        origin,
        &checks,
    )?;
    add_receipt_to_summary(&mut summary, &execution_receipt)?;

    Ok(AskPipelineResult {
        prompt: prompt.to_string(),
        workflow_id: plan.resolved.workflow_id.as_str().to_string(),
        confidence: plan.resolved.confidence,
        ambiguities: plan.resolved.ambiguities.clone(),
        workflow_steps: plan.workflow.steps.len(),
        verification: VerificationReport {
            crs: vector.crs.clone(),
            coordinate_unit: coordinate_unit_for_crs(&vector.crs).into(),
            area_unit: "n/a".into(),
            area_method: "n/a".into(),
            density_unit: "n/a".into(),
            source: source.clone(),
            checks,
        },
        analysis: None,
        summary,
        html: String::new(),
        png: Vec::new(),
        png_base64: String::new(),
        duckdb_verified: verified,
        dataset: dataset.clone(),
        stac_item: dataset.to_stac_item(),
        command,
        execution_receipt,
        workflow,
        provenance,
    })
}

pub fn build_stac_collection_ask_result(
    prompt: &str,
    plan: &PlanResult,
    collection: StacCollection,
    dataset: DatasetRecord,
    verified: bool,
) -> Result<AskPipelineResult, AnalysisError> {
    build_stac_collection_ask_result_with_origin(
        prompt,
        plan,
        collection,
        dataset,
        verified,
        CommandOrigin::Ai,
    )
}

/// Assemble the ask result for a verified PMTiles dashboard export.
pub fn build_flood_ask_result(
    prompt: &str,
    plan: &PlanResult,
    analysis: FloodExposureAnalysis,
    dataset: DatasetRecord,
    verified: bool,
) -> Result<AskPipelineResult, AnalysisError> {
    build_flood_ask_result_with_origin(prompt, plan, analysis, dataset, verified, CommandOrigin::Ai)
}

fn build_flood_ask_result_with_origin(
    prompt: &str,
    plan: &PlanResult,
    analysis: FloodExposureAnalysis,
    dataset: DatasetRecord,
    verified: bool,
    origin: CommandOrigin,
) -> Result<AskPipelineResult, AnalysisError> {
    let source = source_metadata(&dataset);
    let html = export_flood_html_map(&analysis, &plan.resolved.goal);
    let mut summary = serde_json::json!({
        "goal": plan.resolved.goal,
        "dataset": dataset.summary_json(),
        "flood_exposure": {
            "zone_count": analysis.zone_count,
            "samples_per_axis": analysis.samples_per_axis,
            "wards": analysis.features.iter().map(|feature| serde_json::json!({
                "ward_name": feature.ward_name,
                "population": feature.population,
                "exposed_population": feature.exposed_population,
                "exposure_rate": feature.exposure_rate,
                "max_depth_class_m": feature.max_depth_class_m,
            })).collect::<Vec<_>>(),
        },
        "verification_passed": verified && source.checksum_verified(),
        "source": source,
    });
    let checks = analysis.verification.checks.clone();

    let (command, workflow, provenance, execution_receipt) = execution_receipt_with_origin(
        plan,
        plan.workflow.clone(),
        &dataset,
        &dataset.crs,
        coordinate_unit_for_crs(&dataset.crs),
        "exposed persons",
        "grid_sampling_even_odd_point_in_polygon",
        verified,
        "duckdb_verify",
        origin,
        &checks,
    )?;
    add_receipt_to_summary(&mut summary, &execution_receipt)?;

    Ok(AskPipelineResult {
        prompt: prompt.to_string(),
        workflow_id: plan.resolved.workflow_id.as_str().to_string(),
        confidence: plan.resolved.confidence,
        ambiguities: plan.resolved.ambiguities.clone(),
        workflow_steps: plan.workflow.steps.len(),
        verification: VerificationReport {
            crs: dataset.crs.clone(),
            coordinate_unit: coordinate_unit_for_crs(&dataset.crs).into(),
            area_unit: "n/a".into(),
            area_method: "grid_sampling_even_odd_point_in_polygon".into(),
            density_unit: "exposed persons".into(),
            source: source.clone(),
            checks,
        },
        analysis: None,
        summary,
        html,
        png: Vec::new(),
        png_base64: String::new(),
        duckdb_verified: verified,
        dataset: dataset.clone(),
        stac_item: dataset.to_stac_item(),
        command,
        execution_receipt,
        workflow,
        provenance,
    })
}

/// Assemble the ask result for a verified 15-minute-city analysis.
pub fn build_accessibility_ask_result(
    prompt: &str,
    plan: &PlanResult,
    analysis: AccessibilityAnalysis,
    dataset: DatasetRecord,
    verified: bool,
) -> Result<AskPipelineResult, AnalysisError> {
    build_accessibility_ask_result_with_origin(
        prompt,
        plan,
        analysis,
        dataset,
        verified,
        CommandOrigin::Ai,
    )
}

fn build_accessibility_ask_result_with_origin(
    prompt: &str,
    plan: &PlanResult,
    analysis: AccessibilityAnalysis,
    dataset: DatasetRecord,
    verified: bool,
    origin: CommandOrigin,
) -> Result<AskPipelineResult, AnalysisError> {
    let source = source_metadata(&dataset);
    let mut summary = serde_json::json!({
        "goal": plan.resolved.goal,
        "dataset": dataset.summary_json(),
        "accessibility": {
            "threshold_minutes": analysis.threshold_minutes,
            "graph": {
                "nodes": analysis.node_count,
                "edges": analysis.edge_count,
                "length_km": analysis.total_length_km,
            },
            "wards": analysis.features.iter().map(|feature| serde_json::json!({
                "ward_name": feature.ward_name,
                "population": feature.population,
                "reachable_pois": feature.reachable_pois,
                "poi_total": feature.poi_total,
                "isochrone_area_m2": feature.isochrone_area_m2,
                "nearest_cost_minutes": feature.nearest_cost_minutes,
                "accessibility_score": feature.accessibility_score,
            })).collect::<Vec<_>>(),
        },
        "verification_passed": verified && source.checksum_verified(),
        "source": source,
    });
    let checks = analysis.verification.checks.clone();

    let (command, workflow, provenance, execution_receipt) = execution_receipt_with_origin(
        plan,
        plan.workflow.clone(),
        &dataset,
        &dataset.crs,
        coordinate_unit_for_crs(&dataset.crs),
        "reachable POI share",
        "dijkstra_grid_walk_graph",
        verified,
        "route_sanity_verify",
        origin,
        &checks,
    )?;
    add_receipt_to_summary(&mut summary, &execution_receipt)?;

    Ok(AskPipelineResult {
        prompt: prompt.to_string(),
        workflow_id: plan.resolved.workflow_id.as_str().to_string(),
        confidence: plan.resolved.confidence,
        ambiguities: plan.resolved.ambiguities.clone(),
        workflow_steps: plan.workflow.steps.len(),
        verification: VerificationReport {
            crs: dataset.crs.clone(),
            coordinate_unit: coordinate_unit_for_crs(&dataset.crs).into(),
            area_unit: "n/a".into(),
            area_method: "dijkstra_grid_walk_graph".into(),
            density_unit: "reachable POI share".into(),
            source: source.clone(),
            checks,
        },
        analysis: None,
        summary,
        html: String::new(),
        png: Vec::new(),
        png_base64: String::new(),
        duckdb_verified: false,
        dataset: dataset.clone(),
        stac_item: dataset.to_stac_item(),
        command,
        execution_receipt,
        workflow,
        provenance,
    })
}

/// Assemble the ask result for a verified flood-penalized evacuation analysis.
pub fn build_evacuation_ask_result(
    prompt: &str,
    plan: &PlanResult,
    analysis: EvacuationAnalysis,
    dataset: DatasetRecord,
    verified: bool,
) -> Result<AskPipelineResult, AnalysisError> {
    build_evacuation_ask_result_with_origin(
        prompt,
        plan,
        analysis,
        dataset,
        verified,
        CommandOrigin::Ai,
    )
}

fn build_evacuation_ask_result_with_origin(
    prompt: &str,
    plan: &PlanResult,
    analysis: EvacuationAnalysis,
    dataset: DatasetRecord,
    verified: bool,
    origin: CommandOrigin,
) -> Result<AskPipelineResult, AnalysisError> {
    let source = source_metadata(&dataset);
    let mut summary = serde_json::json!({
        "goal": plan.resolved.goal,
        "dataset": dataset.summary_json(),
        "evacuation": {
            "shelter_count": analysis.shelter_count,
            "depth_penalty_per_m": analysis.depth_penalty_per_m,
            "flooded_edge_count": analysis.flooded_edge_count,
            "flooded_length_share": analysis.flooded_length_share,
            "graph": {
                "nodes": analysis.node_count,
                "edges": analysis.edge_count,
                "length_km": analysis.total_length_km,
            },
            "wards": analysis.features.iter().map(|feature| serde_json::json!({
                "ward_name": feature.ward_name,
                "population": feature.population,
                "shelter_name": feature.shelter_name,
                "baseline_minutes": feature.baseline_minutes,
                "flooded_minutes": feature.flooded_minutes,
                "delay_minutes": feature.delay_minutes,
                "flooded_shelter_name": feature.flooded_shelter_name,
            })).collect::<Vec<_>>(),
        },
        "verification_passed": verified && source.checksum_verified(),
        "source": source,
    });
    let checks = analysis.verification.checks.clone();

    let (command, workflow, provenance, execution_receipt) = execution_receipt_with_origin(
        plan,
        plan.workflow.clone(),
        &dataset,
        &dataset.crs,
        coordinate_unit_for_crs(&dataset.crs),
        "evacuation minutes",
        "dijkstra_penalized_walk_graph",
        verified,
        "route_sanity_verify",
        origin,
        &checks,
    )?;
    add_receipt_to_summary(&mut summary, &execution_receipt)?;

    Ok(AskPipelineResult {
        prompt: prompt.to_string(),
        workflow_id: plan.resolved.workflow_id.as_str().to_string(),
        confidence: plan.resolved.confidence,
        ambiguities: plan.resolved.ambiguities.clone(),
        workflow_steps: plan.workflow.steps.len(),
        verification: VerificationReport {
            crs: dataset.crs.clone(),
            coordinate_unit: coordinate_unit_for_crs(&dataset.crs).into(),
            area_unit: "n/a".into(),
            area_method: "dijkstra_penalized_walk_graph".into(),
            density_unit: "evacuation minutes".into(),
            source: source.clone(),
            checks,
        },
        analysis: None,
        summary,
        html: String::new(),
        png: Vec::new(),
        png_base64: String::new(),
        duckdb_verified: false,
        dataset: dataset.clone(),
        stac_item: dataset.to_stac_item(),
        command,
        execution_receipt,
        workflow,
        provenance,
    })
}

/// Assemble the ask result for a verified NDVI time-series run.
/// Assemble the ask result for a verified point-cloud change detection run.
pub fn build_change_ask_result(
    prompt: &str,
    plan: &PlanResult,
    analysis: ChangeDetectionAnalysis,
    dataset: DatasetRecord,
    verified: bool,
) -> Result<AskPipelineResult, AnalysisError> {
    build_change_ask_result_with_origin(
        prompt,
        plan,
        analysis,
        dataset,
        verified,
        CommandOrigin::Ai,
    )
}

fn build_change_ask_result_with_origin(
    prompt: &str,
    plan: &PlanResult,
    analysis: ChangeDetectionAnalysis,
    dataset: DatasetRecord,
    verified: bool,
    origin: CommandOrigin,
) -> Result<AskPipelineResult, AnalysisError> {
    let source = source_metadata(&dataset);
    let mut summary = serde_json::json!({
        "goal": plan.resolved.goal,
        "dataset": dataset.summary_json(),
        "change_detection": {
            "epoch_a_points": analysis.epoch_a_points,
            "epoch_b_points": analysis.epoch_b_points,
            "cell_size_m": analysis.cell_size_m,
            "cells_compared": analysis.cells_compared,
            "aoi_bounds_xy": analysis.aoi_bounds_xy,
            "classes": analysis.summaries.iter().map(|summary| serde_json::json!({
                "class": summary.class,
                "cell_count": summary.cell_count,
                "mean_delta_m": summary.mean_delta_m,
            })).collect::<Vec<_>>(),
        },
        "verification_passed": verified && source.checksum_verified(),
        "source": source,
    });
    let checks = analysis.verification.checks.clone();

    let (command, workflow, provenance, execution_receipt) = execution_receipt_with_origin(
        plan,
        plan.workflow.clone(),
        &dataset,
        &dataset.crs,
        coordinate_unit_for_crs(&dataset.crs),
        "nDSM delta (m)",
        "cell_p90_height_diff",
        verified,
        "volume_delta_verify",
        origin,
        &checks,
    )?;
    add_receipt_to_summary(&mut summary, &execution_receipt)?;

    Ok(AskPipelineResult {
        prompt: prompt.to_string(),
        workflow_id: plan.resolved.workflow_id.as_str().to_string(),
        confidence: plan.resolved.confidence,
        ambiguities: plan.resolved.ambiguities.clone(),
        workflow_steps: plan.workflow.steps.len(),
        verification: VerificationReport {
            crs: dataset.crs.clone(),
            coordinate_unit: coordinate_unit_for_crs(&dataset.crs).into(),
            area_unit: "m²".into(),
            area_method: "cell_p90_height_diff".into(),
            density_unit: "nDSM Δ (m)".into(),
            source: source.clone(),
            checks,
        },
        analysis: None,
        summary,
        html: String::new(),
        png: Vec::new(),
        png_base64: String::new(),
        duckdb_verified: false,
        dataset: dataset.clone(),
        stac_item: dataset.to_stac_item(),
        command,
        execution_receipt,
        workflow,
        provenance,
    })
}

pub fn build_ndvi_ask_result(
    prompt: &str,
    plan: &PlanResult,
    analysis: NdviTimeseriesAnalysis,
    dataset: DatasetRecord,
    verified: bool,
) -> Result<AskPipelineResult, AnalysisError> {
    build_ndvi_ask_result_with_origin(prompt, plan, analysis, dataset, verified, CommandOrigin::Ai)
}

fn build_ndvi_ask_result_with_origin(
    prompt: &str,
    plan: &PlanResult,
    analysis: NdviTimeseriesAnalysis,
    dataset: DatasetRecord,
    verified: bool,
    origin: CommandOrigin,
) -> Result<AskPipelineResult, AnalysisError> {
    let source = source_metadata(&dataset);
    let mut summary = serde_json::json!({
        "goal": plan.resolved.goal,
        "dataset": dataset.summary_json(),
        "ndvi_timeseries": {
            "epochs": analysis.epochs,
            "raster": { "width": analysis.width, "height": analysis.height },
            "wards": analysis.features.iter().map(|feature| serde_json::json!({
                "ward_name": feature.ward_name,
                "population": feature.population,
                "mean_ndvi_per_epoch": feature.mean_ndvi_per_epoch,
                "delta_ndvi": feature.delta_ndvi,
                "change_class": feature.change_class,
                "sampled_pixels": feature.sampled_pixels,
            })).collect::<Vec<_>>(),
        },
        "verification_passed": verified && source.checksum_verified(),
        "source": source,
    });
    let checks = analysis.verification.checks.clone();

    let (command, workflow, provenance, execution_receipt) = execution_receipt_with_origin(
        plan,
        plan.workflow.clone(),
        &dataset,
        &dataset.crs,
        coordinate_unit_for_crs(&dataset.crs),
        "NDVI",
        "pixel_center_zonal_mean",
        verified,
        "index_range_verify",
        origin,
        &checks,
    )?;
    add_receipt_to_summary(&mut summary, &execution_receipt)?;

    Ok(AskPipelineResult {
        prompt: prompt.to_string(),
        workflow_id: plan.resolved.workflow_id.as_str().to_string(),
        confidence: plan.resolved.confidence,
        ambiguities: plan.resolved.ambiguities.clone(),
        workflow_steps: plan.workflow.steps.len(),
        verification: VerificationReport {
            crs: dataset.crs.clone(),
            coordinate_unit: coordinate_unit_for_crs(&dataset.crs).into(),
            area_unit: "n/a".into(),
            area_method: "pixel_center_zonal_mean".into(),
            density_unit: "NDVI".into(),
            source: source.clone(),
            checks,
        },
        analysis: None,
        summary,
        html: String::new(),
        png: Vec::new(),
        png_base64: String::new(),
        duckdb_verified: false,
        dataset: dataset.clone(),
        stac_item: dataset.to_stac_item(),
        command,
        execution_receipt,
        workflow,
        provenance,
    })
}

pub fn build_dashboard_ask_result(
    prompt: &str,
    plan: &PlanResult,
    report: DashboardExportReport,
    dataset: DatasetRecord,
    verified: bool,
) -> Result<AskPipelineResult, AnalysisError> {
    build_dashboard_ask_result_with_origin(
        prompt,
        plan,
        report,
        dataset,
        verified,
        CommandOrigin::Ai,
    )
}

fn build_stac_collection_ask_result_with_origin(
    prompt: &str,
    plan: &PlanResult,
    collection: StacCollection,
    dataset: DatasetRecord,
    verified: bool,
    origin: CommandOrigin,
) -> Result<AskPipelineResult, AnalysisError> {
    let source = source_metadata(&dataset);
    let mut summary = serde_json::json!({
        "goal": plan.resolved.goal,
        "dataset": dataset.summary_json(),
        "stac_collection": collection.summary_json(),
        "verification_passed": verified && source.checksum_verified(),
        "source": source,
    });
    let checks = source_checks(&source);

    let (command, workflow, provenance, execution_receipt) = execution_receipt_with_origin(
        plan,
        plan.workflow.clone(),
        &dataset,
        &dataset.crs,
        coordinate_unit_for_crs(&dataset.crs),
        "catalog metadata",
        "n/a",
        verified,
        "stac_collection_verify",
        origin,
        &checks,
    )?;
    add_receipt_to_summary(&mut summary, &execution_receipt)?;

    Ok(AskPipelineResult {
        prompt: prompt.to_string(),
        workflow_id: plan.resolved.workflow_id.as_str().to_string(),
        confidence: plan.resolved.confidence,
        ambiguities: plan.resolved.ambiguities.clone(),
        workflow_steps: plan.workflow.steps.len(),
        verification: VerificationReport {
            crs: dataset.crs.clone(),
            coordinate_unit: coordinate_unit_for_crs(&dataset.crs).into(),
            area_unit: "n/a".into(),
            area_method: "n/a".into(),
            density_unit: "n/a".into(),
            source: source.clone(),
            checks,
        },
        analysis: None,
        summary,
        html: String::new(),
        png: Vec::new(),
        png_base64: String::new(),
        duckdb_verified: verified,
        dataset: dataset.clone(),
        stac_item: dataset.to_stac_item(),
        command,
        execution_receipt,
        workflow,
        provenance,
    })
}

fn build_dashboard_ask_result_with_origin(
    prompt: &str,
    plan: &PlanResult,
    report: DashboardExportReport,
    dataset: DatasetRecord,
    verified: bool,
    origin: CommandOrigin,
) -> Result<AskPipelineResult, AnalysisError> {
    let source = source_metadata(&dataset);
    let export_json =
        serde_json::to_value(&report).map_err(|err| AnalysisError::Message(err.to_string()))?;
    let mut summary = serde_json::json!({
        "goal": plan.resolved.goal,
        "dataset": dataset.summary_json(),
        "dashboard_export": export_json,
        "verification_passed": verified && source.checksum_verified(),
        "source": source,
    });
    let checks = report.checks.clone();

    let (command, workflow, provenance, execution_receipt) = execution_receipt_with_origin(
        plan,
        plan.workflow.clone(),
        &dataset,
        &dataset.crs,
        coordinate_unit_for_crs(&dataset.crs),
        "tiles",
        "n/a",
        verified,
        "tile_roundtrip_verify",
        origin,
        &checks,
    )?;
    add_receipt_to_summary(&mut summary, &execution_receipt)?;

    Ok(AskPipelineResult {
        prompt: prompt.to_string(),
        workflow_id: plan.resolved.workflow_id.as_str().to_string(),
        confidence: plan.resolved.confidence,
        ambiguities: plan.resolved.ambiguities.clone(),
        workflow_steps: plan.workflow.steps.len(),
        verification: VerificationReport {
            crs: dataset.crs.clone(),
            coordinate_unit: coordinate_unit_for_crs(&dataset.crs).into(),
            area_unit: "n/a".into(),
            area_method: "n/a".into(),
            density_unit: "n/a".into(),
            source: source.clone(),
            checks,
        },
        analysis: None,
        summary,
        html: String::new(),
        png: Vec::new(),
        png_base64: String::new(),
        duckdb_verified: verified,
        dataset: dataset.clone(),
        stac_item: dataset.to_stac_item(),
        command,
        execution_receipt,
        workflow,
        provenance,
    })
}

fn execution_receipt_with_origin(
    plan: &PlanResult,
    workflow: GeoWorkflow,
    dataset: &DatasetRecord,
    crs: &str,
    coordinate_units: &str,
    units: &str,
    area_method: &str,
    verified: bool,
    verifier: &str,
    origin: CommandOrigin,
    checks: &[VerificationCheck],
) -> Result<
    (
        CommandEnvelope,
        GeoWorkflow,
        ProvenanceStore,
        ExecutionReceipt,
    ),
    AnalysisError,
> {
    execution_receipt_with_executor_and_origin(
        plan,
        workflow,
        dataset,
        crs,
        coordinate_units,
        units,
        area_method,
        verified,
        verifier,
        origin,
        checks,
        None,
    )
}

fn execution_receipt_with_executor(
    plan: &PlanResult,
    workflow: GeoWorkflow,
    dataset: &DatasetRecord,
    crs: &str,
    coordinate_units: &str,
    units: &str,
    area_method: &str,
    verified: bool,
    verifier: &str,
    checks: &[VerificationCheck],
    executor: Option<&dyn WorkflowExecutor>,
) -> Result<
    (
        CommandEnvelope,
        GeoWorkflow,
        ProvenanceStore,
        ExecutionReceipt,
    ),
    AnalysisError,
> {
    execution_receipt_with_executor_and_origin(
        plan,
        workflow,
        dataset,
        crs,
        coordinate_units,
        units,
        area_method,
        verified,
        verifier,
        CommandOrigin::Ai,
        checks,
        executor,
    )
}

fn execution_receipt_with_executor_and_origin(
    plan: &PlanResult,
    workflow: GeoWorkflow,
    dataset: &DatasetRecord,
    crs: &str,
    coordinate_units: &str,
    units: &str,
    area_method: &str,
    verified: bool,
    verifier: &str,
    origin: CommandOrigin,
    checks: &[VerificationCheck],
    executor: Option<&dyn WorkflowExecutor>,
) -> Result<
    (
        CommandEnvelope,
        GeoWorkflow,
        ProvenanceStore,
        ExecutionReceipt,
    ),
    AnalysisError,
> {
    execution_receipt_for_workflow_with_checks_and_executor(
        workflow,
        dataset,
        crs,
        coordinate_units,
        units,
        area_method,
        verified,
        verifier,
        origin,
        &plan.mode,
        checks,
        executor,
    )
}

/// Build a source-aware command and provenance receipt for a workflow.
///
/// Source snapshots are derived from catalog metadata and local bytes. The
/// command/provenance event timestamps describe execution, while
/// `retrieved_at` remains absent unless an adapter supplied it explicitly and
/// therefore does not enter the stable source snapshot.
pub fn execution_receipt_for_workflow(
    workflow: GeoWorkflow,
    dataset: &DatasetRecord,
    crs: &str,
    coordinate_units: &str,
    units: &str,
    area_method: &str,
    verified: bool,
    verifier: &str,
    origin: CommandOrigin,
    planner_mode: &str,
) -> (CommandEnvelope, GeoWorkflow, ProvenanceStore) {
    let checks = source_checks(&source_metadata(dataset));
    let (command, workflow, provenance, _) = execution_receipt_for_workflow_with_checks(
        workflow,
        dataset,
        crs,
        coordinate_units,
        units,
        area_method,
        verified,
        verifier,
        origin,
        planner_mode,
        &checks,
    )
    .expect("workflow receipt must be valid before dispatch");
    (command, workflow, provenance)
}

/// Build and dispatch a source-aware workflow receipt, retaining the full
/// verification check list in the machine-readable evidence object.
pub fn execution_receipt_for_workflow_with_checks(
    workflow: GeoWorkflow,
    dataset: &DatasetRecord,
    crs: &str,
    coordinate_units: &str,
    units: &str,
    area_method: &str,
    verified: bool,
    verifier: &str,
    origin: CommandOrigin,
    planner_mode: &str,
    checks: &[VerificationCheck],
) -> Result<
    (
        CommandEnvelope,
        GeoWorkflow,
        ProvenanceStore,
        ExecutionReceipt,
    ),
    AnalysisError,
> {
    execution_receipt_for_workflow_with_checks_and_executor(
        workflow,
        dataset,
        crs,
        coordinate_units,
        units,
        area_method,
        verified,
        verifier,
        origin,
        planner_mode,
        checks,
        None,
    )
}

/// Build and dispatch a receipt with a domain executor supplied by the
/// caller. CLI, AI, and plugins can therefore choose their command origin
/// while sharing the exact same validation/dispatch/receipt path.
pub fn execution_receipt_for_workflow_with_executor(
    workflow: GeoWorkflow,
    dataset: &DatasetRecord,
    crs: &str,
    coordinate_units: &str,
    units: &str,
    area_method: &str,
    verified: bool,
    verifier: &str,
    origin: CommandOrigin,
    planner_mode: &str,
    checks: &[VerificationCheck],
    executor: &dyn WorkflowExecutor,
) -> Result<
    (
        CommandEnvelope,
        GeoWorkflow,
        ProvenanceStore,
        ExecutionReceipt,
    ),
    AnalysisError,
> {
    execution_receipt_for_workflow_with_checks_and_executor(
        workflow,
        dataset,
        crs,
        coordinate_units,
        units,
        area_method,
        verified,
        verifier,
        origin,
        planner_mode,
        checks,
        Some(executor),
    )
}

fn execution_receipt_for_workflow_with_checks_and_executor(
    mut workflow: GeoWorkflow,
    dataset: &DatasetRecord,
    crs: &str,
    coordinate_units: &str,
    units: &str,
    area_method: &str,
    verified: bool,
    verifier: &str,
    origin: CommandOrigin,
    planner_mode: &str,
    checks: &[VerificationCheck],
    executor: Option<&dyn WorkflowExecutor>,
) -> Result<
    (
        CommandEnvelope,
        GeoWorkflow,
        ProvenanceStore,
        ExecutionReceipt,
    ),
    AnalysisError,
> {
    workflow
        .validate()
        .map_err(|error| AnalysisError::Message(format!("invalid workflow graph: {error}")))?;
    let workflow_digest = WorkflowDigest::new(
        workflow
            .stable_digest()
            .map_err(|error| AnalysisError::Message(format!("workflow digest failed: {error}")))?,
    );
    let source = source_metadata(dataset);
    let verification_passed =
        verified && source.checksum_verified() && checks.iter().all(|check| check.passed);
    workflow.review_status = if verification_passed {
        ReviewStatus::Executed
    } else {
        ReviewStatus::PendingReview
    };
    let typed_crs = Crs::parse(crs).map_err(|error| AnalysisError::Message(error.to_string()))?;
    typed_crs
        .require_known()
        .map_err(|error| AnalysisError::Message(error.to_string()))?;

    let mut input_snapshots = Vec::new();
    for contract in &workflow.input_contracts {
        if let Some(contract_source) = &contract.source_snapshot {
            let mut snapshot = InputSnapshot::new(contract.name.clone(), contract_source.clone());
            if let Some(contract_crs) = &contract.crs {
                snapshot = snapshot.with_crs(contract_crs.clone());
            }
            if let Some(value_unit) = &contract.value_unit {
                snapshot = snapshot.with_value_unit(value_unit.clone());
            }
            input_snapshots.push(snapshot);
        }
    }
    let mut source_snapshots = vec![source.clone()];
    for snapshot in &input_snapshots {
        if !source_snapshots
            .iter()
            .any(|candidate| same_source_identity(candidate, &snapshot.source))
        {
            source_snapshots.push(snapshot.source.clone());
        }
    }

    let mut command = CommandEnvelope::new(
        origin,
        Command::RunWorkflow {
            workflow_id: workflow.id,
        },
    )
    .with_workflow_digest(workflow_digest.clone());
    for snapshot in &source_snapshots {
        command = command.with_source_snapshot(snapshot.clone());
    }
    for snapshot in &input_snapshots {
        command = command.with_input_snapshot(snapshot.clone());
    }

    let engine = Default::default();
    let fallback_result_digest = canonical_result_digest(
        &workflow_digest,
        &source_snapshots,
        &input_snapshots,
        &typed_crs,
        coordinate_units,
        units,
        area_method,
        verified,
        checks,
        &engine,
    );
    let fallback_executor = ReceiptExecutor::new(
        fallback_result_digest,
        &workflow_digest,
        &source_snapshots,
        &input_snapshots,
        &typed_crs,
        coordinate_units,
        units,
        area_method,
        verified,
        checks,
        command.timestamp,
    );
    let dispatch_executor: &dyn WorkflowExecutor = match executor {
        Some(executor) => executor,
        None => &fallback_executor,
    };

    // The receipt itself uses the same dispatcher as UI/AI/CLI commands. A
    // RunWorkflow that is invalid, has a stale digest, or has stale inputs
    // therefore fails before a receipt can claim success.
    let mut project = Project::new("execution-receipt");
    // The receipt dispatcher has no user-authored project identity. Pin this
    // synthetic workspace so its state digest is reproducible across runs.
    project.manifest.workspace.id = Uuid::nil();
    let mut bus = CommandBus::new(project.clone());
    let execution = bus
        .apply_with_workflow_executor(
            &mut project,
            command.clone(),
            workflow.clone(),
            dispatch_executor,
        )
        .map_err(|error| AnalysisError::Message(format!("command dispatch failed: {error}")))?;
    let result_digest = execution.result_digest.clone().ok_or_else(|| {
        AnalysisError::Message("workflow dispatcher returned no result digest".into())
    })?;
    let evidence = execution
        .evidence
        .clone()
        .unwrap_or(serde_json::Value::Null);
    let retrieval_events = execution.events.clone();
    let receipt = ExecutionReceipt {
        command_id: command.id,
        command_timestamp: command.timestamp,
        workflow_id: workflow.id,
        workflow_digest,
        source_snapshots,
        input_snapshots,
        crs: Some(typed_crs),
        coordinate_unit: coordinate_units.to_string(),
        value_unit: units.to_string(),
        area_method: area_method.to_string(),
        verifier: verifier.to_string(),
        verification_passed,
        checks: checks.to_vec(),
        verification_policy: None,
        verification_graph: None,
        verification_graph_digest: None,
        trust_assessment: None,
        trust_evidence: None,
        evidence,
        retrieval_events,
        engine,
        state_digest: execution.state_digest,
        result_digest,
    };

    let receipt_json = serde_json::to_value(&receipt)
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    let mut provenance = ProvenanceStore::default();
    provenance.entries.push(ProvenanceEntry {
        id: command.id,
        timestamp: command.timestamp,
        actor: format!("{:?}", command.origin).to_lowercase(),
        action: "execute_verified_workflow".into(),
        target: dataset.id.clone(),
        details: serde_json::json!({
            "command_id": command.id,
            "command_timestamp": command.timestamp,
            "planner_mode": planner_mode,
            "source_uri": dataset.uri,
            "source_id": dataset.id,
            "stac_item_id": dataset.id,
            "workflow_digest": receipt.workflow_digest,
            "crs": crs,
            "coordinate_units": coordinate_units,
            "units": units,
            "area_method": area_method,
            "area_unit": if area_method == "n/a" { "n/a" } else { "km²" },
            "license": dataset.license,
            "source": source,
            "source_version": source.source_version,
            "checksum": source.checksum,
            "expected_checksum": source.expected_checksum,
            "observed_checksum": source.observed_checksum,
            "checksum_status": source.checksum_status,
            "verifier": verifier,
            "verification_passed": verification_passed,
            "checks": checks,
            "engine": receipt.engine,
            "state_digest": receipt.state_digest,
            "result_digest": receipt.result_digest,
            "execution_receipt": receipt_json,
        }),
        agent_run_id: None,
        workflow_id: Some(workflow.id.to_string()),
    });
    Ok((command, workflow, provenance, receipt))
}

/// Build the command-side authorization envelope for a resolved workflow.
/// Snapshot identity comparison deliberately ignores retrieval event time.
fn command_for_workflow(
    workflow: &GeoWorkflow,
    dataset: &DatasetRecord,
    origin: CommandOrigin,
) -> Result<
    (
        CommandEnvelope,
        Vec<SourceMetadata>,
        Vec<InputSnapshot>,
        WorkflowDigest,
    ),
    AnalysisError,
> {
    workflow
        .validate()
        .map_err(|error| AnalysisError::Message(format!("invalid workflow graph: {error}")))?;
    let workflow_digest = WorkflowDigest::new(
        workflow
            .stable_digest()
            .map_err(|error| AnalysisError::Message(format!("workflow digest failed: {error}")))?,
    );
    let source = source_metadata(dataset);
    let mut input_snapshots = Vec::new();
    for contract in &workflow.input_contracts {
        if let Some(contract_source) = &contract.source_snapshot {
            let mut snapshot = InputSnapshot::new(contract.name.clone(), contract_source.clone());
            if let Some(contract_crs) = &contract.crs {
                snapshot = snapshot.with_crs(contract_crs.clone());
            }
            if let Some(value_unit) = &contract.value_unit {
                snapshot = snapshot.with_value_unit(value_unit.clone());
            }
            input_snapshots.push(snapshot);
        }
    }
    let mut source_snapshots = vec![source];
    for snapshot in &input_snapshots {
        if !source_snapshots
            .iter()
            .any(|candidate| same_source_identity(candidate, &snapshot.source))
        {
            source_snapshots.push(snapshot.source.clone());
        }
    }
    let mut command = CommandEnvelope::new(
        origin,
        Command::RunWorkflow {
            workflow_id: workflow.id,
        },
    )
    .with_workflow_digest(workflow_digest.clone());
    for snapshot in &source_snapshots {
        command = command.with_source_snapshot(snapshot.clone());
    }
    for snapshot in &input_snapshots {
        command = command.with_input_snapshot(snapshot.clone());
    }
    Ok((command, source_snapshots, input_snapshots, workflow_digest))
}

fn same_source_identity(left: &SourceMetadata, right: &SourceMetadata) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    left.retrieved_at = None;
    right.retrieved_at = None;
    left == right
}

/// Minimal executor used for legacy receipt builders that do not have a
/// domain-specific data-plane executor. It still goes through CommandBus and
/// emits stable evidence/events; the Nagoya path always supplies its real
/// executor instead.
struct ReceiptExecutor {
    result_digest: String,
    output: serde_json::Value,
    evidence: serde_json::Value,
    events: Vec<WorkflowExecutionEvent>,
}

impl ReceiptExecutor {
    #[allow(clippy::too_many_arguments)]
    fn new(
        result_digest: String,
        workflow_digest: &WorkflowDigest,
        source_snapshots: &[SourceMetadata],
        input_snapshots: &[InputSnapshot],
        crs: &Crs,
        coordinate_units: &str,
        units: &str,
        area_method: &str,
        verified: bool,
        checks: &[VerificationCheck],
        command_timestamp: DateTime<Utc>,
    ) -> Self {
        let events = source_snapshots
            .iter()
            .map(|source| WorkflowExecutionEvent {
                kind: "source_read".into(),
                source_uri: Some(source.uri.clone()),
                observed_at: source_event_time(source, command_timestamp),
                details: serde_json::json!({
                    "stable_identity": stable_source_identity(source),
                    "retrieved_at": source.retrieved_at,
                    "checksum_status": source.checksum_status,
                }),
            })
            .collect::<Vec<_>>();
        let evidence = serde_json::json!({
            "workflow_digest": workflow_digest,
            "crs": crs,
            "coordinate_units": coordinate_units,
            "units": units,
            "area_method": area_method,
            "verification_passed": verified,
            "checks": checks,
            "input_snapshots": input_snapshots,
        });
        let output = serde_json::json!({
            "workflow_digest": workflow_digest,
            "source_snapshots": source_snapshots,
            "input_snapshots": input_snapshots,
            "evidence": evidence,
        });
        Self {
            result_digest,
            output,
            evidence,
            events,
        }
    }
}

impl WorkflowExecutor for ReceiptExecutor {
    fn execute(
        &self,
        _workflow: &GeoWorkflow,
        _context: &WorkflowExecutionContext,
    ) -> Result<WorkflowExecution, WorkflowExecutionError> {
        Ok(WorkflowExecution {
            result_digest: self.result_digest.clone(),
            output: self.output.clone(),
            evidence: self.evidence.clone(),
            events: self.events.clone(),
        })
    }
}

/// Compatibility executor for the old `build_ask_result` API. The caller has
/// already supplied the analysis and render bytes, so this executor only
/// transports that immutable output through the CommandBus; it never invokes
/// the Nagoya data loader, verifier, or renderer again.
struct StaticNagoyaOutputExecutor {
    result_digest: String,
    output: serde_json::Value,
    evidence: serde_json::Value,
}

impl WorkflowExecutor for StaticNagoyaOutputExecutor {
    fn execute(
        &self,
        _workflow: &GeoWorkflow,
        context: &WorkflowExecutionContext,
    ) -> Result<WorkflowExecution, WorkflowExecutionError> {
        let events = context
            .source_snapshots
            .iter()
            .map(|source| WorkflowExecutionEvent {
                kind: "source_read".into(),
                source_uri: Some(source.uri.clone()),
                observed_at: source_event_time(source, context.command_timestamp),
                details: serde_json::json!({
                    "stable_identity": stable_source_identity(source),
                    "retrieved_at": source.retrieved_at,
                    "checksum_status": source.checksum_status,
                }),
            })
            .collect();
        Ok(WorkflowExecution {
            result_digest: self.result_digest.clone(),
            output: self.output.clone(),
            evidence: self.evidence.clone(),
            events,
        })
    }
}

fn source_event_time(source: &SourceMetadata, fallback: DateTime<Utc>) -> DateTime<Utc> {
    source
        .retrieved_at
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or(fallback)
}

fn stable_source_identity(source: &SourceMetadata) -> serde_json::Value {
    let mut stable = source.clone();
    stable.retrieved_at = None;
    serde_json::to_value(stable).expect("source metadata is serializable")
}

fn canonical_result_digest(
    workflow_digest: &WorkflowDigest,
    source_snapshots: &[SourceMetadata],
    input_snapshots: &[InputSnapshot],
    crs: &Crs,
    coordinate_units: &str,
    units: &str,
    area_method: &str,
    verified: bool,
    checks: &[VerificationCheck],
    engine: &EngineIdentity,
) -> String {
    let source_snapshots = source_snapshots
        .iter()
        .cloned()
        .map(|mut source| {
            source.retrieved_at = None;
            source
        })
        .collect::<Vec<_>>();
    let input_snapshots = input_snapshots
        .iter()
        .cloned()
        .map(|mut snapshot| {
            snapshot.source.retrieved_at = None;
            snapshot
        })
        .collect::<Vec<_>>();
    let document = serde_json::json!({
        "workflow_digest": workflow_digest,
        "source_snapshots": source_snapshots,
        "input_snapshots": input_snapshots,
        "crs": crs,
        "coordinate_units": coordinate_units,
        "units": units,
        "area_method": area_method,
        "verified": verified,
        "checks": checks,
        "engine": engine,
    });
    let canonical = serde_json::to_string(&document).expect("result receipt is serializable");
    format!("sha256:{:x}", Sha256::digest(canonical.as_bytes()))
}

fn build_summary(result: &AnalysisResult, dataset: &DatasetRecord) -> serde_json::Value {
    serde_json::json!({
        "goal": result.workflow.goal,
        "dataset": dataset.summary_json(),
        "ward_count": result.features.len(),
        "density_unit": result.verification.density_unit,
        "crs": result.verification.crs,
        "coordinate_unit": result.verification.coordinate_unit,
        "area_unit": result.verification.area_unit,
        "area_method": result.verification.area_method,
        "source": result.verification.source,
        "source_version": result.verification.source.source_version,
        "checksum": result.verification.source.checksum,
        "expected_checksum": result.verification.source.expected_checksum,
        "observed_checksum": result.verification.source.observed_checksum,
        "checksum_status": result.verification.source.checksum_status,
        "verification_passed": result.verification.checks.iter().all(|c| c.passed),
        "top_density_ward": result.features.iter()
            .max_by(|a, b| a.density_per_km2.partial_cmp(&b.density_per_km2).unwrap())
            .map(|f| serde_json::json!({
                "ward_name": f.ward_name,
                "density_per_km2": f.density_per_km2,
            })),
    })
}

fn add_receipt_to_summary(
    summary: &mut serde_json::Value,
    receipt: &ExecutionReceipt,
) -> Result<(), AnalysisError> {
    if let Some(summary_object) = summary.as_object_mut() {
        summary_object.insert(
            "execution_receipt".into(),
            serde_json::to_value(receipt)
                .map_err(|error| AnalysisError::Message(error.to_string()))?,
        );
    }
    Ok(())
}

fn source_metadata(dataset: &DatasetRecord) -> SourceMetadata {
    dataset.source_metadata()
}

fn source_checks(source: &SourceMetadata) -> Vec<crate::result::VerificationCheck> {
    vec![
        crate::result::VerificationCheck {
            name: "source_uri_declared".into(),
            passed: !source.uri.trim().is_empty(),
            detail: source.uri.clone(),
        },
        crate::result::VerificationCheck {
            name: "source_checksum".into(),
            passed: source.checksum_status == ChecksumVerification::Verified,
            detail: format!(
                "expected={} observed={} ({})",
                source.expected_checksum.as_deref().unwrap_or("unknown"),
                source.observed_checksum.as_deref().unwrap_or("unknown"),
                source.checksum_status
            ),
        },
    ]
}

fn coordinate_unit_for_crs(crs: &str) -> &'static str {
    Crs::parse(crs)
        .ok()
        .map(|parsed| parsed.coordinate_unit())
        .map(CoordinateUnit::as_str)
        .unwrap_or("unknown")
}

#[cfg(test)]
mod tests {
    use super::*;

    use genegis_catalog::NAGOYA_WARDS_DENSITY_ID;

    #[test]
    fn runs_north_star_pipeline() {
        let result = run_ask_pipeline("名古屋市の人口密度を表示").expect("pipeline");
        assert!(result.duckdb_verified);
        assert_eq!(result.workflow_steps, 14);
        assert!(result.html.contains("svg"));
        assert!(result.png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(
            result.png,
            STANDARD.decode(&result.png_base64).expect("png_base64")
        );
        assert_eq!(result.dataset.id, NAGOYA_WARDS_DENSITY_ID);
        assert!(result.summary.get("dataset").is_some());
        assert_eq!(result.stac_item.id, NAGOYA_WARDS_DENSITY_ID);
        assert!(result.stac_item.assets.contains_key("geojson"));
        match result.command.command {
            Command::RunWorkflow { workflow_id } => assert_eq!(workflow_id, result.workflow.id),
            _ => panic!("ask pipeline must emit RunWorkflow"),
        }
        assert_eq!(
            result
                .command
                .workflow_digest
                .as_ref()
                .map(|digest| digest.as_str()),
            Some(result.execution_receipt.workflow_digest.as_str())
        );
        assert_eq!(result.execution_receipt.workflow_id, result.workflow.id);
        assert_eq!(result.execution_receipt.command_id, result.command.id);
        assert!(result.execution_receipt.verification_passed);
        let trust = result
            .execution_receipt
            .trust_assessment
            .as_ref()
            .expect("policy-derived trust assessment");
        assert_eq!(trust.level, TrustLevel::Verified);
        assert_eq!(trust.policy_id, "genegis.nagoya-density.release-v1");
        assert_eq!(
            result
                .execution_receipt
                .verification_policy
                .as_ref()
                .map(|policy| policy.policy_id.as_str()),
            Some("genegis.nagoya-density.release-v1")
        );
        let verification_graph = result
            .execution_receipt
            .verification_graph
            .as_ref()
            .expect("explicit verification graph");
        assert_eq!(verification_graph.nodes.len(), 5);
        assert_eq!(
            result.execution_receipt.verification_graph_digest,
            Some(verification_graph.stable_digest().expect("graph digest"))
        );
        assert_eq!(
            result.execution_receipt.evidence["verification_graph_digest"],
            result
                .execution_receipt
                .verification_graph_digest
                .as_deref()
                .expect("receipt graph digest")
        );
        assert!(!result.execution_receipt.checks.is_empty());
        for check_name in [
            "population_total_oracle",
            "ward_coverage_oracle",
            "area_oracle_relative_error",
            "density_oracle",
        ] {
            assert!(
                result
                    .execution_receipt
                    .checks
                    .iter()
                    .any(|check| check.name == check_name && check.passed),
                "receipt missing passing {check_name}"
            );
        }
        assert_eq!(
            result.execution_receipt.evidence["topological_order"][0],
            "resolve-place"
        );
        assert_eq!(
            result.execution_receipt.evidence["topological_order"][13],
            "attach-sources"
        );
        assert!(!result.execution_receipt.retrieval_events.is_empty());
        assert_eq!(
            result.execution_receipt.retrieval_events[0].observed_at,
            result.command.timestamp
        );
        assert!(
            result.execution_receipt.retrieval_events[0].details["stable_identity"]["retrieved_at"]
                .is_null()
        );
        assert_eq!(
            result.execution_receipt.source_snapshots[0].checksum_status,
            ChecksumVerification::Verified
        );
        assert_eq!(
            result.summary["execution_receipt"]["workflow_digest"],
            result.execution_receipt.workflow_digest.as_str()
        );
        let repeated = run_ask_pipeline("名古屋市の人口密度を表示").expect("repeat pipeline");
        assert_eq!(
            result.execution_receipt.workflow_digest,
            repeated.execution_receipt.workflow_digest
        );
        assert_eq!(
            result.execution_receipt.result_digest,
            repeated.execution_receipt.result_digest
        );
        assert!(
            result.execution_receipt.evidence["artifact_digests"]["html_sha256"]
                .as_str()
                .is_some()
        );
        assert!(
            result.execution_receipt.evidence["artifact_digests"]["png_sha256"]
                .as_str()
                .is_some()
        );
        assert_eq!(
            result.execution_receipt.evidence["artifact_digests"]["html_sha256"],
            format!("sha256:{:x}", Sha256::digest(result.html.as_bytes()))
        );
        assert_eq!(
            result.execution_receipt.evidence["artifact_digests"]["png_sha256"],
            format!("sha256:{:x}", Sha256::digest(&result.png))
        );
        for _ in 0..8 {
            let repeated = run_ask_pipeline("名古屋市の人口密度を表示").expect("repeat pipeline");
            assert_eq!(
                result.execution_receipt.result_digest,
                repeated.execution_receipt.result_digest
            );
        }
        assert_eq!(result.workflow.review_status, ReviewStatus::Executed);
        let provenance = &result.provenance.entries[0];
        assert_eq!(
            provenance.workflow_id.as_deref(),
            Some(result.workflow.id.to_string().as_str())
        );
        assert_eq!(provenance.details["crs"], "EPSG:4326");
        assert_eq!(provenance.details["coordinate_units"], "degrees");
        assert_eq!(provenance.details["units"], "persons/km²");
        assert_eq!(provenance.details["area_method"], "ellipsoidal_wgs84");
        assert_eq!(
            provenance.details["source"]["uri"],
            result.dataset.uri.as_str()
        );
        assert!(provenance.details["source"]["retrieved_at"].is_null());
        assert_eq!(provenance.details["source"]["checksum_status"], "verified");
        assert_eq!(
            provenance.details["source"]["source_version"],
            "nagoya-2020-census-final-n03-v2"
        );
        assert_eq!(
            provenance.details["source"]["expected_checksum"],
            "sha256:d0f8958813fe28e9428169ca7c638a0ea3b3ed7ae526750156d3f94e1308d30e"
        );
        assert_eq!(
            provenance.details["source"]["observed_checksum"],
            provenance.details["source"]["expected_checksum"]
        );
        assert_eq!(provenance.details["checksum_status"], "verified");
        assert_eq!(
            provenance.details["checksum"],
            "sha256:d0f8958813fe28e9428169ca7c638a0ea3b3ed7ae526750156d3f94e1308d30e"
        );
        assert_eq!(
            result.verification.source.dataset_id.as_deref(),
            Some(result.dataset.id.as_str())
        );
        assert_eq!(
            result.verification.source.license.as_deref(),
            Some(result.dataset.license.as_str())
        );
        assert!(result.verification.source.retrieved_at.is_none());
        assert_eq!(
            result.verification.source.checksum_status,
            ChecksumVerification::Verified
        );
        assert!(result.summary["provenance"]["entries"].is_array());
        assert_eq!(provenance.details["verification_passed"], true);
    }

    #[test]
    fn cli_and_ai_origins_share_the_single_north_star_dispatcher() {
        let config = PlannerConfig::default();
        let ai = run_ask_pipeline_with_config_and_origin(
            "名古屋市の人口密度を表示",
            &config,
            CommandOrigin::Ai,
        )
        .expect("AI pipeline");
        let cli = run_ask_pipeline_with_config_and_origin(
            "名古屋市の人口密度を表示",
            &config,
            CommandOrigin::Cli,
        )
        .expect("CLI pipeline");

        assert_eq!(ai.command.origin, CommandOrigin::Ai);
        assert_eq!(cli.command.origin, CommandOrigin::Cli);
        assert_eq!(
            ai.execution_receipt.workflow_digest,
            cli.execution_receipt.workflow_digest
        );
        assert_eq!(
            ai.execution_receipt.result_digest,
            cli.execution_receipt.result_digest
        );
        assert_eq!(
            ai.execution_receipt.evidence["topological_order"],
            cli.execution_receipt.evidence["topological_order"]
        );
        assert_eq!(ai.summary["command"]["origin"], "ai");
        assert_eq!(cli.summary["command"]["origin"], "cli");
    }

    #[test]
    fn remote_cog_metadata_verifier_accepts_valid_info() {
        let info = CogInfo {
            path: Some("demo.tif".into()),
            width: 512,
            height: 512,
            band_count: 1,
            epsg: Some(4326),
            crs: "EPSG:4326".into(),
            geo_bounds: None,
            tiled: true,
            tile_width: Some(256),
            tile_height: Some(256),
            overview_count: 0,
            cloud_optimized: true,
            read_mode: Some("http_range".into()),
        };
        assert!(verify_remote_cog_metadata(&info).expect("verify"));
    }

    #[test]
    fn geoparquet_feature_verifier_accepts_nagoya_fixture() {
        let catalog = alpha_catalog();
        let record = catalog
            .require(genegis_catalog::NAGOYA_WARDS_GEOPARQUET_ID)
            .expect("record");
        if !std::path::Path::new(&record.uri).exists() {
            return;
        }
        let dataset = read_geoparquet_uri(&record.uri).expect("read geoparquet");
        assert!(verify_geoparquet_features(&dataset).expect("verify"));
    }
}
