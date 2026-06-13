//! Shared MVP ask → analyze → verify → export pipeline.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use genegis_ai::{PlanResult, PlannerConfig, WorkflowId, plan_with_config};
use genegis_catalog::{alpha_catalog, DatasetRecord};
use genegis_query::verify_nagoya_densities;
use genegis_raster::CogInfo;

use crate::error::AnalysisError;
use crate::export::{export_html_map, export_png_map};
use crate::nagoya::run_nagoya_population_density_for_dataset;
use crate::result::{AnalysisResult, VerificationReport};

/// Executed workflow payload for agent orchestration (Phase 8 alpha).
#[derive(Debug, Clone)]
pub enum ExecutedWorkflow {
    NagoyaDensity(AnalysisResult),
    CogMetadata(CogInfo),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AskPipelineResult {
    pub prompt: String,
    pub workflow_id: String,
    pub confidence: f32,
    pub ambiguities: Vec<String>,
    pub workflow_steps: usize,
    pub verification: VerificationReport,
    pub summary: serde_json::Value,
    pub html: String,
    #[serde(skip)]
    pub png: Vec<u8>,
    pub png_base64: String,
    pub duckdb_verified: bool,
    pub dataset: DatasetRecord,
    pub stac_item: genegis_catalog::StacItem,
}

pub fn run_ask_pipeline(prompt: &str) -> Result<AskPipelineResult, AnalysisError> {
    run_ask_pipeline_with_config(prompt, &PlannerConfig::default())
}

pub fn run_ask_pipeline_with_config(
    prompt: &str,
    config: &PlannerConfig,
) -> Result<AskPipelineResult, AnalysisError> {
    let plan =
        plan_with_config(prompt, config).map_err(|e| AnalysisError::Message(e.to_string()))?;
    execute_from_plan(prompt, &plan)
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
    }
}

pub fn execute_workflow_for_plan(
    plan: &PlanResult,
) -> Result<(ExecutedWorkflow, DatasetRecord), AnalysisError> {
    let catalog = alpha_catalog();
    let dataset_record = catalog
        .require(&plan.resolved.dataset_id)
        .map_err(|e| AnalysisError::Message(e.to_string()))?
        .clone();

    match plan.resolved.workflow_id {
        WorkflowId::NagoyaDensity => {
            let analysis = run_nagoya_population_density_for_dataset(&plan.resolved.dataset_id)?;
            Ok((ExecutedWorkflow::NagoyaDensity(analysis), dataset_record))
        }
        WorkflowId::RemoteCogDemo | WorkflowId::LocalCogDemo => {
            let info = genegis_raster::read_cog_uri(&dataset_record.uri)
                .map_err(|err| AnalysisError::Message(err.to_string()))?;
            Ok((ExecutedWorkflow::CogMetadata(info), dataset_record))
        }
    }
}

pub fn verify_executed_workflow(result: &ExecutedWorkflow) -> Result<bool, AnalysisError> {
    match result {
        ExecutedWorkflow::NagoyaDensity(analysis) => verify_analysis_densities(analysis),
        ExecutedWorkflow::CogMetadata(info) => verify_remote_cog_metadata(info),
    }
}

pub fn verify_analysis_densities(analysis: &AnalysisResult) -> Result<bool, AnalysisError> {
    let rows: Vec<(String, u64, f64, f64)> = analysis
        .features
        .iter()
        .map(|f| {
            (
                f.ward_name.clone(),
                f.population,
                f.area_km2,
                f.density_per_km2,
            )
        })
        .collect();

    verify_nagoya_densities(&rows).map_err(|e| AnalysisError::Message(e.to_string()))
}

pub fn verify_remote_cog_metadata(info: &CogInfo) -> Result<bool, AnalysisError> {
    Ok(info.width > 0 && info.height > 0 && info.band_count >= 1 && !info.crs.is_empty())
}

pub fn build_ask_result(
    prompt: &str,
    plan: &PlanResult,
    analysis: AnalysisResult,
    dataset: DatasetRecord,
    duckdb_verified: bool,
) -> Result<AskPipelineResult, AnalysisError> {
    let summary = build_summary(&analysis, &dataset);
    let html = export_html_map(&analysis, "名古屋市 人口密度");
    let png = export_png_map(&analysis, "名古屋市 人口密度")?;
    let png_base64 = STANDARD.encode(&png);

    Ok(AskPipelineResult {
        prompt: prompt.to_string(),
        workflow_id: plan.resolved.workflow_id.as_str().to_string(),
        confidence: plan.resolved.confidence,
        ambiguities: plan.resolved.ambiguities.clone(),
        workflow_steps: plan.workflow.steps.len(),
        verification: analysis.verification.clone(),
        summary,
        html,
        png,
        png_base64,
        duckdb_verified,
        dataset: dataset.clone(),
        stac_item: dataset.to_stac_item(),
    })
}

pub fn execute_from_plan(
    prompt: &str,
    plan: &PlanResult,
) -> Result<AskPipelineResult, AnalysisError> {
    let (executed, dataset) = execute_workflow_for_plan(plan)?;
    let duckdb_verified = verify_executed_workflow(&executed)?;

    if !duckdb_verified {
        return Err(AnalysisError::Message(
            "workflow verification failed".into(),
        ));
    }

    match executed {
        ExecutedWorkflow::NagoyaDensity(analysis) => {
            build_ask_result(prompt, plan, analysis, dataset, duckdb_verified)
        }
        ExecutedWorkflow::CogMetadata(info) => {
            build_remote_cog_ask_result(prompt, plan, info, dataset, duckdb_verified)
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
    let summary = serde_json::json!({
        "goal": plan.resolved.goal,
        "dataset": dataset.summary_json(),
        "cog": info.summary_json(),
        "verification_passed": verified,
        "read_mode": info.read_mode,
    });

    Ok(AskPipelineResult {
        prompt: prompt.to_string(),
        workflow_id: plan.resolved.workflow_id.as_str().to_string(),
        confidence: plan.resolved.confidence,
        ambiguities: plan.resolved.ambiguities.clone(),
        workflow_steps: plan.workflow.steps.len(),
        verification: VerificationReport {
            crs: info.crs.clone(),
            area_method: "n/a".into(),
            density_unit: "n/a".into(),
            checks: vec![],
        },
        summary,
        html: String::new(),
        png: Vec::new(),
        png_base64: String::new(),
        duckdb_verified: verified,
        dataset: dataset.clone(),
        stac_item: dataset.to_stac_item(),
    })
}

fn build_summary(result: &AnalysisResult, dataset: &DatasetRecord) -> serde_json::Value {
    serde_json::json!({
        "goal": result.workflow.goal,
        "dataset": dataset.summary_json(),
        "ward_count": result.features.len(),
        "density_unit": result.verification.density_unit,
        "crs": result.verification.crs,
        "verification_passed": result.verification.checks.iter().all(|c| c.passed),
        "top_density_ward": result.features.iter()
            .max_by(|a, b| a.density_per_km2.partial_cmp(&b.density_per_km2).unwrap())
            .map(|f| serde_json::json!({
                "ward_name": f.ward_name,
                "density_per_km2": f.density_per_km2,
            })),
    })
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
}
