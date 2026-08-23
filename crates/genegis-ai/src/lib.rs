//! AI-native intent parsing and workflow planning (Phase 1: rule-based MVP).

pub mod backend;
pub mod error;
pub mod intent;
pub mod llm;
pub mod planner;
pub mod resolver;
pub mod stac_url;
pub mod tool_call;

pub use backend::{PlannerBackend, PlannerConfig};
pub use error::AiError;
pub use intent::{IntentSignals, ParsedIntent};
pub use planner::{
    plan_from_prompt, plan_with_config, PlanMode, PlanResult, DEFAULT_AGENT_PLAN_PATH,
};
pub use resolver::{bind_catalog_dataset, resolve_workflow, ResolvedWorkflow, WorkflowId};
pub use stac_url::extract_catalog_url;
pub use tool_call::{llm_tool_calls, rule_based_tool_calls, PlannerToolCall};
