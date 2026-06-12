use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OperationId(pub Uuid);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationDescriptor {
    pub id: OperationId,
    pub name: String,
    pub description_for_ai: String,
    pub input_schema: serde_json::Value,
    pub output_schema: serde_json::Value,
    pub deterministic: bool,
    pub crs_requirements: Option<String>,
}

impl OperationDescriptor {
    pub fn new(name: impl Into<String>, description_for_ai: impl Into<String>) -> Self {
        Self {
            id: OperationId(Uuid::new_v4()),
            name: name.into(),
            description_for_ai: description_for_ai.into(),
            input_schema: serde_json::json!({}),
            output_schema: serde_json::json!({}),
            deterministic: true,
            crs_requirements: None,
        }
    }
}
