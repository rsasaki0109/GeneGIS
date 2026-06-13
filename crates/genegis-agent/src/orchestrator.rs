use chrono::Utc;
use genegis_ai::{plan_with_config, PlanResult, WorkflowId, DEFAULT_AGENT_PLAN_PATH};
use genegis_analysis::{
    build_ask_result, build_geoparquet_ask_result, build_remote_cog_ask_result,
    execute_workflow_for_plan, verify_executed_workflow, ExecutedWorkflow,
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
        let mut verified = false;
        let mut executed: Option<ExecutedWorkflow> = None;

        for attempt in 1..=max_attempts {
            if attempt > 1 {
                steps.push(record_retry_step(attempt)?);
            }

            let (next_executed, next_dataset) = execute_workflow_for_plan(&plan)
                .map_err(|err| AgentError::Message(err.to_string()))?;
            if attempt == 1 {
                debug_assert_eq!(next_dataset.id, dataset.id);
            }
            steps.push(record_execute_step(
                &plan,
                &next_executed,
                attempt,
            )?);

            verified = verify_executed_workflow(&next_executed)
                .map_err(|err| AgentError::Message(err.to_string()))?;
            verify_attempts = attempt;
            steps.push(record_verify_step(&plan, verified, attempt)?);

            executed = Some(next_executed);
            if verified {
                break;
            }
        }

        let executed = executed.expect("at least one execute attempt");
        if !verified {
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
                    "error": "workflow verification failed",
                    "verify_attempts": verify_attempts,
                    "workflow_id": plan.resolved.workflow_id.as_str(),
                }),
            });
        }

        let summary = match executed {
            ExecutedWorkflow::NagoyaDensity(analysis) => {
                let ask = build_ask_result(prompt, &plan, analysis, dataset, verified)
                    .map_err(|err| AgentError::Message(err.to_string()))?;
                ask.summary
            }
            ExecutedWorkflow::CogMetadata(info) => {
                let ask = build_remote_cog_ask_result(prompt, &plan, info, dataset, verified)
                    .map_err(|err| AgentError::Message(err.to_string()))?;
                ask.summary
            }
            ExecutedWorkflow::Geoparquet(vector) => {
                let ask = build_geoparquet_ask_result(prompt, &plan, vector, dataset, verified)
                    .map_err(|err| AgentError::Message(err.to_string()))?;
                ask.summary
            }
        };

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
            summary,
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
    executed: &ExecutedWorkflow,
    attempt: u32,
) -> Result<AgentStep, AgentError> {
    let (tool, agent, detail, output) = match (&plan.resolved.workflow_id, executed) {
        (WorkflowId::NagoyaDensity, ExecutedWorkflow::NagoyaDensity(analysis)) => (
            "run_nagoya_density",
            "nagoya_executor",
            format!("Run Nagoya density analysis (attempt {attempt})"),
            serde_json::json!({ "ward_count": analysis.features.len() }),
        ),
        (WorkflowId::RemoteCogDemo, ExecutedWorkflow::CogMetadata(info)) => (
            "run_remote_cog_metadata",
            "raster_executor",
            format!("Fetch remote COG metadata (attempt {attempt})"),
            info.summary_json(),
        ),
        (WorkflowId::LocalCogDemo, ExecutedWorkflow::CogMetadata(info)) => (
            "run_local_cog_metadata",
            "raster_executor",
            format!("Read local COG metadata (attempt {attempt})"),
            info.summary_json(),
        ),
        (WorkflowId::NagoyaGeoparquet, ExecutedWorkflow::Geoparquet(dataset)) => (
            "run_geoparquet_read",
            "vector_executor",
            format!("Read Nagoya GeoParquet fixture (attempt {attempt})"),
            genegis_vector::geoparquet_summary(dataset),
        ),
        _ => {
            return Err(AgentError::Message(format!(
                "workflow mismatch for {}",
                plan.resolved.workflow_id.as_str()
            )));
        }
    };

    validate_executor_tool(tool)?;
    Ok(
        AgentStep::new(AgentRole::Executor, agent, detail)
            .with_tool_call(ToolCall {
                tool: tool.into(),
                input: serde_json::json!({
                    "workflow_id": plan.resolved.workflow_id.as_str(),
                    "dataset_id": plan.resolved.dataset_id,
                    "attempt": attempt,
                }),
                output,
                ok: true,
            })
            .finish(),
    )
}

fn record_verify_step(
    plan: &PlanResult,
    passed: bool,
    attempt: u32,
) -> Result<AgentStep, AgentError> {
    let (tool, agent, detail) = match plan.resolved.workflow_id {
        WorkflowId::NagoyaDensity => (
            "duckdb_verify",
            "duckdb_verifier",
            format!("Cross-check ward densities with DuckDB (attempt {attempt})"),
        ),
        WorkflowId::RemoteCogDemo | WorkflowId::LocalCogDemo => (
            "cog_metadata_verify",
            "raster_verifier",
            format!("Validate COG metadata fields (attempt {attempt})"),
        ),
        WorkflowId::NagoyaGeoparquet => (
            "geoparquet_feature_verify",
            "vector_verifier",
            format!("Validate GeoParquet feature count (attempt {attempt})"),
        ),
    };

    validate_verifier_tool(tool)?;
    Ok(
        AgentStep::new(AgentRole::Verifier, agent, detail)
            .with_tool_call(ToolCall {
                tool: tool.into(),
                input: serde_json::json!({
                    "workflow": plan.resolved.workflow_id.as_str(),
                    "attempt": attempt,
                }),
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
        assert_eq!(run.steps[0].tool_calls.len(), 4);
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

    #[test]
    fn remote_cog_plan_only_resolves_workflow() {
        let run = AgentOrchestrator::new()
            .with_config(AgentRunConfig::rule_based_offline().plan_only())
            .run("リモートCOGデモのメタデータを表示")
            .expect("run");

        assert_eq!(run.workflow_id.as_deref(), Some("remote-cog-demo"));
        assert_eq!(run.steps.len(), 1);
        assert_eq!(run.steps[0].role, AgentRole::Planner);
    }

    #[test]
    fn local_cog_plan_only_resolves_workflow() {
        let run = AgentOrchestrator::new()
            .with_config(AgentRunConfig::rule_based_offline().plan_only())
            .run("ローカルCOGデモのメタデータを表示")
            .expect("run");

        assert_eq!(run.workflow_id.as_deref(), Some("local-cog-demo"));
        assert_eq!(run.steps.len(), 1);
    }

    #[test]
    fn runs_local_cog_agent_trace() {
        let run = AgentOrchestrator::new()
            .with_config(AgentRunConfig::rule_based_offline())
            .run("ローカルCOGデモのメタデータを表示")
            .expect("run");

        assert!(run.verification_passed);
        assert_eq!(run.workflow_id.as_deref(), Some("local-cog-demo"));
        assert_eq!(run.steps[2].tool_calls[0].tool, "run_local_cog_metadata");
        assert_eq!(run.steps[3].tool_calls[0].tool, "cog_metadata_verify");
    }

    #[test]
    fn runs_nagoya_geoparquet_agent_trace() {
        let path = genegis_catalog::nagoya_wards_geoparquet_path();
        if !std::path::Path::new(path).exists() {
            return;
        }

        let run = AgentOrchestrator::new()
            .with_config(AgentRunConfig::rule_based_offline())
            .run("名古屋 wards GeoParquet を検証")
            .expect("run");

        assert!(run.verification_passed);
        assert_eq!(run.workflow_id.as_deref(), Some("nagoya-geoparquet"));
        assert_eq!(run.steps[2].tool_calls[0].tool, "run_geoparquet_read");
        assert_eq!(run.steps[3].tool_calls[0].tool, "geoparquet_feature_verify");
    }

    #[test]
    #[ignore = "requires network access to OSGeo sample COG"]
    fn runs_remote_cog_agent_trace() {
        let run = AgentOrchestrator::new()
            .with_config(AgentRunConfig::rule_based_offline())
            .run("リモートCOGデモのメタデータを表示")
            .expect("run");

        assert!(run.verification_passed);
        assert_eq!(run.workflow_id.as_deref(), Some("remote-cog-demo"));
        assert_eq!(run.steps.len(), 4);
        assert_eq!(run.steps[2].tool_calls[0].tool, "run_remote_cog_metadata");
        assert_eq!(run.steps[3].tool_calls[0].tool, "cog_metadata_verify");
    }
}
