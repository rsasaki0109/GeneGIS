use serde::Deserialize;

use crate::backend::PlannerConfig;
use crate::error::AiError;
use crate::intent::ParsedIntent;
use crate::resolver::{ResolvedWorkflow, WorkflowId};
use crate::tool_call::PlannerToolCall;

#[derive(Debug, Deserialize)]
struct LlmPlanPayload {
    workflow_id: WorkflowId,
    goal: String,
    confidence: f32,
    rationale: Vec<String>,
    ambiguities: Vec<String>,
    #[serde(default)]
    tool_calls: Vec<PlannerToolCall>,
}

const SYSTEM_PROMPT: &str = r#"You are the GeneGIS workflow planner. Map the user request to exactly one MVP workflow.

Available workflows:
- nagoya-density — population density choropleth for Nagoya city wards (16 wards, 2020 census, N03 boundaries)
- remote-cog-demo — fetch remote GeoTIFF/COG metadata via catalog + HTTP range-read
- local-cog-demo — read bundled local COG metadata fixture (offline)

Respond with JSON only (no markdown):
{
  "workflow_id":"nagoya-density|remote-cog-demo|local-cog-demo",
  "goal":"<user goal>",
  "confidence":0.0,
  "rationale":["..."],
  "ambiguities":["..."],
  "tool_calls":[
    {
      "tool":"llm_plan_workflow",
      "input":{"prompt":"<echo user prompt>"},
      "output":{"workflow_id":"nagoya-density","confidence":0.9},
      "ok":true
    }
  ]
}

tool_calls must list structured planner steps you performed (at minimum llm_plan_workflow). If unsupported, set confidence to 0, explain in ambiguities, and set ok=false on tool_calls."#;

pub fn plan_with_llm(
    prompt: &str,
    config: &PlannerConfig,
) -> Result<(ResolvedWorkflow, Vec<PlannerToolCall>), AiError> {
    let api_key = config
        .llm_api_key
        .as_deref()
        .filter(|key| !key.trim().is_empty())
        .ok_or_else(|| AiError::LlmConfig("GENEGIS_LLM_API_KEY is not set".into()))?;

    let base = config.llm_base_url.trim_end_matches('/');
    let url = format!("{base}/chat/completions");

    let body = serde_json::json!({
        "model": config.llm_model,
        "temperature": 0.0,
        "response_format": { "type": "json_object" },
        "messages": [
            { "role": "system", "content": SYSTEM_PROMPT },
            { "role": "user", "content": prompt }
        ]
    });

    let mut response = ureq::post(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .send_json(body)
        .map_err(|err: ureq::Error| AiError::LlmTransport(err.to_string()))?;

    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        let detail = response.body_mut().read_to_string().unwrap_or_default();
        return Err(AiError::LlmTransport(format!("HTTP {status}: {detail}")));
    }

    let payload: serde_json::Value = response
        .body_mut()
        .read_json()
        .map_err(|err: ureq::Error| AiError::LlmTransport(err.to_string()))?;

    let content = payload
        .pointer("/choices/0/message/content")
        .and_then(|value| value.as_str())
        .ok_or_else(|| AiError::LlmResponse("missing message content".into()))?;

    parse_llm_plan(content, prompt)
}

pub fn parse_llm_plan(
    content: &str,
    prompt: &str,
) -> Result<(ResolvedWorkflow, Vec<PlannerToolCall>), AiError> {
    let parsed: LlmPlanPayload = serde_json::from_str(content.trim())
        .map_err(|err| AiError::LlmResponse(format!("invalid JSON: {err}")))?;

    if parsed.confidence <= 0.0 {
        return Err(AiError::Unresolved(format!(
            "LLM could not resolve prompt: \"{prompt}\""
        )));
    }

    Ok((
        ResolvedWorkflow {
            workflow_id: parsed.workflow_id,
            dataset_id: String::new(),
            goal: if parsed.goal.trim().is_empty() {
                prompt.to_string()
            } else {
                parsed.goal
            },
            confidence: parsed.confidence.clamp(0.0, 1.0),
            rationale: parsed.rationale,
            ambiguities: parsed.ambiguities,
        },
        parsed.tool_calls,
    ))
}

pub fn merge_llm_intent(prompt: &str, resolved: &ResolvedWorkflow) -> ParsedIntent {
    let mut intent = ParsedIntent::parse(prompt);
    intent.confidence = resolved.confidence.max(intent.confidence);
    if !resolved.rationale.is_empty() {
        intent.signals.matched_tokens = resolved.rationale.clone();
    }
    intent
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_llm_json_payload() {
        let json = r#"{
            "workflow_id": "nagoya-density",
            "goal": "Show Nagoya population density",
            "confidence": 0.92,
            "rationale": ["place:nagoya", "metric:population_density"],
            "ambiguities": ["ward granularity"],
            "tool_calls": [
                {
                    "tool": "llm_plan_workflow",
                    "input": {"prompt": "Show Nagoya population density"},
                    "output": {"workflow_id": "nagoya-density", "confidence": 0.92},
                    "ok": true
                }
            ]
        }"#;
        let (resolved, tools) =
            parse_llm_plan(json, "Show Nagoya population density").expect("parse");
        assert_eq!(resolved.workflow_id, WorkflowId::NagoyaDensity);
        assert!(resolved.confidence > 0.9);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].tool, "llm_plan_workflow");
    }
}
