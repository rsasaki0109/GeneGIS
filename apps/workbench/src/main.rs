//! Local web workbench — serves GeneGIS UI and runs the ask pipeline via HTTP.

use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use genegis_analysis::{run_ask_pipeline, spawn_nagoya_gpu_preview};
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use tower_http::services::ServeDir;

#[derive(Clone)]
struct AppState {
    static_dir: PathBuf,
}

#[derive(Deserialize)]
struct AskRequest {
    prompt: String,
}

#[derive(Serialize)]
struct AskResponse {
    ok: bool,
    error: Option<String>,
    result: Option<genegis_analysis::AskPipelineResult>,
}

#[derive(Serialize)]
struct GpuPreviewResponse {
    ok: bool,
    error: Option<String>,
    message: Option<String>,
}

#[tokio::main]
async fn main() {
    let static_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../desktop/ui");
    let state = Arc::new(AppState {
        static_dir: static_dir.clone(),
    });

    let app = Router::new()
        .route("/", get(index))
        .route("/api/ask", post(ask))
        .route("/api/gpu-preview", post(gpu_preview))
        .fallback_service(ServeDir::new(static_dir))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 7812));
    let url = format!("http://{addr}/");
    println!("GeneGIS Workbench at {url}");

    if open::that(&url).is_err() {
        eprintln!("Open {url} in your browser.");
    }

    let listener = tokio::net::TcpListener::bind(addr).await.expect("bind");
    axum::serve(listener, app).await.expect("serve");
}

async fn index(State(state): State<Arc<AppState>>) -> Html<String> {
    let path = state.static_dir.join("index.html");
    Html(tokio::fs::read_to_string(path).await.unwrap_or_else(|_| fallback_index()))
}

async fn ask(Json(body): Json<AskRequest>) -> impl IntoResponse {
    match run_ask_pipeline(body.prompt.trim()) {
        Ok(result) => (
            StatusCode::OK,
            Json(AskResponse {
                ok: true,
                error: None,
                result: Some(result),
            }),
        ),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(AskResponse {
                ok: false,
                error: Some(err.to_string()),
                result: None,
            }),
        ),
    }
}

async fn gpu_preview() -> impl IntoResponse {
    match spawn_nagoya_gpu_preview() {
        Ok(()) => (
            StatusCode::OK,
            Json(GpuPreviewResponse {
                ok: true,
                error: None,
                message: Some("WebGPU choropleth preview launched".into()),
            }),
        ),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(GpuPreviewResponse {
                ok: false,
                error: Some(err.to_string()),
                message: None,
            }),
        ),
    }
}

fn fallback_index() -> String {
    "<html><body><h1>GeneGIS Workbench</h1><p>Static UI not found.</p></body></html>"
        .into()
}
