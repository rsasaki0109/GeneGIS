//! AI-native intent parsing and workflow planning (Phase 1: rule-based MVP).

pub mod backend;
pub mod error;
pub mod intent;
pub mod llm;
pub mod planner;
pub mod resolver;
pub mod tool_call;

pub use backend::{PlannerBackend, PlannerConfig};
pub use error::AiError;
pub use intent::{IntentSignals, ParsedIntent};
pub use planner::{PlanMode, PlanResult, plan_from_prompt, plan_with_config};
pub use resolver::{bind_catalog_dataset, ResolvedWorkflow, WorkflowId, resolve_workflow};
pub use tool_call::{PlannerToolCall, llm_tool_calls, rule_based_tool_calls};
