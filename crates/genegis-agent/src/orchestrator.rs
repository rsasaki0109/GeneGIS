use chrono::Utc;
use genegis_ai::{plan_with_config, PlanResult};
use genegis_analysis::{
    build_ask_result, run_analysis_for_plan, verify_analysis_densities,
};
use genegis_catalog::{alpha_catalog, DatasetRecord};
use uuid::Uuid;

use crate::error::AgentError;
use crate::model::{AgentRole, AgentRun, AgentRunConfig, AgentStep, ToolCall};

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
        let started_at = Utc::now();
        let run_id = Uuid::new_v4();
        let mut steps = Vec::new();

        let plan = plan_with_config(prompt, &self.config.planner)?;
        steps.push(record_plan_step(&plan)?);

        if self.config.plan_only {
            return Ok(AgentRun {
                id: run_id,
                prompt: prompt.to_string(),
                plan_only: true,
                planner_mode: plan.mode.to_string(),
                workflow_id: Some(plan.resolved.workflow_id.as_str().to_string()),
                confidence: Some(plan.resolved.confidence),
                verification_passed: false,
                verify_attempts: 0,
                collab_comment_ids: Vec::new(),
                started_at,
                finished_at: Utc::now(),
                steps,
                summary: plan_summary(&plan),
            });
        }

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
                planner_mode: plan.mode.to_string(),
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
            planner_mode: plan.mode.to_string(),
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

fn resolve_dataset(plan: &PlanResult) -> Result<DatasetRecord, AgentError> {
    let catalog = alpha_catalog();
    catalog
        .require(&plan.resolved.dataset_id)
        .map(|record| record.clone())
        .map_err(|err| AgentError::Message(err.to_string()))
}

fn record_plan_step(plan: &PlanResult) -> Result<AgentStep, AgentError> {
    Ok(
        AgentStep::new(AgentRole::Planner, "rule_planner", "Resolve intent to workflow IR")
            .with_tool_call(ToolCall {
                tool: "plan_workflow".into(),
                input: serde_json::json!({ "prompt": plan.intent.raw_prompt }),
                output: serde_json::json!({
                    "workflow_id": plan.resolved.workflow_id.as_str(),
                    "dataset_id": plan.resolved.dataset_id,
                    "confidence": plan.resolved.confidence,
                    "mode": plan.mode,
                }),
                ok: true,
            })
            .finish(),
    )
}

fn record_catalog_step(plan: &PlanResult, dataset: &DatasetRecord) -> Result<AgentStep, AgentError> {
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(run.steps[1].agent, "catalog_agent");
        assert_eq!(run.steps[2].role, AgentRole::Executor);
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
}
