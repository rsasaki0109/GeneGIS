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
