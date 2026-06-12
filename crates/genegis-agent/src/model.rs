use std::path::Path;

use chrono::{DateTime, Utc};
use genegis_ai::PlannerConfig;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AgentError;

/// Default path for the latest agent run trace JSON.
pub const DEFAULT_AGENT_RUN_PATH: &str = ".genegis/agent-run.json";

/// Agent role in the orchestration graph (Phase 6 alpha).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Planner,
    Executor,
    Verifier,
}

/// Single tool invocation recorded in an agent step trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub tool: String,
    pub input: serde_json::Value,
    pub output: serde_json::Value,
    pub ok: bool,
}

/// One agent step in a run (planner / executor / verifier).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStep {
    pub id: Uuid,
    pub role: AgentRole,
    pub agent: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub tool_calls: Vec<ToolCall>,
    pub detail: String,
}

impl AgentStep {
    pub fn new(role: AgentRole, agent: impl Into<String>, detail: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            role,
            agent: agent.into(),
            started_at: now,
            finished_at: now,
            tool_calls: Vec::new(),
            detail: detail.into(),
        }
    }

    pub fn with_tool_call(mut self, call: ToolCall) -> Self {
        self.tool_calls.push(call);
        self.finished_at = Utc::now();
        self
    }

    pub fn finish(mut self) -> Self {
        self.finished_at = Utc::now();
        self
    }
}

/// End-to-end agent orchestration run with trace metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRun {
    pub id: Uuid,
    pub prompt: String,
    pub plan_only: bool,
    pub planner_mode: String,
    pub workflow_id: Option<String>,
    pub confidence: Option<f32>,
    pub verification_passed: bool,
    #[serde(default)]
    pub verify_attempts: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collab_comment_ids: Vec<Uuid>,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub steps: Vec<AgentStep>,
    pub summary: serde_json::Value,
}

impl AgentRun {
    pub fn trace_json(&self) -> Result<String, AgentError> {
        serde_json::to_string_pretty(self).map_err(|err| AgentError::Json(err.to_string()))
    }

    pub fn save_to_path(&self, path: impl AsRef<Path>) -> Result<(), AgentError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| AgentError::Json(err.to_string()))?;
        }
        std::fs::write(path, self.trace_json()?).map_err(|err| AgentError::Json(err.to_string()))
    }

    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, AgentError> {
        let json = std::fs::read_to_string(path.as_ref())
            .map_err(|err| AgentError::Json(err.to_string()))?;
        serde_json::from_str(&json).map_err(|err| AgentError::Json(err.to_string()))
    }
}

/// Configuration for an agent orchestration run.
#[derive(Debug, Clone)]
pub struct AgentRunConfig {
    pub planner: PlannerConfig,
    pub plan_only: bool,
    /// Maximum execute→verify attempts (includes the first try). Default: 2.
    pub verify_retries: u32,
    /// When true, callers may attach collab comments on verification failure.
    pub link_collab_on_failure: bool,
}

impl AgentRunConfig {
    pub fn rule_based_offline() -> Self {
        Self {
            planner: PlannerConfig {
                backend: genegis_ai::PlannerBackend::RuleBased,
                fallback_to_rules: true,
                ..PlannerConfig::default()
            },
            plan_only: false,
            verify_retries: 2,
            link_collab_on_failure: false,
        }
    }

    pub fn plan_only(mut self) -> Self {
        self.plan_only = true;
        self
    }

    pub fn with_verify_retries(mut self, retries: u32) -> Self {
        self.verify_retries = retries.max(1);
        self
    }

    pub fn with_link_collab_on_failure(mut self, enabled: bool) -> Self {
        self.link_collab_on_failure = enabled;
        self
    }

    pub fn with_planner(mut self, planner: PlannerConfig) -> Self {
        self.planner = planner;
        self
    }
}

impl Default for AgentRunConfig {
    fn default() -> Self {
        Self::rule_based_offline()
    }
}
