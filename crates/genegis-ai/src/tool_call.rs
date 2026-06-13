use genegis_catalog::{alpha_catalog, browse_alpha_stac_collection, bind_stac_item};
use serde::{Deserialize, Serialize};

use crate::intent::ParsedIntent;
use crate::resolver::ResolvedWorkflow;

/// Structured planner tool invocation (Phase 6 beta — LLM + rule fallback).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlannerToolCall {
    pub tool: String,
    pub input: serde_json::Value,
    pub output: serde_json::Value,
    pub ok: bool,
}

impl PlannerToolCall {
    pub fn new(
        tool: impl Into<String>,
        input: serde_json::Value,
        output: serde_json::Value,
        ok: bool,
    ) -> Self {
        Self {
            tool: tool.into(),
            input,
            output,
            ok,
        }
    }
}

pub fn rule_based_tool_calls(intent: &ParsedIntent, resolved: &ResolvedWorkflow) -> Vec<PlannerToolCall> {
    let catalog = alpha_catalog();
    let collection = browse_alpha_stac_collection(&catalog);
    let stac_item = bind_stac_item(&catalog, &resolved.dataset_id).ok();

    vec![
        PlannerToolCall::new(
            "parse_intent",
            serde_json::json!({ "prompt": intent.raw_prompt }),
            serde_json::json!({
                "place": intent.signals.place,
                "metric": intent.signals.metric,
                "confidence": intent.confidence,
                "matched_tokens": intent.signals.matched_tokens,
            }),
            true,
        ),
        PlannerToolCall::new(
            "resolve_workflow",
            serde_json::json!({
                "place": intent.signals.place,
                "metric": intent.signals.metric,
            }),
            serde_json::json!({
                "workflow_id": resolved.workflow_id.as_str(),
                "goal": resolved.goal,
                "confidence": resolved.confidence,
                "ambiguities": resolved.ambiguities,
            }),
            true,
        ),
        PlannerToolCall::new(
            "stac_browse",
            serde_json::json!({ "collection_id": collection.id }),
            collection.summary_json(),
            true,
        ),
        PlannerToolCall::new(
            "stac_bind",
            serde_json::json!({
                "collection_id": collection.id,
                "workflow_id": resolved.workflow_id.as_str(),
                "tags": resolved.workflow_id.dataset_tags(),
            }),
            serde_json::json!({
                "dataset_id": resolved.dataset_id,
                "stac_item_id": stac_item.as_ref().map(|item| item.id.clone()),
                "rationale": resolved.rationale,
            }),
            !resolved.dataset_id.is_empty(),
        ),
    ]
}

pub fn llm_tool_calls(resolved: &ResolvedWorkflow, raw: &[PlannerToolCall]) -> Vec<PlannerToolCall> {
    if raw.is_empty() {
        return llm_synthetic_tool_calls(resolved);
    }
    raw.to_vec()
}

fn llm_synthetic_tool_calls(resolved: &ResolvedWorkflow) -> Vec<PlannerToolCall> {
    let catalog = alpha_catalog();
    let collection = browse_alpha_stac_collection(&catalog);
    vec![
        PlannerToolCall::new(
            "llm_plan_workflow",
            serde_json::json!({ "model": "openai_compatible" }),
            serde_json::json!({
                "workflow_id": resolved.workflow_id.as_str(),
                "goal": resolved.goal,
                "confidence": resolved.confidence,
                "rationale": resolved.rationale,
                "ambiguities": resolved.ambiguities,
            }),
            true,
        ),
        PlannerToolCall::new(
            "stac_browse",
            serde_json::json!({ "collection_id": collection.id }),
            collection.summary_json(),
            true,
        ),
        PlannerToolCall::new(
            "stac_bind",
            serde_json::json!({
                "collection_id": collection.id,
                "workflow_id": resolved.workflow_id.as_str(),
                "tags": resolved.workflow_id.dataset_tags(),
            }),
            serde_json::json!({
                "dataset_id": resolved.dataset_id,
            }),
            !resolved.dataset_id.is_empty(),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::ParsedIntent;
    use crate::resolver::resolve_workflow;
    use genegis_catalog::NAGOYA_WARDS_DENSITY_ID;

    #[test]
    fn rule_based_emits_stac_tool_calls() {
        let intent = ParsedIntent::parse("名古屋市の人口密度を表示");
        let resolved = resolve_workflow(&intent).expect("resolve");
        let calls = rule_based_tool_calls(&intent, &resolved);
        assert_eq!(calls.len(), 4);
        assert_eq!(calls[0].tool, "parse_intent");
        assert_eq!(calls[2].tool, "stac_browse");
        assert_eq!(calls[3].tool, "stac_bind");
        assert!(calls[3].ok);
        assert_eq!(resolved.dataset_id, NAGOYA_WARDS_DENSITY_ID);
    }

    #[test]
    fn llm_synthetic_when_payload_empty() {
        let resolved = resolve_workflow(&ParsedIntent::parse("名古屋市の人口密度を表示")).expect("resolve");
        let calls = llm_tool_calls(&resolved, &[]);
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].tool, "llm_plan_workflow");
        assert_eq!(calls[1].tool, "stac_browse");
    }
}
