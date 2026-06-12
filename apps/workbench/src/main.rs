//! Local web workbench — serves GeneGIS UI and runs the ask pipeline via HTTP.

use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use genegis_agent::{
    pull_latest_agent_run, push_agent_run, AgentOrchestrator, AgentRun, AgentRunConfig,
    AgentRole, DEFAULT_AGENT_RUN_PATH,
};
use genegis_analysis::{run_ask_pipeline, spawn_nagoya_gpu_preview};
use genegis_collab::{pull_session, push_session, CollabSession, MapComment, DEFAULT_SERVER_URL};
use genegis_plugin_host::PluginHost;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use tower_http::{cors::CorsLayer, services::ServeDir};

const DEFAULT_COLLAB_PATH: &str = ".genegis/collab.json";
const DEFAULT_AGENT_RUN_PATH_LOCAL: &str = DEFAULT_AGENT_RUN_PATH;

#[derive(Clone, Debug)]
struct SyncStatus {
    source: String,
    server_url: String,
    synced: bool,
    error: Option<String>,
}

#[derive(Clone)]
struct AppState {
    static_dir: PathBuf,
    plugin_root: PathBuf,
    collab_path: PathBuf,
    agent_run_path: PathBuf,
    server_url: String,
    collab: Arc<Mutex<CollabSession>>,
    sync: Arc<Mutex<SyncStatus>>,
}

#[derive(Serialize)]
struct CollabSyncMeta {
    source: String,
    server_url: String,
    synced: bool,
    error: Option<String>,
}

#[derive(Serialize)]
struct CollabResponse {
    ok: bool,
    summary: serde_json::Value,
    comments: serde_json::Value,
    sync: CollabSyncMeta,
}

#[derive(Deserialize)]
struct AddCommentRequest {
    author: String,
    body: String,
    map_anchor: Option<[f64; 2]>,
}

#[derive(Deserialize)]
struct AskRequest {
    prompt: String,
}

#[derive(Serialize)]
struct AgentRunResponse {
    ok: bool,
    error: Option<String>,
    run: Option<AgentRun>,
}

#[derive(Deserialize)]
struct AgentRunRequest {
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
    let collab_path = std::env::var("GENEGIS_COLLAB_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_COLLAB_PATH));
    let agent_run_path = std::env::var("GENEGIS_AGENT_RUN_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_AGENT_RUN_PATH_LOCAL));
    let server_url = std::env::var("GENEGIS_SERVER_URL")
        .unwrap_or_else(|_| DEFAULT_SERVER_URL.into());

    let (collab, sync) = load_initial_collab(&collab_path, &server_url).await;
    let agent_run_path_for_load = agent_run_path.clone();
    let server_url_for_agent = server_url.clone();
    let _ = load_initial_agent_run(&agent_run_path_for_load, &server_url_for_agent).await;
    let state = Arc::new(AppState {
        static_dir: static_dir.clone(),
        plugin_root: plugin_root.clone(),
        collab_path,
        agent_run_path,
        server_url: server_url.clone(),
        collab: Arc::new(Mutex::new(collab)),
        sync: Arc::new(Mutex::new(sync)),
    });

    let app = Router::new()
        .route("/", get(index))
        .route("/api/ask", post(ask))
        .route("/api/gpu-preview", post(gpu_preview))
        .route("/api/plugins", get(list_plugins))
        .route("/api/collab", get(collab_snapshot))
        .route("/api/collab/comment", post(add_comment))
        .route("/api/collab/sync", post(sync_collab))
        .route("/api/agent/runs/latest", get(latest_agent_run))
        .route("/api/agent/run", post(run_agent))
        .fallback_service(ServeDir::new(static_dir))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 7812));
    let url = format!("http://{addr}/");
    println!("GeneGIS Workbench at {url}");
    println!("Plugin root: {}", plugin_root.display());
    println!("Collab server: {server_url} (set GENEGIS_SERVER_URL to override)");

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

async fn load_initial_agent_run(agent_run_path: &PathBuf, server_url: &str) -> Option<AgentRun> {
    let agent_run_path = agent_run_path.clone();
    let server_url = server_url.to_string();

    tokio::task::spawn_blocking(move || {
        if let Ok(run) = pull_latest_agent_run(&server_url) {
            let _ = run.save_to_path(&agent_run_path);
            return Some(run);
        }

        AgentRun::load_from_path(&agent_run_path).ok()
    })
    .await
    .ok()
    .flatten()
}

async fn load_initial_collab(collab_path: &PathBuf, server_url: &str) -> (CollabSession, SyncStatus) {
    let collab_path = collab_path.clone();
    let server_url = server_url.to_string();

    tokio::task::spawn_blocking(move || {
        if let Ok(session) = pull_session(&server_url) {
            save_collab_session(&session, &collab_path);
            return (
                session,
                SyncStatus {
                    source: "server".into(),
                    server_url: server_url.clone(),
                    synced: true,
                    error: None,
                },
            );
        }

        if collab_path.is_file() {
            if let Ok(json) = std::fs::read_to_string(&collab_path) {
                if let Ok(session) = CollabSession::import_json(&json) {
                    return (
                        session,
                        SyncStatus {
                            source: "local".into(),
                            server_url: server_url.clone(),
                            synced: false,
                            error: Some(
                                "GeneGIS Server unreachable; using local collab file".into(),
                            ),
                        },
                    );
                }
            }
        }

        let automerge_path = automerge_path_for(&collab_path);
        if automerge_path.is_file() {
            if let Ok(bytes) = std::fs::read(&automerge_path) {
                if let Ok(session) = CollabSession::from_snapshot(&bytes) {
                    return (
                        session,
                        SyncStatus {
                            source: "local".into(),
                            server_url: server_url.clone(),
                            synced: false,
                            error: Some(
                                "GeneGIS Server unreachable; using local Automerge snapshot".into(),
                            ),
                        },
                    );
                }
            }
        }

        (
            CollabSession::demo_nagoya(),
            SyncStatus {
                source: "demo".into(),
                server_url,
                synced: false,
                error: Some("GeneGIS Server unreachable; using demo session".into()),
            },
        )
    })
    .await
    .expect("collab bootstrap")
}

fn save_collab_session(session: &CollabSession, path: &PathBuf) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = session.export_json() {
        let _ = std::fs::write(path, json);
    }
    let mut session = session.clone();
    let automerge_path = automerge_path_for(path);
    let _ = std::fs::write(automerge_path, session.snapshot_bytes());
}

fn automerge_path_for(json_path: &PathBuf) -> PathBuf {
    let ext = json_path
        .extension()
        .map(|ext| ext.to_string_lossy().into_owned())
        .unwrap_or_else(|| "json".into());
    json_path.with_extension(format!("{ext}.automerge"))
}

fn push_to_server(session: &CollabSession, server_url: &str) -> Result<(), String> {
    push_session(server_url, session).map(|_| ()).map_err(|err| err.to_string())
}

fn collab_response(session: &CollabSession, sync: &SyncStatus) -> CollabResponse {
    CollabResponse {
        ok: true,
        summary: session.summary_json().unwrap_or_else(|err| {
            serde_json::json!({ "error": err.to_string() })
        }),
        comments: session.comments_json().unwrap_or_else(|_| serde_json::json!([])),
        sync: CollabSyncMeta {
            source: sync.source.clone(),
            server_url: sync.server_url.clone(),
            synced: sync.synced,
            error: sync.error.clone(),
        },
    }
}

fn collab_error_response(
    session: Option<&CollabSession>,
    sync: &SyncStatus,
    message: &str,
) -> CollabResponse {
    CollabResponse {
        ok: false,
        summary: serde_json::json!({ "error": message }),
        comments: session
            .as_ref()
            .and_then(|value| value.comments_json().ok())
            .unwrap_or_else(|| serde_json::json!([])),
        sync: CollabSyncMeta {
            source: sync.source.clone(),
            server_url: sync.server_url.clone(),
            synced: sync.synced,
            error: Some(message.into()),
        },
    }
}

async fn collab_snapshot(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let session = state
        .collab
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let sync = state
        .sync
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    (StatusCode::OK, Json(collab_response(&session, &sync)))
}

async fn add_comment(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AddCommentRequest>,
) -> impl IntoResponse {
    let author = body.author.trim();
    let text = body.body.trim();
    if author.is_empty() || text.is_empty() {
        let sync = state
            .sync
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        return (
            StatusCode::BAD_REQUEST,
            Json(collab_error_response(None, &sync, "author and body are required")),
        );
    }

    let mut comment = MapComment::new(author, text);
    if let Some([lon, lat]) = body.map_anchor {
        comment = comment.with_map_anchor(lon, lat);
    }

    let snapshot = {
        let mut session = state
            .collab
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if let Err(err) = session.add_comment(comment) {
            let sync = state
                .sync
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            return (
                StatusCode::BAD_REQUEST,
                Json(collab_error_response(Some(&session), &sync, &err.to_string())),
            );
        }

        save_collab_session(&session, &state.collab_path);
        session.clone()
    };

    let server_url = state.server_url.clone();
    let push_snapshot = snapshot.clone();
    let push_result =
        tokio::task::spawn_blocking(move || push_to_server(&push_snapshot, &server_url))
            .await
            .unwrap_or_else(|err| Err(err.to_string()));

    let mut sync = state
        .sync
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    sync.source = "local".into();
    match push_result {
        Ok(()) => {
            sync.synced = true;
            sync.error = None;
        }
        Err(err) => {
            sync.synced = false;
            sync.error = Some(format!("Saved locally; server push failed: {err}"));
        }
    }

    (
        StatusCode::OK,
        Json(collab_response(&snapshot, &sync)),
    )
}

async fn sync_collab(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let collab_path = state.collab_path.clone();
    let server_url = state.server_url.clone();

    let pull_result = tokio::task::spawn_blocking(move || pull_session(&server_url))
        .await
        .unwrap_or_else(|err| Err(genegis_collab::CollabError::Remote(err.to_string())));

    match pull_result {
        Ok(session) => {
            save_collab_session(&session, &collab_path);
            let mut collab = state
                .collab
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *collab = session.clone();

            let mut sync = state
                .sync
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            sync.source = "server".into();
            sync.synced = true;
            sync.error = None;

            (StatusCode::OK, Json(collab_response(&session, &sync)))
        }
        Err(err) => {
            let session = state
                .collab
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let mut sync = state
                .sync
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            sync.synced = false;
            sync.error = Some(err.to_string());

            (
                StatusCode::BAD_GATEWAY,
                Json(collab_error_response(
                    Some(&session),
                    &sync,
                    &err.to_string(),
                )),
            )
        }
    }
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

async fn latest_agent_run(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match AgentRun::load_from_path(&state.agent_run_path) {
        Ok(run) => (
            StatusCode::OK,
            Json(AgentRunResponse {
                ok: true,
                error: None,
                run: Some(run),
            }),
        ),
        Err(_) => (
            StatusCode::OK,
            Json(AgentRunResponse {
                ok: true,
                error: None,
                run: None,
            }),
        ),
    }
}

async fn run_agent(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AgentRunRequest>,
) -> impl IntoResponse {
    let prompt = body.prompt.trim().to_string();
    if prompt.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(AgentRunResponse {
                ok: false,
                error: Some("prompt is required".into()),
                run: None,
            }),
        );
    }

    let agent_run_path = state.agent_run_path.clone();
    let server_url = state.server_url.clone();
    let collab_path = state.collab_path.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut run = AgentOrchestrator::new()
            .with_config(AgentRunConfig::rule_based_offline().with_link_collab_on_failure(true))
            .run(&prompt)?;

        if !run.verification_passed {
            link_agent_failure_comment(&mut run, &collab_path)?;
        }

        run.save_to_path(&agent_run_path)?;
        let _ = push_agent_run(&server_url, &run);
        Ok(run)
    })
    .await
    .unwrap_or_else(|err| Err(genegis_agent::AgentError::Message(err.to_string())));

    match result {
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
                run: AgentRun::load_from_path(&state.agent_run_path).ok(),
            }),
        ),
    }
}

fn link_agent_failure_comment(
    run: &mut AgentRun,
    collab_path: &PathBuf,
) -> Result<(), genegis_agent::AgentError> {
    let verify_step = run
        .steps
        .iter()
        .rev()
        .find(|step| step.role == AgentRole::Verifier)
        .ok_or_else(|| genegis_agent::AgentError::Message("missing verifier step".into()))?;
    let body = format!(
        "Workbench agent verification failed after {} attempt(s)",
        run.verify_attempts.max(1)
    );

    let mut session = load_collab_from_disk(collab_path);
    let comment = session
        .add_agent_comment(run.id, verify_step.id, "workbench", body)
        .map_err(|err| genegis_agent::AgentError::Message(err.to_string()))?;
    save_collab_session(&session, collab_path);
    run.collab_comment_ids.push(comment.id);
    Ok(())
}

fn load_collab_from_disk(collab_path: &PathBuf) -> CollabSession {
    if collab_path.is_file() {
        if let Ok(json) = std::fs::read_to_string(collab_path) {
            if let Ok(session) = CollabSession::import_json(&json) {
                return session;
            }
        }
    }
    CollabSession::demo_nagoya()
}

fn fallback_index() -> String {
    "<html><body><h1>GeneGIS Workbench</h1><p>Static UI not found.</p></body></html>"
        .into()
}
