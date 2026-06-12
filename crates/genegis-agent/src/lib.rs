//! GeneGIS multi-agent orchestration — plan, execute, verify with trace export.

pub mod error;
pub mod model;
pub mod orchestrator;
pub mod remote;

pub use error::AgentError;
pub use model::{
    AgentRole, AgentRun, AgentRunConfig, AgentStep, ToolCall, DEFAULT_AGENT_RUN_PATH,
};
pub use orchestrator::AgentOrchestrator;
pub use remote::{pull_latest_agent_run, push_agent_run, DEFAULT_SERVER_URL};
