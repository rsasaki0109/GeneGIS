use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{ReviewStatus, WorkflowStep};

/// GeoWorkflow IR — goal, assumptions, steps, outputs, citations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoWorkflow {
    pub id: Uuid,
    pub goal: String,
    pub assumptions: Vec<String>,
    pub inputs: Vec<serde_json::Value>,
    pub steps: Vec<WorkflowStep>,
    pub outputs: Vec<serde_json::Value>,
    pub citations: Vec<Citation>,
    pub review_status: ReviewStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Citation {
    pub title: String,
    pub url: Option<String>,
    pub license: Option<String>,
    pub retrieved_at: Option<String>,
}

impl GeoWorkflow {
    pub fn new(goal: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            goal: goal.into(),
            assumptions: Vec::new(),
            inputs: Vec::new(),
            steps: Vec::new(),
            outputs: Vec::new(),
            citations: Vec::new(),
            review_status: ReviewStatus::Draft,
        }
    }

    pub fn push_step(&mut self, step: WorkflowStep) {
        self.steps.push(step);
    }
}

/// MVP north-star workflow: Nagoya population density.
pub fn nagoya_population_density_template() -> GeoWorkflow {
    let mut workflow = GeoWorkflow::new("名古屋市の人口密度を表示");
    workflow.assumptions.push("行政区域は ward または cho 粒度".into());
    workflow.steps = vec![
        WorkflowStep::new("ResolvePlace", serde_json::json!({ "name": "名古屋市" })),
        WorkflowStep::new(
            "FindDataset",
            serde_json::json!({ "type": "admin_boundary", "area": "Nagoya" }),
        ),
        WorkflowStep::new(
            "FindDataset",
            serde_json::json!({ "type": "population", "area": "Nagoya" }),
        ),
        WorkflowStep::new("LoadBoundary", serde_json::json!({})),
        WorkflowStep::new("LoadPopulation", serde_json::json!({})),
        WorkflowStep::new("NormalizeSchema", serde_json::json!({})),
        WorkflowStep::new(
            "ReprojectForAreaCalculation",
            serde_json::json!({ "method": "equal_area" }),
        ),
        WorkflowStep::new("CalculateAreaKm2", serde_json::json!({})),
        WorkflowStep::new("JoinPopulationToGeometry", serde_json::json!({})),
        WorkflowStep::new(
            "CalculateDensity",
            serde_json::json!({ "formula": "population / area_km2" }),
        ),
        WorkflowStep::new("GenerateChoropleth", serde_json::json!({})),
        WorkflowStep::new("VerifyUnits", serde_json::json!({})),
        WorkflowStep::new("RenderMap", serde_json::json!({})),
        WorkflowStep::new("AttachSources", serde_json::json!({})),
    ];
    workflow
}

/// Remote COG / GeoTIFF metadata probe workflow (catalog + HTTP range-read demo).
pub fn remote_cog_metadata_template() -> GeoWorkflow {
    let mut workflow = GeoWorkflow::new("リモートCOGデモのメタデータを表示");
    workflow.assumptions.push("Asset is fetched over HTTP range-read when remote".into());
    workflow.steps = vec![
        WorkflowStep::new(
            "FindDataset",
            serde_json::json!({ "tags": ["cog", "remote", "demo"] }),
        ),
        WorkflowStep::new(
            "ProbeRasterMetadata",
            serde_json::json!({ "read_mode": "http_range" }),
        ),
        WorkflowStep::new("SummarizeCogInfo", serde_json::json!({})),
        WorkflowStep::new("AttachSources", serde_json::json!({})),
    ];
    workflow
}

/// Local bundled COG metadata probe workflow (offline fixture).
pub fn local_cog_metadata_template() -> GeoWorkflow {
    let mut workflow = GeoWorkflow::new("ローカルCOGデモのメタデータを表示");
    workflow.assumptions.push("Asset is read from bundled smoke GeoTIFF fixture".into());
    workflow.steps = vec![
        WorkflowStep::new(
            "FindDataset",
            serde_json::json!({ "tags": ["cog", "local", "demo"] }),
        ),
        WorkflowStep::new(
            "ProbeRasterMetadata",
            serde_json::json!({ "read_mode": "local" }),
        ),
        WorkflowStep::new("SummarizeCogInfo", serde_json::json!({})),
        WorkflowStep::new("AttachSources", serde_json::json!({})),
    ];
    workflow
}

/// Nagoya GeoParquet read + feature-count verification workflow (Phase 9 alpha).
pub fn nagoya_geoparquet_template() -> GeoWorkflow {
    let mut workflow = GeoWorkflow::new("名古屋 wards GeoParquet を検証");
    workflow.assumptions.push("Bundled GeoParquet fixture with 16 Nagoya wards".into());
    workflow.steps = vec![
        WorkflowStep::new(
            "FindDataset",
            serde_json::json!({ "tags": ["nagoya", "geoparquet", "demo"] }),
        ),
        WorkflowStep::new("LoadGeoParquet", serde_json::json!({ "format": "geoparquet" })),
        WorkflowStep::new(
            "VerifyFeatureCount",
            serde_json::json!({ "expected": 16, "field": "ward_name" }),
        ),
        WorkflowStep::new("AttachSources", serde_json::json!({})),
    ];
    workflow
}

/// Nagoya GeoParquet population density choropleth workflow (Phase 9 beta).
pub fn nagoya_geoparquet_density_template() -> GeoWorkflow {
    let mut workflow = GeoWorkflow::new("名古屋 GeoParquet 人口密度を表示");
    workflow.assumptions.push("Density computed from bundled GeoParquet wards fixture".into());
    workflow.steps = vec![
        WorkflowStep::new(
            "FindDataset",
            serde_json::json!({ "tags": ["nagoya", "geoparquet", "density"] }),
        ),
        WorkflowStep::new("LoadGeoParquet", serde_json::json!({ "format": "geoparquet" })),
        WorkflowStep::new("CalculateAreaKm2", serde_json::json!({})),
        WorkflowStep::new(
            "CalculateDensity",
            serde_json::json!({ "formula": "population / area_km2" }),
        ),
        WorkflowStep::new("GenerateChoropleth", serde_json::json!({})),
        WorkflowStep::new("VerifyUnits", serde_json::json!({})),
        WorkflowStep::new("RenderMap", serde_json::json!({})),
        WorkflowStep::new("AttachSources", serde_json::json!({})),
    ];
    workflow
}

/// External STAC collection fetch workflow (Phase 9 beta).
pub fn external_stac_fetch_template() -> GeoWorkflow {
    let mut workflow = GeoWorkflow::new("外部 STAC collection を fetch");
    workflow.assumptions.push("Collection URL is extracted from the user prompt".into());
    workflow.steps = vec![
        WorkflowStep::new(
            "FindDataset",
            serde_json::json!({ "tags": ["stac", "external", "demo"] }),
        ),
        WorkflowStep::new(
            "FetchStacCollection",
            serde_json::json!({ "tool": "stac_fetch" }),
        ),
        WorkflowStep::new("SummarizeCollection", serde_json::json!({})),
        WorkflowStep::new("AttachSources", serde_json::json!({})),
    ];
    workflow
}
