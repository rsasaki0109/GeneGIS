//! GeneGIS multi-agent orchestration — plan, execute, verify with trace export.

pub mod error;
pub mod model;
pub mod orchestrator;
pub mod remote;
pub mod tool_registry;

pub use error::AgentError;
pub use model::{
    AgentRole, AgentRun, AgentRunConfig, AgentRunSummary, AgentStep, ToolCall,
    DEFAULT_AGENT_RUN_PATH, DEFAULT_AGENT_RUNS_DIR,
};
pub use orchestrator::AgentOrchestrator;
pub use remote::{
    get_agent_run, list_agent_runs, pull_latest_agent_run, push_agent_run, DEFAULT_SERVER_URL,
};
pub use tool_registry::{validate_executor_tool, validate_planner_tools, validate_verifier_tool};
