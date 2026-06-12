use genegis_catalog::alpha_catalog;
use genegis_workflow::{
    nagoya_population_density_template, remote_cog_metadata_template, GeoWorkflow,
};

use crate::backend::{PlannerBackend, PlannerConfig};
use crate::error::AiError;
use crate::intent::ParsedIntent;
use crate::llm::{merge_llm_intent, plan_with_llm};
use crate::resolver::{bind_catalog_dataset, ResolvedWorkflow, WorkflowId, resolve_workflow};
use crate::tool_call::{llm_tool_calls, rule_based_tool_calls, PlannerToolCall};

/// Whether to return the plan only or proceed to execution (CLI maps this).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanMode {
    PlanOnly,
    Execute,
}

/// AI planning output: intent → workflow IR.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlanResult {
    pub intent: ParsedIntent,
    pub resolved: ResolvedWorkflow,
    pub workflow: GeoWorkflow,
    pub mode: &'static str,
    pub tool_calls: Vec<PlannerToolCall>,
}

pub fn plan_from_prompt(prompt: &str) -> Result<PlanResult, AiError> {
    plan_with_config(prompt, &PlannerConfig::default())
}

pub fn plan_with_config(prompt: &str, config: &PlannerConfig) -> Result<PlanResult, AiError> {
    match config.backend {
        PlannerBackend::RuleBased => plan_rule_based(prompt),
        PlannerBackend::Llm => match plan_with_llm(prompt, config) {
            Ok((mut resolved, llm_tools)) => {
                let intent = merge_llm_intent(prompt, &resolved);
                bind_catalog_dataset(&alpha_catalog(), &mut resolved)?;
                let tool_calls = llm_tool_calls(&resolved, &llm_tools);
                build_plan(intent, resolved, "llm_openai_compatible", tool_calls)
            }
            Err(err) if config.fallback_to_rules => {
                eprintln!("LLM planner failed ({err}); falling back to rule-based resolver");
                plan_rule_based(prompt)
            }
            Err(err) => Err(err),
        },
    }
}

fn plan_rule_based(prompt: &str) -> Result<PlanResult, AiError> {
    let intent = ParsedIntent::parse(prompt);
    let resolved = resolve_workflow(&intent)?;
    let tool_calls = rule_based_tool_calls(&intent, &resolved);
    build_plan(intent, resolved, "rule_based_mvp", tool_calls)
}

fn build_plan(
    intent: ParsedIntent,
    resolved: ResolvedWorkflow,
    mode: &'static str,
    tool_calls: Vec<PlannerToolCall>,
) -> Result<PlanResult, AiError> {
    let mut workflow = workflow_for(resolved.workflow_id);
    workflow.goal = resolved.goal.clone();

    Ok(PlanResult {
        intent,
        resolved,
        workflow,
        mode,
        tool_calls,
    })
}

fn workflow_for(id: WorkflowId) -> GeoWorkflow {
    match id {
        WorkflowId::NagoyaDensity => nagoya_population_density_template(),
        WorkflowId::RemoteCogDemo => remote_cog_metadata_template(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use genegis_catalog::{NAGOYA_WARDS_DENSITY_ID, REMOTE_COG_DEMO_ID};

    #[test]
    fn plans_north_star() {
        let plan = plan_from_prompt("名古屋市の人口密度を表示").expect("plan");
        assert_eq!(plan.resolved.workflow_id, WorkflowId::NagoyaDensity);
        assert_eq!(plan.resolved.dataset_id, NAGOYA_WARDS_DENSITY_ID);
        assert_eq!(plan.workflow.steps.len(), 14);
        assert_eq!(plan.mode, "rule_based_mvp");
    }

    #[test]
    fn plans_remote_cog_demo() {
        let plan = plan_from_prompt("リモートCOGデモのメタデータを表示").expect("plan");
        assert_eq!(plan.resolved.workflow_id, WorkflowId::RemoteCogDemo);
        assert_eq!(plan.resolved.dataset_id, REMOTE_COG_DEMO_ID);
        assert_eq!(plan.workflow.steps.len(), 4);
    }

    #[test]
    fn llm_without_key_falls_back_to_rules() {
        let config = PlannerConfig {
            backend: PlannerBackend::Llm,
            llm_api_key: None,
            llm_base_url: "https://example.com/v1".into(),
            llm_model: "test".into(),
            fallback_to_rules: true,
        };
        let plan = plan_with_config("名古屋市の人口密度を表示", &config).expect("plan");
        assert_eq!(plan.mode, "rule_based_mvp");
        assert_eq!(plan.tool_calls.len(), 3);
        assert_eq!(plan.tool_calls[0].tool, "parse_intent");
    }

    #[test]
    fn rule_based_plan_includes_tool_calls() {
        let plan = plan_from_prompt("名古屋市の人口密度を表示").expect("plan");
        assert_eq!(plan.tool_calls.len(), 3);
        assert!(plan.tool_calls.iter().all(|call| call.ok));
    }
}
