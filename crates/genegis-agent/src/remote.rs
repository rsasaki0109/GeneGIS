use serde::Deserialize;
use uuid::Uuid;

use crate::error::AgentError;
use crate::model::{AgentRun, AgentRunSummary};

/// Default GeneGIS Server base URL (`genegis-server` on port 7813).
pub const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:7813";

#[derive(Debug, Deserialize)]
struct AgentRunApiResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    run: Option<AgentRun>,
}

#[derive(Debug, Deserialize)]
struct AgentRunListApiResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    runs: Vec<AgentRunSummary>,
}

fn normalize_base_url(base_url: &str) -> &str {
    base_url.trim_end_matches('/')
}

fn parse_agent_response(url: &str, body: &str) -> Result<AgentRun, AgentError> {
    let payload: AgentRunApiResponse =
        serde_json::from_str(body).map_err(|err| AgentError::Json(err.to_string()))?;

    if !payload.ok {
        return Err(AgentError::Message(
            payload
                .error
                .unwrap_or_else(|| format!("{url} returned ok=false")),
        ));
    }

    payload
        .run
        .ok_or_else(|| AgentError::Message(format!("{url} missing run field")))
}

fn parse_agent_list_response(url: &str, body: &str) -> Result<Vec<AgentRunSummary>, AgentError> {
    let payload: AgentRunListApiResponse =
        serde_json::from_str(body).map_err(|err| AgentError::Json(err.to_string()))?;

    if !payload.ok {
        return Err(AgentError::Message(
            payload
                .error
                .unwrap_or_else(|| format!("{url} returned ok=false")),
        ));
    }

    Ok(payload.runs)
}

/// Pull the latest agent run trace from GeneGIS Server.
pub fn pull_latest_agent_run(base_url: &str) -> Result<AgentRun, AgentError> {
    let url = format!("{}/api/agent/runs/latest", normalize_base_url(base_url));
    let mut response = ureq::get(&url)
        .call()
        .map_err(|err| AgentError::Message(err.to_string()))?;

    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|err| AgentError::Message(err.to_string()))?;

    parse_agent_response(&url, &body)
}

/// List agent run summaries from GeneGIS Server.
pub fn list_agent_runs(base_url: &str) -> Result<Vec<AgentRunSummary>, AgentError> {
    let url = format!("{}/api/agent/runs", normalize_base_url(base_url));
    let mut response = ureq::get(&url)
        .call()
        .map_err(|err| AgentError::Message(err.to_string()))?;

    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|err| AgentError::Message(err.to_string()))?;

    parse_agent_list_response(&url, &body)
}

/// Fetch a specific agent run trace from GeneGIS Server.
pub fn get_agent_run(base_url: &str, id: Uuid) -> Result<AgentRun, AgentError> {
    let url = format!("{}/api/agent/runs/{id}", normalize_base_url(base_url));
    let mut response = ureq::get(&url)
        .call()
        .map_err(|err| AgentError::Message(err.to_string()))?;

    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|err| AgentError::Message(err.to_string()))?;

    parse_agent_response(&url, &body)
}

/// Push an agent run trace to GeneGIS Server.
pub fn push_agent_run(base_url: &str, run: &AgentRun) -> Result<AgentRun, AgentError> {
    let url = format!("{}/api/agent/runs", normalize_base_url(base_url));
    let mut response = ureq::post(&url)
        .send_json(run)
        .map_err(|err| AgentError::Message(err.to_string()))?;

    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|err| AgentError::Message(err.to_string()))?;

    parse_agent_response(&url, &body).or_else(|_| Ok(run.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_fails_when_server_unreachable() {
        let err = pull_latest_agent_run("http://127.0.0.1:1").expect_err("connection refused");
        assert!(matches!(err, AgentError::Message(_)));
    }

    #[test]
    fn list_fails_when_server_unreachable() {
        let err = list_agent_runs("http://127.0.0.1:1").expect_err("connection refused");
        assert!(matches!(err, AgentError::Message(_)));
    }
}
