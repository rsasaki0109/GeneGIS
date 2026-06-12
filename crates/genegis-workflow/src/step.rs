use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkflowStepId(pub Uuid);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    pub id: WorkflowStepId,
    pub operation: String,
    pub parameters: serde_json::Value,
    pub expected_schema: Option<serde_json::Value>,
    pub validation: Option<serde_json::Value>,
    pub provenance: Option<String>,
}

impl WorkflowStep {
    pub fn new(operation: impl Into<String>, parameters: serde_json::Value) -> Self {
        Self {
            id: WorkflowStepId(Uuid::new_v4()),
            operation: operation.into(),
            parameters,
            expected_schema: None,
            validation: None,
            provenance: None,
        }
    }
}
