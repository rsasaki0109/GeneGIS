use genegis_catalog::alpha_catalog;
use genegis_workflow::{GeoWorkflow, nagoya_population_density_template};

use crate::backend::{PlannerBackend, PlannerConfig};
use crate::error::AiError;
use crate::intent::ParsedIntent;
use crate::llm::{merge_llm_intent, plan_with_llm};
use crate::resolver::{bind_catalog_dataset, ResolvedWorkflow, WorkflowId, resolve_workflow};

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
}

pub fn plan_from_prompt(prompt: &str) -> Result<PlanResult, AiError> {
    plan_with_config(prompt, &PlannerConfig::default())
}

pub fn plan_with_config(prompt: &str, config: &PlannerConfig) -> Result<PlanResult, AiError> {
    match config.backend {
        PlannerBackend::RuleBased => plan_rule_based(prompt),
        PlannerBackend::Llm => match plan_with_llm(prompt, config) {
            Ok(mut resolved) => {
                let intent = merge_llm_intent(prompt, &resolved);
                bind_catalog_dataset(&alpha_catalog(), &mut resolved)?;
                build_plan(intent, resolved, "llm_openai_compatible")
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
    build_plan(intent, resolved, "rule_based_mvp")
}

fn build_plan(
    intent: ParsedIntent,
    resolved: ResolvedWorkflow,
    mode: &'static str,
) -> Result<PlanResult, AiError> {
    let mut workflow = workflow_for(resolved.workflow_id);
    workflow.goal = resolved.goal.clone();

    Ok(PlanResult {
        intent,
        resolved,
        workflow,
        mode,
    })
}

fn workflow_for(id: WorkflowId) -> GeoWorkflow {
    match id {
        WorkflowId::NagoyaDensity => nagoya_population_density_template(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use genegis_catalog::NAGOYA_WARDS_DENSITY_ID;

    #[test]
    fn plans_north_star() {
        let plan = plan_from_prompt("名古屋市の人口密度を表示").expect("plan");
        assert_eq!(plan.resolved.workflow_id, WorkflowId::NagoyaDensity);
        assert_eq!(plan.resolved.dataset_id, NAGOYA_WARDS_DENSITY_ID);
        assert_eq!(plan.workflow.steps.len(), 14);
        assert_eq!(plan.mode, "rule_based_mvp");
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
    }
}
