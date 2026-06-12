//! Local web workbench — serves GeneGIS UI and runs the ask pipeline via HTTP.

use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use genegis_analysis::{run_ask_pipeline, spawn_nagoya_gpu_preview};
use genegis_collab::CollabSession;
use genegis_plugin_host::PluginHost;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use tower_http::services::ServeDir;

#[derive(Clone)]
struct AppState {
    static_dir: PathBuf,
    plugin_root: PathBuf,
    collab: Mutex<CollabSession>,
}

#[derive(Serialize)]
struct CollabResponse {
    ok: bool,
    summary: serde_json::Value,
    comments: serde_json::Value,
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

#[derive(Serialize)]
struct PluginsResponse {
    ok: bool,
    error: Option<String>,
    plugin_root: String,
    plugins: Vec<serde_json::Value>,
}

#[tokio::main]
async fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let static_dir = manifest_dir.join("../desktop/ui");
    let plugin_root = resolve_plugin_root(&manifest_dir);
    let collab = load_collab_session();
    let state = Arc::new(AppState {
        static_dir: static_dir.clone(),
        plugin_root: plugin_root.clone(),
        collab: Mutex::new(collab),
    });

    let app = Router::new()
        .route("/", get(index))
        .route("/api/ask", post(ask))
        .route("/api/gpu-preview", post(gpu_preview))
        .route("/api/plugins", get(list_plugins))
        .route("/api/collab", get(collab_snapshot))
        .fallback_service(ServeDir::new(static_dir))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 7812));
    let url = format!("http://{addr}/");
    println!("GeneGIS Workbench at {url}");
    println!("Plugin root: {}", plugin_root.display());

    if open::that(&url).is_err() {
        eprintln!("Open {url} in your browser.");
    }

    let listener = tokio::net::TcpListener::bind(addr).await.expect("bind");
    axum::serve(listener, app).await.expect("serve");
}

fn resolve_plugin_root(manifest_dir: &PathBuf) -> PathBuf {
    let cwd_plugins = PathBuf::from("plugins");
    if cwd_plugins.is_dir() {
        return cwd_plugins;
    }

    let repo_plugins = manifest_dir.join("../../plugins");
    if repo_plugins.is_dir() {
        return repo_plugins;
    }

    cwd_plugins
}

fn load_collab_session() -> CollabSession {
    let path = PathBuf::from(".genegis/collab.json");
    if path.is_file() {
        if let Ok(json) = std::fs::read_to_string(&path) {
            if let Ok(session) = CollabSession::import_json(&json) {
                return session;
            }
        }
    }
    CollabSession::demo_nagoya()
}

async fn collab_snapshot(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let session = state
        .collab
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    (
        StatusCode::OK,
        Json(CollabResponse {
            ok: true,
            summary: session.summary_json(),
            comments: session.comments_json(),
        }),
    )
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

async fn list_plugins(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let host = PluginHost::new();
    match host.discover_plugins(&state.plugin_root) {
        Ok(entries) => {
            let plugins = entries
                .iter()
                .map(|entry| entry.summary_json())
                .collect();
            (
                StatusCode::OK,
                Json(PluginsResponse {
                    ok: true,
                    error: None,
                    plugin_root: state.plugin_root.display().to_string(),
                    plugins,
                }),
            )
        }
        Err(err) => (
            StatusCode::OK,
            Json(PluginsResponse {
                ok: false,
                error: Some(err.to_string()),
                plugin_root: state.plugin_root.display().to_string(),
                plugins: Vec::new(),
            }),
        ),
    }
}

fn fallback_index() -> String {
    "<html><body><h1>GeneGIS Workbench</h1><p>Static UI not found.</p></body></html>"
        .into()
}
