use genegis_catalog::alpha_catalog;
use genegis_workflow::{
    local_cog_metadata_template, nagoya_population_density_template,
    remote_cog_metadata_template, GeoWorkflow,
};

use crate::backend::{PlannerBackend, PlannerConfig};
use crate::error::AiError;
use crate::intent::ParsedIntent;
use crate::llm::{merge_llm_intent, plan_with_llm};
use crate::resolver::{bind_catalog_dataset, ResolvedWorkflow, WorkflowId, resolve_workflow};
use crate::tool_call::{llm_tool_calls, rule_based_tool_calls, PlannerToolCall};

/// Default path for a human-gated pending agent plan JSON.
pub const DEFAULT_AGENT_PLAN_PATH: &str = ".genegis/agent-plan.json";

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
    pub mode: String,
    pub tool_calls: Vec<PlannerToolCall>,
}

impl PlanResult {
    pub fn trace_json(&self) -> Result<String, AiError> {
        serde_json::to_string_pretty(self).map_err(|err| AiError::Json(err.to_string()))
    }

    pub fn save_to_path(&self, path: impl AsRef<std::path::Path>) -> Result<(), AiError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| AiError::Json(err.to_string()))?;
        }
        std::fs::write(path, self.trace_json()?).map_err(|err| AiError::Json(err.to_string()))
    }

    pub fn load_from_path(path: impl AsRef<std::path::Path>) -> Result<Self, AiError> {
        let json = std::fs::read_to_string(path.as_ref())
            .map_err(|err| AiError::Json(err.to_string()))?;
        serde_json::from_str(&json).map_err(|err| AiError::Json(err.to_string()))
    }
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
                build_plan(intent, resolved, "llm_openai_compatible".into(), tool_calls)
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
    build_plan(intent, resolved, "rule_based_mvp".into(), tool_calls)
}

fn build_plan(
    intent: ParsedIntent,
    resolved: ResolvedWorkflow,
    mode: String,
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
        WorkflowId::LocalCogDemo => local_cog_metadata_template(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use genegis_catalog::{LOCAL_COG_DEMO_ID, NAGOYA_WARDS_DENSITY_ID, REMOTE_COG_DEMO_ID};

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
        assert_eq!(plan.tool_calls.len(), 4);
        assert_eq!(plan.tool_calls[0].tool, "parse_intent");
    }

    #[test]
    fn plans_local_cog_demo() {
        let plan = plan_from_prompt("ローカルCOGデモのメタデータを表示").expect("plan");
        assert_eq!(plan.resolved.workflow_id, WorkflowId::LocalCogDemo);
        assert_eq!(plan.workflow.steps.len(), 4);
    }

    #[test]
    fn rule_based_plan_includes_tool_calls() {
        let plan = plan_from_prompt("名古屋市の人口密度を表示").expect("plan");
        assert_eq!(plan.tool_calls.len(), 4);
        assert!(plan.tool_calls.iter().all(|call| call.ok));
    }

    #[test]
    fn plan_result_roundtrips_json() {
        let plan = plan_from_prompt("名古屋市の人口密度を表示").expect("plan");
        let json = serde_json::to_string(&plan).expect("serialize");
        let restored: PlanResult = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.resolved.workflow_id, plan.resolved.workflow_id);
        assert_eq!(restored.mode, plan.mode);
    }
}
