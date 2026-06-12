use base64::Engine;
use serde::Deserialize;

use crate::crdt::{CollabApiPayload, CollabUpload};
use crate::error::CollabError;
use crate::session::CollabSession;

/// Default GeneGIS Server base URL (`genegis-server` on port 7813).
pub const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:7813";

#[derive(Debug, Deserialize)]
struct CollabApiResponse {
    ok: bool,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    automerge_snapshot: Option<String>,
}

fn normalize_base_url(base_url: &str) -> &str {
    base_url.trim_end_matches('/')
}

fn parse_collab_response(url: &str, body: &str) -> Result<CollabSession, CollabError> {
    let payload: CollabApiResponse =
        serde_json::from_str(body).map_err(|err| CollabError::Remote(err.to_string()))?;

    if !payload.ok {
        return Err(CollabError::Remote(format!("{url} returned ok=false")));
    }

    if let Some(snapshot) = payload.automerge_snapshot.as_deref() {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(snapshot)
            .map_err(|err| CollabError::Remote(err.to_string()))?;
        let mut session = CollabSession::from_snapshot(&bytes)?;
        if let Some(session_json) = payload.session.as_deref() {
            session.merge_json(session_json)?;
        }
        return Ok(session);
    }

    let Some(session_json) = payload.session else {
        return Err(CollabError::Remote(format!("{url} missing session field")));
    };

    CollabSession::import_json(&session_json)
}

/// Pull the shared collab session from GeneGIS Server.
pub fn pull_session(base_url: &str) -> Result<CollabSession, CollabError> {
    let url = format!("{}/api/collab", normalize_base_url(base_url));
    let mut response = ureq::get(&url)
        .call()
        .map_err(|err| CollabError::Remote(err.to_string()))?;

    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|err| CollabError::Remote(err.to_string()))?;

    parse_collab_response(&url, &body)
}

/// Push a collab session to GeneGIS Server (Automerge merge + JSON projection).
pub fn push_session(base_url: &str, session: &CollabSession) -> Result<CollabSession, CollabError> {
    let mut session = session.clone();
    let upload = CollabUpload {
        session: session.export_json()?,
        automerge_snapshot: Some(session.snapshot_base64()?),
    };
    let url = format!("{}/api/collab", normalize_base_url(base_url));
    let mut response = ureq::put(&url)
        .send_json(&upload)
        .map_err(|err| CollabError::Remote(err.to_string()))?;

    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|err| CollabError::Remote(err.to_string()))?;

    parse_collab_response(&url, &body).or_else(|_| CollabSession::import_json(&upload.session))
}

/// Parse a server JSON payload into the API view (for tests and tooling).
pub fn parse_api_payload(body: &str) -> Result<CollabApiPayload, CollabError> {
    serde_json::from_str(body).map_err(|err| CollabError::Remote(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_fails_when_server_unreachable() {
        let err = pull_session("http://127.0.0.1:1").expect_err("connection refused");
        assert!(matches!(err, CollabError::Remote(_)));
    }
}
