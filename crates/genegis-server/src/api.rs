//! HTTP routes for collab + agent run sync.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use genegis_agent::{AgentRun, AgentRunSummary};
use genegis_collab::{CollabApiPayload, CollabUpload};
use crate::agent_store::AgentRunStore;
use crate::store::CollabStore;
use serde::Serialize;
use tower_http::cors::CorsLayer;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub collab: Arc<CollabStore>,
    pub agent_runs: Arc<AgentRunStore>,
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub ok: bool,
    pub service: &'static str,
    pub collab_path: String,
    pub automerge_path: String,
    pub agent_runs_dir: String,
    pub agent_latest_path: String,
}

#[derive(Serialize)]
pub struct AgentRunResponse {
    pub ok: bool,
    pub error: Option<String>,
    pub run: Option<AgentRun>,
}

#[derive(Serialize)]
pub struct AgentRunListResponse {
    pub ok: bool,
    pub error: Option<String>,
    pub runs: Vec<AgentRunSummary>,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/collab", get(get_collab).put(put_collab))
        .route("/api/agent/runs/latest", get(get_latest_agent_run))
        .route("/api/agent/runs/{id}", get(get_agent_run_by_id))
        .route(
            "/api/agent/runs",
            get(list_agent_runs).post(post_agent_run),
        )
        .layer(CorsLayer::permissive())
        .with_state(state)
}

pub async fn serve(state: AppState, addr: SocketAddr) -> Result<(), std::io::Error> {
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await
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

async fn list_agent_runs(State(state): State<AppState>) -> impl IntoResponse {
    match state.agent_runs.list() {
        Ok(runs) => (
            StatusCode::OK,
            Json(AgentRunListResponse {
                ok: true,
                error: None,
                runs,
            }),
        ),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(AgentRunListResponse {
                ok: false,
                error: Some(err.to_string()),
                runs: Vec::new(),
            }),
        ),
    }
}

async fn get_latest_agent_run(State(state): State<AppState>) -> impl IntoResponse {
    match state.agent_runs.latest() {
        Ok(Some(run)) => agent_run_ok(run),
        Ok(None) => agent_run_ok_empty(),
        Err(err) => agent_run_err(err.to_string()),
    }
}

async fn get_agent_run_by_id(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.agent_runs.get(id) {
        Ok(Some(run)) => agent_run_ok(run),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(AgentRunResponse {
                ok: false,
                error: Some(format!("agent run not found: {id}")),
                run: None,
            }),
        ),
        Err(err) => agent_run_err(err.to_string()),
    }
}

async fn post_agent_run(
    State(state): State<AppState>,
    Json(body): Json<AgentRun>,
) -> impl IntoResponse {
    match state.agent_runs.save(&body) {
        Ok(run) => agent_run_ok(run),
        Err(err) => agent_run_err(err.to_string()),
    }
}

fn agent_run_ok(run: AgentRun) -> (StatusCode, Json<AgentRunResponse>) {
    (
        StatusCode::OK,
        Json(AgentRunResponse {
            ok: true,
            error: None,
            run: Some(run),
        }),
    )
}

fn agent_run_ok_empty() -> (StatusCode, Json<AgentRunResponse>) {
    (
        StatusCode::OK,
        Json(AgentRunResponse {
            ok: true,
            error: None,
            run: None,
        }),
    )
}

fn agent_run_err(message: String) -> (StatusCode, Json<AgentRunResponse>) {
    (
        StatusCode::BAD_REQUEST,
        Json(AgentRunResponse {
            ok: false,
            error: Some(message),
            run: None,
        }),
    )
}

fn collab_error(message: String) -> (StatusCode, Json<CollabApiPayload>) {
    (
        StatusCode::BAD_REQUEST,
        Json(CollabApiPayload::error(&message)),
    )
}
