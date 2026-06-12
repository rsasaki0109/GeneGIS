//! GeneGIS Server — collab session + agent run sync prototype (Phase 5–6 beta).

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use genegis_agent::AgentRun;
use genegis_collab::{CollabApiPayload, CollabUpload};
use genegis_server::agent_store::{
    AgentRunStore, DEFAULT_AGENT_LATEST_PATH, DEFAULT_AGENT_RUNS_DIR,
};
use genegis_server::store::{CollabStore, DEFAULT_COLLAB_PATH};
use serde::Serialize;
use tower_http::cors::CorsLayer;

#[derive(Clone)]
struct AppState {
    collab: Arc<CollabStore>,
    agent_runs: Arc<AgentRunStore>,
}

#[derive(Serialize)]
struct HealthResponse {
    ok: bool,
    service: &'static str,
    collab_path: String,
    automerge_path: String,
    agent_runs_dir: String,
    agent_latest_path: String,
}

#[derive(Serialize)]
struct AgentRunResponse {
    ok: bool,
    error: Option<String>,
    run: Option<AgentRun>,
}

#[tokio::main]
async fn main() {
    let collab_path = std::env::var("GENEGIS_COLLAB_PATH")
        .unwrap_or_else(|_| DEFAULT_COLLAB_PATH.into());
    let agent_runs_dir = std::env::var("GENEGIS_AGENT_RUNS_DIR")
        .unwrap_or_else(|_| DEFAULT_AGENT_RUNS_DIR.into());
    let agent_latest_path = std::env::var("GENEGIS_AGENT_LATEST_PATH")
        .unwrap_or_else(|_| DEFAULT_AGENT_LATEST_PATH.into());

    let collab = Arc::new(CollabStore::load(&collab_path));
    let agent_runs = Arc::new(AgentRunStore::load(&agent_runs_dir, &agent_latest_path));
    let state = AppState {
        collab: Arc::clone(&collab),
        agent_runs: Arc::clone(&agent_runs),
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/api/collab", get(get_collab).put(put_collab))
        .route("/api/agent/runs/latest", get(get_latest_agent_run))
        .route("/api/agent/runs", get(get_latest_agent_run).post(post_agent_run))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let port: u16 = std::env::var("GENEGIS_SERVER_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(7813);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    println!("GeneGIS Server listening on http://{addr}/");
    println!("Collab JSON: {}", collab.json_path().display());
    println!("Collab Automerge: {}", collab.snapshot_path().display());
    println!("Agent runs: {}", agent_runs.runs_dir().display());
    println!("Agent latest: {}", agent_runs.latest_path().display());

    let listener = tokio::net::TcpListener::bind(addr).await.expect("bind");
    axum::serve(listener, app).await.expect("serve");
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        service: "genegis-server",
        collab_path: state.collab.json_path().display().to_string(),
        automerge_path: state.collab.snapshot_path().display().to_string(),
        agent_runs_dir: state.agent_runs.runs_dir().display().to_string(),
        agent_latest_path: state.agent_runs.latest_path().display().to_string(),
    })
}

async fn get_collab(State(state): State<AppState>) -> impl IntoResponse {
    match state.collab.snapshot() {
        Ok(session) => match CollabApiPayload::from_session(&session, true) {
            Ok(payload) => (StatusCode::OK, Json(payload)),
            Err(err) => collab_error(err.to_string()),
        },
        Err(err) => collab_error(err.to_string()),
    }
}

async fn put_collab(
    State(state): State<AppState>,
    Json(body): Json<CollabUpload>,
) -> impl IntoResponse {
    match state.collab.merge_upload(&body) {
        Ok(session) => match CollabApiPayload::from_session(&session, true) {
            Ok(payload) => (StatusCode::OK, Json(payload)),
            Err(err) => collab_error(err.to_string()),
        },
        Err(err) => collab_error(err.to_string()),
    }
}

async fn get_latest_agent_run(State(state): State<AppState>) -> impl IntoResponse {
    match state.agent_runs.latest() {
        Ok(Some(run)) => (
            StatusCode::OK,
            Json(AgentRunResponse {
                ok: true,
                error: None,
                run: Some(run),
            }),
        ),
        Ok(None) => (
            StatusCode::OK,
            Json(AgentRunResponse {
                ok: true,
                error: None,
                run: None,
            }),
        ),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(AgentRunResponse {
                ok: false,
                error: Some(err.to_string()),
                run: None,
            }),
        ),
    }
}

async fn post_agent_run(
    State(state): State<AppState>,
    Json(body): Json<AgentRun>,
) -> impl IntoResponse {
    match state.agent_runs.save(&body) {
        Ok(run) => (
            StatusCode::OK,
            Json(AgentRunResponse {
                ok: true,
                error: None,
                run: Some(run),
            }),
        ),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(AgentRunResponse {
                ok: false,
                error: Some(err.to_string()),
                run: None,
            }),
        ),
    }
}

fn collab_error(message: String) -> (StatusCode, Json<CollabApiPayload>) {
    (
        StatusCode::BAD_REQUEST,
        Json(CollabApiPayload::error(&message)),
    )
}
