use chrono::Utc;
use genegis_ai::{plan_with_config, PlanResult, DEFAULT_AGENT_PLAN_PATH};
use genegis_analysis::{
    build_ask_result, run_analysis_for_plan, verify_analysis_densities,
};
use genegis_catalog::{alpha_catalog, DatasetRecord};
use uuid::Uuid;

use crate::error::AgentError;
use crate::model::{AgentRole, AgentRun, AgentRunConfig, AgentStep, ToolCall};
use crate::tool_registry::{
    validate_executor_tool, validate_planner_tools, validate_verifier_tool,
};

/// Multi-agent orchestrator (Phase 6 — plan → catalog → execute → verify with retries).
#[derive(Debug, Clone, Default)]
pub struct AgentOrchestrator {
    config: AgentRunConfig,
}

impl AgentOrchestrator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_config(mut self, config: AgentRunConfig) -> Self {
        self.config = config;
        self
    }

    pub fn run(&self, prompt: &str) -> Result<AgentRun, AgentError> {
        let plan = plan_with_config(prompt, &self.config.planner)?;
        if plan.mode.starts_with("llm_") {
            validate_planner_tools(&plan.tool_calls)?;
        }

        let steps = vec![record_plan_step(&plan)?];
        if self.config.plan_only {
            plan.save_to_path(DEFAULT_AGENT_PLAN_PATH)
                .map_err(|err| AgentError::Message(err.to_string()))?;
            return Ok(build_plan_only_run(prompt, plan, steps));
        }

        self.execute_after_plan(prompt, plan, steps)
    }

    /// Human gate: execute a previously approved plan without re-planning.
    pub fn execute_plan(&self, plan: PlanResult) -> Result<AgentRun, AgentError> {
        if plan.mode.starts_with("llm_") {
            validate_planner_tools(&plan.tool_calls)?;
        }
        let prompt = plan.intent.raw_prompt.clone();
        let steps = vec![record_plan_step(&plan)?];
        self.execute_after_plan(&prompt, plan, steps)
    }

    fn execute_after_plan(
        &self,
        prompt: &str,
        plan: PlanResult,
        mut steps: Vec<AgentStep>,
    ) -> Result<AgentRun, AgentError> {
        let started_at = Utc::now();
        let run_id = Uuid::new_v4();

        let dataset = resolve_dataset(&plan)?;
        steps.push(record_catalog_step(&plan, &dataset)?);

        let max_attempts = self.config.verify_retries.max(1);
        let mut verify_attempts = 0_u32;
        let mut duckdb_verified = false;
        let mut analysis = None;

        for attempt in 1..=max_attempts {
            if attempt > 1 {
                steps.push(record_retry_step(attempt)?);
            }

            let (next_analysis, next_dataset) = run_analysis_for_plan(&plan)?;
            if attempt == 1 {
                debug_assert_eq!(next_dataset.id, dataset.id);
            }
            steps.push(record_execute_step(&plan, next_analysis.features.len(), attempt)?);

            duckdb_verified = verify_analysis_densities(&next_analysis)?;
            verify_attempts = attempt;
            steps.push(record_verify_step(duckdb_verified, attempt)?);

            analysis = Some(next_analysis);
            if duckdb_verified {
                break;
            }
        }

        let analysis = analysis.expect("at least one execute attempt");
        if !duckdb_verified {
            return Ok(AgentRun {
                id: run_id,
                prompt: prompt.to_string(),
                plan_only: false,
                planner_mode: plan.mode.clone(),
                workflow_id: Some(plan.resolved.workflow_id.as_str().to_string()),
                confidence: Some(plan.resolved.confidence),
                verification_passed: false,
                verify_attempts,
                collab_comment_ids: Vec::new(),
                started_at,
                finished_at: Utc::now(),
                steps,
                summary: serde_json::json!({
                    "error": "DuckDB density verification failed",
                    "verify_attempts": verify_attempts,
                    "workflow_id": plan.resolved.workflow_id.as_str(),
                }),
            });
        }

        let ask = build_ask_result(prompt, &plan, analysis, dataset, duckdb_verified)?;

        Ok(AgentRun {
            id: run_id,
            prompt: prompt.to_string(),
            plan_only: false,
            planner_mode: plan.mode.clone(),
            workflow_id: Some(plan.resolved.workflow_id.as_str().to_string()),
            confidence: Some(plan.resolved.confidence),
            verification_passed: true,
            verify_attempts,
            collab_comment_ids: Vec::new(),
            started_at,
            finished_at: Utc::now(),
            steps,
            summary: ask.summary,
        })
    }
}

fn build_plan_only_run(prompt: &str, plan: PlanResult, steps: Vec<AgentStep>) -> AgentRun {
    AgentRun {
        id: Uuid::new_v4(),
        prompt: prompt.to_string(),
        plan_only: true,
        planner_mode: plan.mode.clone(),
        workflow_id: Some(plan.resolved.workflow_id.as_str().to_string()),
        confidence: Some(plan.resolved.confidence),
        verification_passed: false,
        verify_attempts: 0,
        collab_comment_ids: Vec::new(),
        started_at: Utc::now(),
        finished_at: Utc::now(),
        steps,
        summary: plan_summary(&plan),
    }
}

fn resolve_dataset(plan: &PlanResult) -> Result<DatasetRecord, AgentError> {
    let catalog = alpha_catalog();
    catalog
        .require(&plan.resolved.dataset_id)
        .map(|record| record.clone())
        .map_err(|err| AgentError::Message(err.to_string()))
}

fn record_plan_step(plan: &PlanResult) -> Result<AgentStep, AgentError> {
    let mut step = AgentStep::new(
        AgentRole::Planner,
        if plan.mode.starts_with("llm_") {
            "llm_planner"
        } else {
            "rule_planner"
        },
        "Resolve intent to workflow IR",
    );

    if plan.tool_calls.is_empty() {
        step = step.with_tool_call(ToolCall {
            tool: "plan_workflow".into(),
            input: serde_json::json!({ "prompt": plan.intent.raw_prompt }),
            output: serde_json::json!({
                "workflow_id": plan.resolved.workflow_id.as_str(),
                "dataset_id": plan.resolved.dataset_id,
                "confidence": plan.resolved.confidence,
                "mode": plan.mode,
            }),
            ok: true,
        });
    } else {
        for call in &plan.tool_calls {
            step = step.with_tool_call(ToolCall {
                tool: call.tool.clone(),
                input: call.input.clone(),
                output: call.output.clone(),
                ok: call.ok,
            });
        }
    }

    Ok(step.finish())
}

fn record_catalog_step(plan: &PlanResult, dataset: &DatasetRecord) -> Result<AgentStep, AgentError> {
    validate_executor_tool("catalog_resolve")?;
    Ok(
        AgentStep::new(
            AgentRole::Executor,
            "catalog_agent",
            "Resolve dataset record from alpha catalog",
        )
        .with_tool_call(ToolCall {
            tool: "catalog_resolve".into(),
            input: serde_json::json!({
                "workflow_id": plan.resolved.workflow_id.as_str(),
                "dataset_id": plan.resolved.dataset_id,
            }),
            output: serde_json::json!({
                "id": dataset.id,
                "format": format!("{:?}", dataset.format),
                "crs": dataset.crs,
                "tags": dataset.tags,
            }),
            ok: true,
        })
        .finish(),
    )
}

fn record_retry_step(attempt: u32) -> Result<AgentStep, AgentError> {
    validate_executor_tool("verify_retry")?;
    Ok(
        AgentStep::new(
            AgentRole::Executor,
            "retry_coordinator",
            format!("Retry execute→verify after DuckDB failure (attempt {attempt})"),
        )
        .with_tool_call(ToolCall {
            tool: "verify_retry".into(),
            input: serde_json::json!({ "attempt": attempt }),
            output: serde_json::json!({ "scheduled": true }),
            ok: true,
        })
        .finish(),
    )
}

fn record_execute_step(
    plan: &PlanResult,
    ward_count: usize,
    attempt: u32,
) -> Result<AgentStep, AgentError> {
    validate_executor_tool("run_nagoya_density")?;
    Ok(
        AgentStep::new(
            AgentRole::Executor,
            "nagoya_executor",
            format!("Run Nagoya density analysis (attempt {attempt})"),
        )
        .with_tool_call(ToolCall {
            tool: "run_nagoya_density".into(),
            input: serde_json::json!({
                "workflow_id": plan.resolved.workflow_id.as_str(),
                "dataset_id": plan.resolved.dataset_id,
                "attempt": attempt,
            }),
            output: serde_json::json!({ "ward_count": ward_count }),
            ok: true,
        })
        .finish(),
    )
}

fn record_verify_step(passed: bool, attempt: u32) -> Result<AgentStep, AgentError> {
    validate_verifier_tool("duckdb_verify")?;
    Ok(
        AgentStep::new(
            AgentRole::Verifier,
            "duckdb_verifier",
            format!("Cross-check ward densities with DuckDB (attempt {attempt})"),
        )
        .with_tool_call(ToolCall {
            tool: "duckdb_verify".into(),
            input: serde_json::json!({ "workflow": "nagoya-density", "attempt": attempt }),
            output: serde_json::json!({ "passed": passed, "attempt": attempt }),
            ok: passed,
        })
        .finish(),
    )
}

fn plan_summary(plan: &PlanResult) -> serde_json::Value {
    serde_json::json!({
        "goal": plan.workflow.goal,
        "workflow_id": plan.resolved.workflow_id.as_str(),
        "dataset_id": plan.resolved.dataset_id,
        "confidence": plan.resolved.confidence,
        "workflow_steps": plan.workflow.steps.len(),
        "planner_mode": plan.mode,
        "pending_plan_path": DEFAULT_AGENT_PLAN_PATH,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use genegis_ai::plan_from_prompt;

    #[test]
    fn runs_north_star_agent_trace() {
        let run = AgentOrchestrator::new()
            .with_config(AgentRunConfig::rule_based_offline())
            .run("名古屋市の人口密度を表示")
            .expect("run");

        assert!(run.verification_passed);
        assert_eq!(run.verify_attempts, 1);
        assert_eq!(run.steps.len(), 4);
        assert_eq!(run.steps[0].role, AgentRole::Planner);
        assert_eq!(run.steps[0].tool_calls.len(), 3);
        assert_eq!(run.steps[0].tool_calls[0].tool, "parse_intent");
        assert_eq!(run.steps[1].agent, "catalog_agent");
        assert_eq!(run.steps[3].role, AgentRole::Verifier);
    }

    #[test]
    fn plan_only_stops_after_planner() {
        let run = AgentOrchestrator::new()
            .with_config(AgentRunConfig::rule_based_offline().plan_only())
            .run("名古屋市の人口密度を表示")
            .expect("run");

        assert!(!run.verification_passed);
        assert_eq!(run.steps.len(), 1);
        assert_eq!(run.steps[0].role, AgentRole::Planner);
    }

    #[test]
    fn human_gate_execute_saved_plan() {
        let plan = plan_from_prompt("名古屋市の人口密度を表示").expect("plan");
        let run = AgentOrchestrator::new()
            .with_config(AgentRunConfig::rule_based_offline())
            .execute_plan(plan)
            .expect("execute");

        assert!(run.verification_passed);
        assert_eq!(run.steps.len(), 4);
    }
}
