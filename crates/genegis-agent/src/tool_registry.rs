use genegis_ai::PlannerToolCall;

use crate::error::AgentError;

const PLANNER_TOOLS: &[&str] = &[
    "parse_intent",
    "resolve_workflow",
    "stac_browse",
    "stac_bind",
    "catalog_bind",
    "llm_plan_workflow",
    "plan_workflow",
];

const EXECUTOR_TOOLS: &[&str] = &[
    "catalog_resolve",
    "run_nagoya_density",
    "run_remote_cog_metadata",
    "run_local_cog_metadata",
    "verify_retry",
];

const VERIFIER_TOOLS: &[&str] = &["duckdb_verify", "cog_metadata_verify"];

/// Validate planner tool calls against the Phase 6 allowlist (ADR 0003).
pub fn validate_planner_tools(calls: &[PlannerToolCall]) -> Result<(), AgentError> {
    for call in calls {
        if !PLANNER_TOOLS.contains(&call.tool.as_str()) {
            return Err(AgentError::Message(format!(
                "planner tool {:?} is not allowlisted",
                call.tool
            )));
        }
        if !call.ok {
            return Err(AgentError::Message(format!(
                "planner tool {:?} reported ok=false",
                call.tool
            )));
        }
    }
    Ok(())
}

pub fn validate_executor_tool(tool: &str) -> Result<(), AgentError> {
    if EXECUTOR_TOOLS.contains(&tool) {
        Ok(())
    } else {
        Err(AgentError::Message(format!(
            "executor tool {tool:?} is not allowlisted"
        )))
    }
}

pub fn validate_verifier_tool(tool: &str) -> Result<(), AgentError> {
    if VERIFIER_TOOLS.contains(&tool) {
        Ok(())
    } else {
        Err(AgentError::Message(format!(
            "verifier tool {tool:?} is not allowlisted"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use genegis_ai::PlannerToolCall;

    #[test]
    fn rejects_unknown_planner_tool() {
        let calls = vec![PlannerToolCall::new(
            "shell_exec",
            serde_json::json!({}),
            serde_json::json!({}),
            true,
        )];
        assert!(validate_planner_tools(&calls).is_err());
    }
}
