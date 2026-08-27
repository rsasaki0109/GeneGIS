//! Local web workbench — serves GeneGIS UI and runs the ask pipeline via HTTP.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use genegis_adapter::{
    GazetteerEntry, GeocodingMode, GeocodingPrivacyPolicy, GeocodingProvider, GeocodingQuery,
    GeocodingRatePolicy, GeocodingRequest, OgcRequest,
};
use genegis_agent::{
    get_agent_run, list_agent_runs, pull_latest_agent_run, push_agent_run, AgentOrchestrator,
    AgentRole, AgentRun, AgentRunConfig, AgentRunSummary, DEFAULT_AGENT_RUNS_DIR,
    DEFAULT_AGENT_RUN_PATH,
};
use genegis_ai::{PlanResult, DEFAULT_AGENT_PLAN_PATH};
use genegis_analysis::{launch_scene3d_preview, run_ask_pipeline, spawn_gpu_preview_for_workflow};
use genegis_catalog::{
    bind_stac_item, browse_alpha_stac_collection, endpoint_registry_path, extended_catalog,
    fetch_stac_collection, import_stac_item_url, load_catalog_overlay, AssetRequirements,
    DatasetRecord, EndpointRegistry, StacSearchRequest,
};
use genegis_collab::{pull_session, push_session, CollabSession, MapComment, DEFAULT_SERVER_URL};
use genegis_core::{Command, CommandEnvelope, CommandOrigin};
use genegis_crs::{ChecksumVerification, Crs, SourceSnapshot};
use genegis_plugin_host::PluginHost;
use genegis_vector::{read_geoparquet_uri_with_options_and_policy, GeoParquetReadOptions};
use genegis_workflow::{
    federated_asset_execution_template, federated_stac_search_template,
    reviewed_workflow_templates, stac_endpoint_registry_template, ComposerCommand,
    WorkflowComposer,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::Mutex};
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use tower_http::{cors::CorsLayer, services::ServeDir};
use uuid::Uuid;

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
    agent_runs_dir: PathBuf,
    server_url: String,
    endpoint_registry_path: PathBuf,
    endpoint_registry: Arc<Mutex<EndpointRegistry>>,
    collab: Arc<Mutex<CollabSession>>,
    sync: Arc<Mutex<SyncStatus>>,
    composers: Arc<Mutex<BTreeMap<Uuid, ComposerSession>>>,
    verified_results: Arc<Mutex<BTreeMap<String, SourceSnapshot>>>,
}

#[derive(Clone)]
struct ComposerSession {
    template_id: String,
    reviewed_digest: String,
    composer: WorkflowComposer,
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
    provenance: serde_json::Value,
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

#[derive(Serialize)]
struct AgentRunListResponse {
    ok: bool,
    error: Option<String>,
    runs: Vec<AgentRunSummary>,
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

#[derive(Deserialize)]
struct GpuPreviewRequest {
    workflow_id: Option<String>,
    copc_path: Option<String>,
    buildings_path: Option<String>,
    crs: Option<String>,
}

#[derive(Serialize)]
struct StacCollectionResponse {
    ok: bool,
    error: Option<String>,
    collection: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct StacItemResponse {
    ok: bool,
    error: Option<String>,
    item: Option<genegis_catalog::StacItem>,
}

#[derive(Deserialize)]
struct StacUrlRequest {
    url: String,
}

#[derive(Serialize)]
struct StacFetchResponse {
    ok: bool,
    error: Option<String>,
    collection: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct StacImportResponse {
    ok: bool,
    error: Option<String>,
    record: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct StacOverlayResponse {
    ok: bool,
    error: Option<String>,
    records: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct EndpointAddRequest {
    id: String,
    title: Option<String>,
    url: String,
    auth_kind: Option<String>,
    auth_env: Option<String>,
    auth_header: Option<String>,
}

#[derive(Deserialize)]
struct EndpointRemoveRequest {
    id: String,
}

#[derive(Deserialize)]
struct FederatedSearchRequest {
    #[serde(default)]
    endpoint_ids: Vec<String>,
    bbox: Option<[f64; 4]>,
    datetime: Option<String>,
    #[serde(default)]
    collections: Vec<String>,
    limit: Option<usize>,
}

#[derive(Serialize)]
struct GpuPreviewResponse {
    ok: bool,
    error: Option<String>,
    message: Option<String>,
    dashboard: Option<genegis_analysis::LiveDashboard>,
}

#[derive(Serialize)]
struct OgcExecuteResponse {
    ok: bool,
    error: Option<String>,
    result: Option<genegis_analysis::OgcWorkflowResult>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum GeocodingProviderRequest {
    OfflineNagoya,
    HttpJson {
        provider_id: String,
        version: String,
        endpoint: String,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GeocodingExecuteRequest {
    mode: GeocodingMode,
    queries: Vec<GeocodingQuery>,
    language: String,
    max_candidates: u32,
    provider: GeocodingProviderRequest,
    privacy: GeocodingPrivacyPolicy,
    #[serde(default)]
    rate: GeocodingRatePolicy,
}

#[derive(Serialize)]
struct GeocodingExecuteResponse {
    ok: bool,
    error: Option<String>,
    result: Option<genegis_analysis::GeocodingWorkflowResult>,
}

#[derive(Serialize)]
struct NarrativeComposeResponse {
    ok: bool,
    error: Option<String>,
    receipt: Option<genegis_capsule::NarrativeCompositionReceipt>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NarrativeComposeRequest {
    title: String,
    result_digest: String,
    frames: Vec<genegis_capsule::NarrativeFrame>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LiveFeedExecuteRequest {
    request: genegis_adapter::LiveFeedRequest,
    #[serde(default)]
    policy: genegis_adapter::FeedFreshnessPolicy,
}

#[derive(Serialize)]
struct LiveFeedExecuteResponse {
    ok: bool,
    error: Option<String>,
    result: Option<genegis_analysis::LiveFeedWorkflowResult>,
}

#[derive(Serialize)]
struct OperationalDashboardResponse {
    ok: bool,
    error: Option<String>,
    receipt: Option<genegis_analysis::OperationalDashboardReceipt>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AlertEvaluateRequest {
    policy: genegis_analysis::VerifiedAlertPolicy,
    metric: genegis_analysis::AlertMetric,
    window: genegis_analysis::AlertTriggeringWindow,
    evaluated_at: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AlertAcknowledgeRequest {
    alert: genegis_analysis::VerifiedAlertRecord,
    acknowledgement: genegis_analysis::AlertAcknowledgement,
}

#[derive(Serialize)]
struct AlertResponse<T> {
    ok: bool,
    error: Option<String>,
    result: Option<T>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenarioCompareRequest {
    base: genegis_analysis::ScenarioBranch,
    scenario: genegis_analysis::ScenarioBranch,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenarioMergeRequest {
    base: genegis_analysis::ScenarioBranch,
    scenario: genegis_analysis::ScenarioBranch,
    approval: genegis_analysis::ScenarioApproval,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CityScenePlanRequest {
    manifest: genegis_render::CitySceneManifest,
    view: genegis_render::SharedSpatialViewState,
    budget: genegis_render::CityStreamBudget,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GovernanceExecuteRequest {
    state: genegis_collab::GovernanceState,
    operation: genegis_analysis::GovernanceOperation,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PrivateCatalogAdmissionRequest {
    endpoints: Vec<genegis_catalog::StacEndpoint>,
    policy: genegis_catalog::FederatedCatalogPolicy,
    context: genegis_catalog::CatalogAccessContext,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginRegistryExecuteRequest {
    registry: genegis_plugin_host::PluginRegistry,
    operation: genegis_plugin_host::PluginRegistryOperation,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SolutionPackAdmissionRequest {
    pack: genegis_plugin_host::SolutionPackManifest,
    registry: genegis_plugin_host::PluginRegistry,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PerformanceMatrixRequest {
    profile: genegis_core::PerformanceMatrixProfile,
    measurements: Vec<genegis_core::PerformanceMeasurement>,
}

#[derive(Deserialize)]
struct PublishRequest {
    capsule_path: String,
    slug: String,
}

#[derive(Serialize)]
struct PublishResponse {
    ok: bool,
    error: Option<String>,
    output_path: Option<String>,
    publication: Option<genegis_capsule::PortablePublication>,
}

#[derive(Serialize)]
struct BridgeCapsuleResponse {
    ok: bool,
    error: Option<String>,
    output_path: Option<String>,
    manifest: Option<genegis_capsule::BridgeCapsuleManifest>,
}

#[derive(Deserialize)]
struct ComposerCreateRequest {
    template_id: String,
}

#[derive(Deserialize)]
struct ComposerEditRequest {
    command: ComposerCommand,
}

#[derive(Serialize)]
struct ComposerResponse {
    ok: bool,
    error: Option<String>,
    session_id: Option<Uuid>,
    template_id: Option<String>,
    composer: Option<WorkflowComposer>,
}

#[derive(Serialize)]
struct ComposerRunResponse {
    ok: bool,
    error: Option<String>,
    command: Option<CommandEnvelope>,
    workflow: Option<genegis_workflow::GeoWorkflow>,
    result: Option<genegis_analysis::AskPipelineResult>,
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
    let agent_runs_dir = std::env::var("GENEGIS_AGENT_RUNS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_AGENT_RUNS_DIR));
    let server_url =
        std::env::var("GENEGIS_SERVER_URL").unwrap_or_else(|_| DEFAULT_SERVER_URL.into());
    let endpoint_registry_path = endpoint_registry_path();
    let endpoint_registry =
        EndpointRegistry::load(&endpoint_registry_path).unwrap_or_else(|error| {
            eprintln!("Endpoint registry warning: {error}");
            EndpointRegistry::default()
        });

    let (collab, sync) = load_initial_collab(&collab_path, &server_url).await;
    let agent_run_path_for_load = agent_run_path.clone();
    let server_url_for_agent = server_url.clone();
    let _ = load_initial_agent_run(&agent_run_path_for_load, &server_url_for_agent).await;
    let state = Arc::new(AppState {
        static_dir: static_dir.clone(),
        plugin_root: plugin_root.clone(),
        collab_path,
        agent_run_path,
        agent_runs_dir,
        server_url: server_url.clone(),
        endpoint_registry_path,
        endpoint_registry: Arc::new(Mutex::new(endpoint_registry)),
        collab: Arc::new(Mutex::new(collab)),
        sync: Arc::new(Mutex::new(sync)),
        composers: Arc::new(Mutex::new(BTreeMap::new())),
        verified_results: Arc::new(Mutex::new(BTreeMap::new())),
    });

    let app = Router::new()
        .route("/", get(index))
        .route("/api/ask", post(ask))
        .route("/api/gpu-preview", post(gpu_preview))
        .route("/api/ogc/execute", post(execute_ogc))
        .route("/api/geocode", post(execute_geocoding))
        .route("/api/narratives/compose", post(compose_narrative))
        .route("/api/live/ingest", post(execute_live_feed))
        .route("/api/operational/views", post(compose_operational_view))
        .route("/api/alerts/evaluate", post(evaluate_alert))
        .route("/api/alerts/acknowledge", post(acknowledge_alert))
        .route("/api/scenarios/branches", post(create_scenario))
        .route("/api/scenarios/compare", post(compare_scenario))
        .route("/api/scenarios/merge", post(merge_scenario))
        .route("/api/city-scene/plan", post(plan_city_scene))
        .route("/api/governance/execute", post(execute_governance))
        .route("/api/catalogs/private/admit", post(admit_private_catalog))
        .route(
            "/api/plugins/registry/execute",
            post(execute_plugin_registry),
        )
        .route("/api/solution-packs/admit", post(admit_solution_pack))
        .route(
            "/api/performance-matrix/evaluate",
            post(evaluate_performance_matrix),
        )
        .route("/api/publish", post(publish_capsule))
        .route("/api/bridge/capsule", post(create_bridge_capsule))
        .route("/api/composer/templates", get(composer_templates))
        .route("/api/composer/sessions", post(create_composer_session))
        .route("/api/composer/sessions/{id}", get(get_composer_session))
        .route(
            "/api/composer/sessions/{id}/edit",
            post(edit_composer_session),
        )
        .route(
            "/api/composer/sessions/{id}/run",
            post(run_composer_session),
        )
        .route("/api/stac/collection", get(stac_collection))
        .route("/api/stac/items/{id}", get(stac_item))
        .route("/api/stac/overlay", get(stac_overlay))
        .route("/api/stac/fetch", post(stac_fetch))
        .route("/api/stac/import", post(stac_import))
        .route(
            "/api/stac/endpoints",
            get(stac_endpoints).post(stac_endpoint_add),
        )
        .route("/api/stac/endpoints/remove", post(stac_endpoint_remove))
        .route("/api/stac/search", post(federated_stac_search))
        .route("/api/stac/execute", post(execute_federated_stac_asset))
        .route("/api/plugins", get(list_plugins))
        .route("/api/collab", get(collab_snapshot))
        .route("/api/collab/comment", post(add_comment))
        .route("/api/collab/sync", post(sync_collab))
        .route("/api/agent/runs/latest", get(latest_agent_run))
        .route("/api/agent/runs/{id}", get(get_agent_run_by_id))
        .route("/api/agent/runs", get(list_agent_runs_handler))
        .route("/api/agent/run", post(run_agent))
        .route("/api/agent/plan", post(plan_agent))
        .route("/api/agent/execute", post(execute_agent))
        .route("/api/agent/retry", post(retry_agent))
        .fallback_service(ServeDir::new(static_dir))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 7812));
    let url = format!("http://{addr}/");
    println!("GeneGIS Workbench at {url}");
    println!("Plugin root: {}", plugin_root.display());
    println!("Collab server: {server_url} (set GENEGIS_SERVER_URL to override)");

    if std::env::var_os("GENEGIS_NO_OPEN").is_none() && open::that(&url).is_err() {
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

async fn load_initial_collab(
    collab_path: &PathBuf,
    server_url: &str,
) -> (CollabSession, SyncStatus) {
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
    push_session(server_url, session)
        .map(|_| ())
        .map_err(|err| err.to_string())
}

fn collab_response(session: &CollabSession, sync: &SyncStatus) -> CollabResponse {
    CollabResponse {
        ok: true,
        summary: session
            .summary_json()
            .unwrap_or_else(|err| serde_json::json!({ "error": err.to_string() })),
        comments: session
            .comments_json()
            .unwrap_or_else(|_| serde_json::json!([])),
        provenance: session
            .provenance_json()
            .unwrap_or_else(|_| serde_json::json!([])),
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
        provenance: session
            .as_ref()
            .and_then(|value| value.provenance_json().ok())
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
            Json(collab_error_response(
                None,
                &sync,
                "author and body are required",
            )),
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
                Json(collab_error_response(
                    Some(&session),
                    &sync,
                    &err.to_string(),
                )),
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

    (StatusCode::OK, Json(collab_response(&snapshot, &sync)))
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
    Html(
        tokio::fs::read_to_string(path)
            .await
            .unwrap_or_else(|_| fallback_index()),
    )
}

async fn ask(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AskRequest>,
) -> impl IntoResponse {
    match run_ask_pipeline(body.prompt.trim()) {
        Ok(result) => {
            if result.execution_receipt.verification_passed {
                let digest = result.execution_receipt.result_digest.clone();
                let mut source = SourceSnapshot::new(format!(
                    "command://{}/result",
                    result.execution_receipt.command_id
                ));
                source.dataset_id = Some(format!("result:{digest}"));
                source.checksum = Some(digest.clone());
                source.expected_checksum = Some(digest.clone());
                source.observed_checksum = Some(digest.clone());
                source.checksum_status = ChecksumVerification::Verified;
                state
                    .verified_results
                    .lock()
                    .expect("verified result lock")
                    .insert(digest, source);
            }
            (
                StatusCode::OK,
                Json(AskResponse {
                    ok: true,
                    error: None,
                    result: Some(result),
                }),
            )
        }
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

async fn gpu_preview(Json(body): Json<GpuPreviewRequest>) -> impl IntoResponse {
    let workflow_id = body.workflow_id.as_deref().unwrap_or("nagoya-density");
    let launch: Result<(String, Option<genegis_analysis::LiveDashboard>), String> =
        if workflow_id == "scene3d-copc-lod1" {
            let copc_path = body
                .copc_path
                .as_deref()
                .ok_or_else(|| "scene3d-copc-lod1 requires copc_path".to_string());
            let buildings_path = body
                .buildings_path
                .as_deref()
                .ok_or_else(|| "scene3d-copc-lod1 requires buildings_path".to_string());
            let crs = body
                .crs
                .as_deref()
                .ok_or_else(|| "scene3d-copc-lod1 requires crs".to_string())
                .and_then(|value| Crs::parse(value).map_err(|error| error.to_string()));
            match (copc_path, buildings_path, crs) {
                (Ok(copc_path), Ok(buildings_path), Ok(crs)) => {
                    launch_scene3d_preview(copc_path, buildings_path, crs)
                        .map(|receipt| {
                            let message = format!(
                                "WebGPU 3D scene launched: {} points, {} buildings, {} POIs",
                                receipt.point_count, receipt.building_count, receipt.poi_count
                            );
                            (message, Some(receipt.dashboard))
                        })
                        .map_err(|error| error.to_string())
                }
                (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => Err(error),
            }
        } else {
            spawn_gpu_preview_for_workflow(workflow_id)
                .map(|message| (message, None))
                .map_err(|error| error.to_string())
        };
    match launch {
        Ok((message, dashboard)) => (
            StatusCode::OK,
            Json(GpuPreviewResponse {
                ok: true,
                error: None,
                message: Some(message),
                dashboard,
            }),
        ),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(GpuPreviewResponse {
                ok: false,
                error: Some(err),
                message: None,
                dashboard: None,
            }),
        ),
    }
}

async fn execute_ogc(Json(request): Json<OgcRequest>) -> impl IntoResponse {
    let execution = tokio::task::spawn_blocking(move || {
        genegis_analysis::execute_ogc_workflow(
            request,
            genegis_storage::RemoteAccessPolicy::default(),
        )
    })
    .await;
    match execution {
        Ok(Ok(result)) => (
            StatusCode::OK,
            Json(OgcExecuteResponse {
                ok: true,
                error: None,
                result: Some(result),
            }),
        ),
        Ok(Err(error)) => (
            StatusCode::BAD_REQUEST,
            Json(OgcExecuteResponse {
                ok: false,
                error: Some(error.to_string()),
                result: None,
            }),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(OgcExecuteResponse {
                ok: false,
                error: Some(error.to_string()),
                result: None,
            }),
        ),
    }
}

async fn execute_geocoding(Json(body): Json<GeocodingExecuteRequest>) -> impl IntoResponse {
    let provider = match body.provider {
        GeocodingProviderRequest::OfflineNagoya => nagoya_gazetteer_provider(),
        GeocodingProviderRequest::HttpJson {
            provider_id,
            version,
            endpoint,
        } => GeocodingProvider::HttpJson {
            provider_id,
            version,
            endpoint,
            remote_policy: genegis_storage::RemoteAccessPolicy::default(),
        },
    };
    let request = GeocodingRequest {
        mode: body.mode,
        queries: body.queries,
        language: body.language,
        max_candidates: body.max_candidates,
    };
    let execution = tokio::task::spawn_blocking(move || {
        genegis_analysis::execute_geocoding_workflow(request, provider, body.privacy, body.rate)
    })
    .await;
    match execution {
        Ok(Ok(result)) => (
            StatusCode::OK,
            Json(GeocodingExecuteResponse {
                ok: true,
                error: None,
                result: Some(result),
            }),
        ),
        Ok(Err(error)) => (
            StatusCode::BAD_REQUEST,
            Json(GeocodingExecuteResponse {
                ok: false,
                error: Some(error.to_string()),
                result: None,
            }),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(GeocodingExecuteResponse {
                ok: false,
                error: Some(error.to_string()),
                result: None,
            }),
        ),
    }
}

async fn compose_narrative(
    State(state): State<Arc<AppState>>,
    Json(request): Json<NarrativeComposeRequest>,
) -> impl IntoResponse {
    let source = state
        .verified_results
        .lock()
        .expect("verified result lock")
        .get(&request.result_digest)
        .cloned();
    let Some(result_source) = source else {
        return (
            StatusCode::CONFLICT,
            Json(NarrativeComposeResponse {
                ok: false,
                error: Some("result digest is not verified in this Workbench session".into()),
                receipt: None,
            }),
        );
    };
    let draft = genegis_capsule::NarrativeProjectViewDraft {
        title: request.title,
        result_digest: request.result_digest,
        result_source,
        frames: request.frames,
    };
    let composition =
        tokio::task::spawn_blocking(move || genegis_capsule::compose_narrative_project_view(draft))
            .await;
    match composition {
        Ok(Ok(receipt)) => (
            StatusCode::OK,
            Json(NarrativeComposeResponse {
                ok: true,
                error: None,
                receipt: Some(receipt),
            }),
        ),
        Ok(Err(error)) => (
            StatusCode::BAD_REQUEST,
            Json(NarrativeComposeResponse {
                ok: false,
                error: Some(error.to_string()),
                receipt: None,
            }),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(NarrativeComposeResponse {
                ok: false,
                error: Some(error.to_string()),
                receipt: None,
            }),
        ),
    }
}

async fn execute_live_feed(Json(body): Json<LiveFeedExecuteRequest>) -> impl IntoResponse {
    let execution = tokio::task::spawn_blocking(move || {
        genegis_analysis::execute_live_feed_workflow(
            body.request,
            body.policy,
            genegis_storage::RemoteAccessPolicy::default(),
        )
    })
    .await;
    match execution {
        Ok(Ok(result)) => (
            StatusCode::OK,
            Json(LiveFeedExecuteResponse {
                ok: true,
                error: None,
                result: Some(result),
            }),
        ),
        Ok(Err(error)) => (
            StatusCode::BAD_REQUEST,
            Json(LiveFeedExecuteResponse {
                ok: false,
                error: Some(error.to_string()),
                result: None,
            }),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(LiveFeedExecuteResponse {
                ok: false,
                error: Some(error.to_string()),
                result: None,
            }),
        ),
    }
}

async fn compose_operational_view(
    Json(draft): Json<genegis_analysis::OperationalDashboardDraft>,
) -> impl IntoResponse {
    let execution =
        tokio::task::spawn_blocking(move || genegis_analysis::compose_operational_dashboard(draft))
            .await;
    match execution {
        Ok(Ok(receipt)) => (
            StatusCode::OK,
            Json(OperationalDashboardResponse {
                ok: true,
                error: None,
                receipt: Some(receipt),
            }),
        ),
        Ok(Err(error)) => (
            StatusCode::BAD_REQUEST,
            Json(OperationalDashboardResponse {
                ok: false,
                error: Some(error.to_string()),
                receipt: None,
            }),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(OperationalDashboardResponse {
                ok: false,
                error: Some(error.to_string()),
                receipt: None,
            }),
        ),
    }
}

async fn evaluate_alert(Json(body): Json<AlertEvaluateRequest>) -> impl IntoResponse {
    let execution = tokio::task::spawn_blocking(move || {
        genegis_analysis::evaluate_verified_alert(
            body.policy,
            body.metric,
            body.window,
            body.evaluated_at,
        )
    })
    .await;
    match execution {
        Ok(Ok(result)) => (
            StatusCode::OK,
            Json(AlertResponse {
                ok: true,
                error: None,
                result: Some(result),
            }),
        ),
        Ok(Err(error)) => (
            StatusCode::BAD_REQUEST,
            Json(AlertResponse {
                ok: false,
                error: Some(error.to_string()),
                result: None,
            }),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(AlertResponse {
                ok: false,
                error: Some(error.to_string()),
                result: None,
            }),
        ),
    }
}

async fn acknowledge_alert(Json(body): Json<AlertAcknowledgeRequest>) -> impl IntoResponse {
    let execution = tokio::task::spawn_blocking(move || {
        genegis_analysis::acknowledge_verified_alert(body.alert, body.acknowledgement)
    })
    .await;
    match execution {
        Ok(Ok(result)) => (
            StatusCode::OK,
            Json(AlertResponse {
                ok: true,
                error: None,
                result: Some(result),
            }),
        ),
        Ok(Err(error)) => (
            StatusCode::BAD_REQUEST,
            Json(AlertResponse {
                ok: false,
                error: Some(error.to_string()),
                result: None,
            }),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(AlertResponse {
                ok: false,
                error: Some(error.to_string()),
                result: None,
            }),
        ),
    }
}

async fn create_scenario(
    Json(draft): Json<genegis_analysis::ScenarioBranchDraft>,
) -> impl IntoResponse {
    match genegis_analysis::create_scenario_branch(draft) {
        Ok(result) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok": true, "result": result})),
        ),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok": false, "error": error.to_string()})),
        ),
    }
}

async fn compare_scenario(Json(body): Json<ScenarioCompareRequest>) -> impl IntoResponse {
    match genegis_analysis::compare_scenarios(body.base, body.scenario) {
        Ok(result) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok": true, "result": result})),
        ),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok": false, "error": error.to_string()})),
        ),
    }
}

async fn merge_scenario(Json(body): Json<ScenarioMergeRequest>) -> impl IntoResponse {
    match genegis_analysis::merge_reviewed_scenario(body.base, body.scenario, body.approval) {
        Ok(result) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok": true, "result": result})),
        ),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok": false, "error": error.to_string()})),
        ),
    }
}

async fn plan_city_scene(Json(body): Json<CityScenePlanRequest>) -> impl IntoResponse {
    let execution = tokio::task::spawn_blocking(move || {
        genegis_analysis::plan_city_scene_workflow(body.manifest, body.view, body.budget)
    })
    .await;
    match execution {
        Ok(Ok(result)) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok": true, "result": result})),
        ),
        Ok(Err(error)) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok": false, "error": error.to_string()})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok": false, "error": error.to_string()})),
        ),
    }
}

async fn execute_governance(Json(body): Json<GovernanceExecuteRequest>) -> impl IntoResponse {
    let execution = tokio::task::spawn_blocking(move || {
        genegis_analysis::execute_governance_operation(body.state, body.operation)
    })
    .await;
    match execution {
        Ok(Ok(result)) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok": true, "result": result})),
        ),
        Ok(Err(error)) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok": false, "error": error.to_string()})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok": false, "error": error.to_string()})),
        ),
    }
}

async fn admit_private_catalog(
    Json(body): Json<PrivateCatalogAdmissionRequest>,
) -> impl IntoResponse {
    let execution = tokio::task::spawn_blocking(move || {
        genegis_analysis::admit_private_catalog_workflow(body.endpoints, body.policy, body.context)
    })
    .await;
    match execution {
        Ok(Ok(result)) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok": true, "result": result})),
        ),
        Ok(Err(error)) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok": false, "error": error.to_string()})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok": false, "error": error.to_string()})),
        ),
    }
}

async fn execute_plugin_registry(
    Json(body): Json<PluginRegistryExecuteRequest>,
) -> impl IntoResponse {
    let execution = tokio::task::spawn_blocking(move || {
        genegis_plugin_host::execute_plugin_registry_operation(body.registry, body.operation)
    })
    .await;
    match execution {
        Ok(Ok(result)) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok": true, "result": result})),
        ),
        Ok(Err(error)) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok": false, "error": error.to_string()})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok": false, "error": error.to_string()})),
        ),
    }
}

async fn admit_solution_pack(Json(body): Json<SolutionPackAdmissionRequest>) -> impl IntoResponse {
    let execution = tokio::task::spawn_blocking(move || {
        genegis_plugin_host::admit_solution_pack_workflow(body.pack, body.registry)
    })
    .await;
    match execution {
        Ok(Ok(result)) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok": true, "result": result})),
        ),
        Ok(Err(error)) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok": false, "error": error.to_string()})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok": false, "error": error.to_string()})),
        ),
    }
}

async fn evaluate_performance_matrix(
    Json(body): Json<PerformanceMatrixRequest>,
) -> impl IntoResponse {
    let execution = tokio::task::spawn_blocking(move || {
        genegis_analysis::evaluate_performance_matrix_workflow(body.profile, body.measurements)
    })
    .await;
    match execution {
        Ok(Ok(result)) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok": true, "result": result})),
        ),
        Ok(Err(error)) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok": false, "error": error.to_string()})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok": false, "error": error.to_string()})),
        ),
    }
}

fn nagoya_gazetteer_provider() -> GeocodingProvider {
    let entries = vec![
        GazetteerEntry {
            feature_id: "station:nagoya".into(),
            label: "名古屋駅".into(),
            aliases: vec!["Nagoya Station".into(), "名駅".into()],
            longitude: 136.8815,
            latitude: 35.1709,
        },
        GazetteerEntry {
            feature_id: "landmark:nagoya-castle".into(),
            label: "名古屋城".into(),
            aliases: vec!["Nagoya Castle".into()],
            longitude: 136.8997,
            latitude: 35.1856,
        },
        GazetteerEntry {
            feature_id: "district:sakae".into(),
            label: "栄".into(),
            aliases: vec!["Sakae".into(), "名古屋市中区栄".into()],
            longitude: 136.9066,
            latitude: 35.1681,
        },
    ];
    let bytes = serde_json::to_vec(&entries).expect("bundled gazetteer must serialize");
    let digest = format!("sha256:{:x}", Sha256::digest(bytes));
    let mut source = SourceSnapshot::new("builtin://genegis/nagoya-gazetteer/v1");
    source.dataset_id = Some("genegis-nagoya-gazetteer-v1".into());
    source.license = Some("CC0-1.0".into());
    source.checksum = Some(digest.clone());
    source.expected_checksum = Some(digest.clone());
    source.observed_checksum = Some(digest);
    source.checksum_status = ChecksumVerification::Verified;
    GeocodingProvider::OfflineGazetteer {
        provider_id: "org.genegis.nagoya-gazetteer".into(),
        version: "1".into(),
        source,
        entries,
    }
}

async fn publish_capsule(Json(request): Json<PublishRequest>) -> impl IntoResponse {
    let slug = match portable_slug(&request.slug) {
        Ok(slug) => slug,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(PublishResponse {
                    ok: false,
                    error: Some(error),
                    output_path: None,
                    publication: None,
                }),
            )
        }
    };
    let source = PathBuf::from(request.capsule_path);
    let output = PathBuf::from(".genegis/publications").join(slug);
    let output_for_task = output.clone();
    let execution = tokio::task::spawn_blocking(move || {
        genegis_capsule::export_portable_publication(
            source,
            &output_for_task,
            &genegis_capsule::PublicationPolicy::default(),
        )
    })
    .await;
    match execution {
        Ok(Ok(publication)) => (
            StatusCode::OK,
            Json(PublishResponse {
                ok: true,
                error: None,
                output_path: Some(output.to_string_lossy().into_owned()),
                publication: Some(publication),
            }),
        ),
        Ok(Err(error)) => (
            StatusCode::BAD_REQUEST,
            Json(PublishResponse {
                ok: false,
                error: Some(error.to_string()),
                output_path: None,
                publication: None,
            }),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(PublishResponse {
                ok: false,
                error: Some(error.to_string()),
                output_path: None,
                publication: None,
            }),
        ),
    }
}

async fn create_bridge_capsule(
    Json(request): Json<genegis_capsule::DesktopBridgeRequest>,
) -> impl IntoResponse {
    let capsule_id = uuid::Uuid::new_v4().to_string();
    let output = PathBuf::from(".genegis/bridge-capsules").join(capsule_id);
    let output_for_task = output.clone();
    let execution = tokio::task::spawn_blocking(move || {
        genegis_capsule::seal_desktop_bridge_capsule(&request, &output_for_task)
    })
    .await;
    match execution {
        Ok(Ok(manifest)) => (
            StatusCode::OK,
            Json(BridgeCapsuleResponse {
                ok: true,
                error: None,
                output_path: Some(output.to_string_lossy().into_owned()),
                manifest: Some(manifest),
            }),
        ),
        Ok(Err(error)) => (
            StatusCode::BAD_REQUEST,
            Json(BridgeCapsuleResponse {
                ok: false,
                error: Some(error.to_string()),
                output_path: None,
                manifest: None,
            }),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(BridgeCapsuleResponse {
                ok: false,
                error: Some(error.to_string()),
                output_path: None,
                manifest: None,
            }),
        ),
    }
}

fn portable_slug(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 64
        || !value.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
    {
        return Err("slug must use 1-64 lowercase ASCII letters, digits, or hyphens".into());
    }
    Ok(value.into())
}

async fn composer_templates() -> impl IntoResponse {
    Json(serde_json::json!({
        "ok": true,
        "templates": reviewed_workflow_templates(),
    }))
}

async fn create_composer_session(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ComposerCreateRequest>,
) -> impl IntoResponse {
    match resolved_composer(&request.template_id) {
        Ok((composer, reviewed_digest)) => {
            let session_id = Uuid::new_v4();
            let response_composer = composer.clone();
            state.composers.lock().expect("composer lock").insert(
                session_id,
                ComposerSession {
                    template_id: request.template_id.clone(),
                    reviewed_digest,
                    composer,
                },
            );
            (
                StatusCode::CREATED,
                Json(ComposerResponse {
                    ok: true,
                    error: None,
                    session_id: Some(session_id),
                    template_id: Some(request.template_id),
                    composer: Some(response_composer),
                }),
            )
        }
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(ComposerResponse {
                ok: false,
                error: Some(error.to_string()),
                session_id: None,
                template_id: None,
                composer: None,
            }),
        ),
    }
}

async fn get_composer_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let session = state
        .composers
        .lock()
        .expect("composer lock")
        .get(&id)
        .cloned();
    match session {
        Some(session) => (
            StatusCode::OK,
            Json(ComposerResponse {
                ok: true,
                error: None,
                session_id: Some(id),
                template_id: Some(session.template_id),
                composer: Some(session.composer),
            }),
        ),
        None => composer_not_found(id),
    }
}

async fn edit_composer_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(request): Json<ComposerEditRequest>,
) -> impl IntoResponse {
    let mut sessions = state.composers.lock().expect("composer lock");
    let Some(session) = sessions.get_mut(&id) else {
        return composer_not_found(id);
    };
    match session.composer.apply(request.command) {
        Ok(_) => (
            StatusCode::OK,
            Json(ComposerResponse {
                ok: true,
                error: None,
                session_id: Some(id),
                template_id: Some(session.template_id.clone()),
                composer: Some(session.composer.clone()),
            }),
        ),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(ComposerResponse {
                ok: false,
                error: Some(error.to_string()),
                session_id: Some(id),
                template_id: Some(session.template_id.clone()),
                composer: Some(session.composer.clone()),
            }),
        ),
    }
}

async fn run_composer_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let session = state
        .composers
        .lock()
        .expect("composer lock")
        .get(&id)
        .cloned();
    let Some(session) = session else {
        return (
            StatusCode::NOT_FOUND,
            Json(ComposerRunResponse {
                ok: false,
                error: Some(format!("composer session {id} not found")),
                command: None,
                workflow: None,
                result: None,
            }),
        );
    };
    let workflow = match session.composer.workflow_for_execution() {
        Ok(workflow) => workflow,
        Err(error) => return composer_run_error(StatusCode::BAD_REQUEST, error.to_string()),
    };
    let digest = match workflow.stable_digest() {
        Ok(digest) => digest,
        Err(error) => return composer_run_error(StatusCode::BAD_REQUEST, error.to_string()),
    };
    if session.reviewed_digest != digest {
        return composer_run_error(
            StatusCode::CONFLICT,
            "edited graph is valid but has no reviewed executor; undo to the reviewed digest before running".into(),
        );
    }
    let Some(prompt) = composer_template_prompt(&session.template_id) else {
        return composer_run_error(
            StatusCode::NOT_IMPLEMENTED,
            format!("template {} has no workbench executor", session.template_id),
        );
    };
    match run_ask_pipeline(prompt) {
        Ok(result) => {
            let executed_digest = result.workflow.stable_digest().unwrap_or_default();
            if executed_digest != digest {
                return composer_run_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "composer and executed workflow digests differ".into(),
                );
            }
            (
                StatusCode::OK,
                Json(ComposerRunResponse {
                    ok: true,
                    error: None,
                    command: Some(result.command.clone()),
                    workflow: Some(result.workflow.clone()),
                    result: Some(result),
                }),
            )
        }
        Err(error) => composer_run_error(StatusCode::BAD_REQUEST, error.to_string()),
    }
}

fn resolved_composer(template_id: &str) -> Result<(WorkflowComposer, String), String> {
    let composer = if template_id == "nagoya-density" {
        let catalog = extended_catalog();
        let dataset = catalog
            .require(genegis_analysis::default_nagoya_dataset_id())
            .map_err(|error| error.to_string())?;
        let workflow = genegis_analysis::nagoya_population_density_workflow_for_dataset(dataset)
            .map_err(|error| error.to_string())?;
        WorkflowComposer::from_reviewed_workflow(workflow).map_err(|error| error.to_string())?
    } else {
        WorkflowComposer::from_template(template_id).map_err(|error| error.to_string())?
    };
    let digest = composer
        .workflow
        .stable_digest()
        .map_err(|error| error.to_string())?;
    Ok((composer, digest))
}

fn composer_not_found(id: Uuid) -> (StatusCode, Json<ComposerResponse>) {
    (
        StatusCode::NOT_FOUND,
        Json(ComposerResponse {
            ok: false,
            error: Some(format!("composer session {id} not found")),
            session_id: None,
            template_id: None,
            composer: None,
        }),
    )
}

fn composer_run_error(
    status: StatusCode,
    error: String,
) -> (StatusCode, Json<ComposerRunResponse>) {
    (
        status,
        Json(ComposerRunResponse {
            ok: false,
            error: Some(error),
            command: None,
            workflow: None,
            result: None,
        }),
    )
}

fn composer_template_prompt(template_id: &str) -> Option<&'static str> {
    match template_id {
        "nagoya-density" => Some("名古屋市の人口密度を表示"),
        "nagoya-geoparquet" => Some("名古屋 wards GeoParquet を検証"),
        "nagoya-geoparquet-density" => Some("名古屋 GeoParquet 人口密度を表示"),
        "local-cog-metadata" => Some("ローカル COG のメタデータを検証"),
        "external-stac" => Some("外部 STAC カタログを検証"),
        "dashboard-export" => Some("名古屋の人口密度ダッシュボードを出力"),
        "flood-exposure" => Some("名古屋の洪水曝露を分析"),
        "xmin-city" => Some("名古屋の徒歩圏を分析"),
        "evacuation" => Some("名古屋の避難所アクセスを分析"),
        "ndvi-timeseries" => Some("Sentinel NDVI 時系列を分析"),
        "pointcloud-change" => Some("2時期の点群変化を分析"),
        _ => None,
    }
}

async fn stac_collection() -> impl IntoResponse {
    let collection = browse_alpha_stac_collection(&extended_catalog());
    (
        StatusCode::OK,
        Json(StacCollectionResponse {
            ok: true,
            error: None,
            collection: Some(collection.summary_json()),
        }),
    )
}

async fn stac_item(Path(id): Path<String>) -> impl IntoResponse {
    match bind_stac_item(&extended_catalog(), &id) {
        Ok(item) => (
            StatusCode::OK,
            Json(StacItemResponse {
                ok: true,
                error: None,
                item: Some(item),
            }),
        ),
        Err(err) => (
            StatusCode::NOT_FOUND,
            Json(StacItemResponse {
                ok: false,
                error: Some(err.to_string()),
                item: None,
            }),
        ),
    }
}

async fn stac_overlay() -> impl IntoResponse {
    let records: Vec<serde_json::Value> = load_catalog_overlay()
        .iter()
        .map(DatasetRecord::summary_json)
        .collect();
    (
        StatusCode::OK,
        Json(StacOverlayResponse {
            ok: true,
            error: None,
            records,
        }),
    )
}

async fn stac_fetch(Json(body): Json<StacUrlRequest>) -> impl IntoResponse {
    match fetch_stac_collection(&body.url) {
        Ok(collection) => (
            StatusCode::OK,
            Json(StacFetchResponse {
                ok: true,
                error: None,
                collection: Some(collection.summary_json()),
            }),
        ),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(StacFetchResponse {
                ok: false,
                error: Some(err.to_string()),
                collection: None,
            }),
        ),
    }
}

async fn stac_import(Json(body): Json<StacUrlRequest>) -> impl IntoResponse {
    match import_stac_item_url(&body.url) {
        Ok(record) => (
            StatusCode::OK,
            Json(StacImportResponse {
                ok: true,
                error: None,
                record: Some(record.summary_json()),
            }),
        ),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(StacImportResponse {
                ok: false,
                error: Some(err.to_string()),
                record: None,
            }),
        ),
    }
}

async fn stac_endpoints(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let registry = state
        .endpoint_registry
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "ok": true,
            "path": state.endpoint_registry_path,
            "endpoints": registry.endpoints,
            "command_count": registry.command_history.len(),
            "workflow_count": registry.workflows.len(),
            "provenance_count": registry.provenance.entries.len(),
        })),
    )
}

async fn stac_endpoint_add(
    State(state): State<Arc<AppState>>,
    Json(body): Json<EndpointAddRequest>,
) -> impl IntoResponse {
    let id = body.id.trim().to_string();
    let url = body.url.trim().to_string();
    if id.is_empty() || url.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "ok": false,
                "error": "endpoint id and URL are required",
            })),
        );
    }
    let command = Command::RegisterStacEndpoint {
        endpoint_id: id.clone(),
        title: body.title.unwrap_or_else(|| id.clone()),
        url,
        auth_kind: body.auth_kind.unwrap_or_else(|| "anonymous".into()),
        auth_env: body.auth_env,
        auth_header: body.auth_header,
    };
    let mut registry = state
        .endpoint_registry
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let result = registry
        .apply(
            CommandEnvelope::new(CommandOrigin::Ui, command),
            stac_endpoint_registry_template("register", &id),
        )
        .and_then(|_| registry.save(&state.endpoint_registry_path));
    match result {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "ok": true,
                "endpoint": registry.get(&id),
            })),
        ),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "ok": false, "error": error.to_string() })),
        ),
    }
}

async fn stac_endpoint_remove(
    State(state): State<Arc<AppState>>,
    Json(body): Json<EndpointRemoveRequest>,
) -> impl IntoResponse {
    let id = body.id.trim().to_string();
    let mut registry = state
        .endpoint_registry
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let result = registry
        .apply(
            CommandEnvelope::new(
                CommandOrigin::Ui,
                Command::RemoveStacEndpoint {
                    endpoint_id: id.clone(),
                },
            ),
            stac_endpoint_registry_template("remove", &id),
        )
        .and_then(|_| registry.save(&state.endpoint_registry_path));
    match result {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({ "ok": true, "removed": id })),
        ),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "ok": false, "error": error.to_string() })),
        ),
    }
}

async fn federated_stac_search(
    State(state): State<Arc<AppState>>,
    Json(body): Json<FederatedSearchRequest>,
) -> impl IntoResponse {
    let request = StacSearchRequest {
        bbox: body.bbox,
        datetime: body.datetime,
        collections: body.collections,
        limit: body.limit,
    };
    let (catalog, endpoint_ids) = {
        let registry = state
            .endpoint_registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match registry.federated_catalog(&body.endpoint_ids) {
            Ok(catalog) => {
                let ids = catalog
                    .endpoints()
                    .iter()
                    .map(|endpoint| endpoint.id.clone())
                    .collect::<Vec<_>>();
                (catalog, ids)
            }
            Err(error) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "ok": false, "error": error.to_string() })),
                )
            }
        }
    };
    if endpoint_ids.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "ok": false, "error": "no endpoints configured" })),
        );
    }

    let envelope = CommandEnvelope::new(
        CommandOrigin::Ui,
        Command::SearchFederatedStac {
            endpoint_ids: endpoint_ids.clone(),
            bbox: request.bbox,
            datetime: request.datetime.clone(),
            collections: request.collections.clone(),
            limit: request.limit,
        },
    );
    let workflow = federated_stac_search_template(&endpoint_ids);
    let remote_policy = genegis_storage::RemoteAccessPolicy::default();
    let result = catalog.search_with_policy(&request, &remote_policy);
    let binding = result
        .compare_and_bind(&AssetRequirements {
            bbox: request.bbox,
            ..AssetRequirements::default()
        })
        .ok();
    let persisted = {
        let mut registry = state
            .endpoint_registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        registry
            .record_search(envelope, workflow, &result)
            .and_then(|_| registry.save(&state.endpoint_registry_path))
    };
    match persisted {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "ok": true,
                "result": result,
                "binding": binding,
                "remote_policy": remote_policy
            })),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "ok": false, "error": error.to_string() })),
        ),
    }
}

async fn execute_federated_stac_asset(
    State(state): State<Arc<AppState>>,
    Json(body): Json<FederatedSearchRequest>,
) -> impl IntoResponse {
    let request = StacSearchRequest {
        bbox: body.bbox,
        datetime: body.datetime,
        collections: body.collections,
        limit: body.limit,
    };
    let (catalog, endpoint_ids) = {
        let registry = state
            .endpoint_registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match registry.federated_catalog(&body.endpoint_ids) {
            Ok(catalog) => {
                let ids = catalog
                    .endpoints()
                    .iter()
                    .map(|endpoint| endpoint.id.clone())
                    .collect::<Vec<_>>();
                (catalog, ids)
            }
            Err(error) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "ok": false, "error": error.to_string() })),
                );
            }
        }
    };
    let remote_policy = genegis_storage::RemoteAccessPolicy::default();
    let search = catalog.search_with_policy(&request, &remote_policy);
    let binding = match search.compare_and_bind(&AssetRequirements {
        bbox: request.bbox,
        ..AssetRequirements::default()
    }) {
        Ok(binding) => binding,
        Err(error) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({
                    "ok": false,
                    "error": error.to_string(),
                    "search": search
                })),
            );
        }
    };
    let uri = &binding.selected.href;
    if !genegis_storage::is_remote_uri(uri) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({
                "ok": false,
                "error": "selected asset is not HTTP(S); Range Read execution requires a remote GeoParquet",
                "binding": binding,
            })),
        );
    }
    let command = CommandEnvelope::new(
        CommandOrigin::Ui,
        Command::BindStacAsset {
            stac_item_key: binding.selected.stac_item_key.clone(),
            asset_key: binding.selected.asset_key.clone(),
            source_endpoints: binding.selected.source_endpoints.clone(),
            href: uri.clone(),
            media_type: binding.selected.media_type.clone(),
            crs: binding.crs.clone(),
            units: binding.units.clone(),
            license: binding.license.clone(),
        },
    );
    let workflow = federated_asset_execution_template(
        &endpoint_ids,
        &binding.selected.stac_item_key,
        &binding.selected.asset_key,
        uri,
    );
    match read_geoparquet_uri_with_options_and_policy(
        uri,
        GeoParquetReadOptions {
            row_groups: Some(vec![0]),
        },
        remote_policy.clone(),
    ) {
        Ok(execution) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "ok": true,
                "command": command,
                "workflow": workflow,
                "binding": binding,
                "remote_policy": remote_policy,
                "execution": execution,
                "verification": {
                    "passed": execution.range_requests > 0
                        && execution.dataset.crs == binding.crs
                        && !execution.schema_fields.is_empty(),
                    "checks": {
                        "http_range_requests": execution.range_requests,
                        "schema_fields": execution.schema_fields.len(),
                        "crs_matches_binding": execution.dataset.crs == binding.crs,
                        "source_matches_binding": execution.source_uri == binding.selected.href,
                    }
                }
            })),
        ),
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "ok": false,
                "error": error.to_string(),
                "command": command,
                "workflow": workflow,
                "binding": binding,
                "remote_policy": remote_policy,
            })),
        ),
    }
}

async fn list_plugins(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let host = PluginHost::new();
    match host.discover_plugins(&state.plugin_root) {
        Ok(entries) => {
            let plugins = entries.iter().map(|entry| entry.summary_json()).collect();
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
        Ok(run) => agent_run_ok(run),
        Err(_) => agent_run_ok_empty(),
    }
}

async fn list_agent_runs_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let server_url = state.server_url.clone();
    let runs_dir = state.agent_runs_dir.clone();
    let result =
        tokio::task::spawn_blocking(move || list_agent_runs_for_workbench(&server_url, &runs_dir))
            .await
            .map_err(|err| err.to_string())
            .and_then(|inner| inner);

    match result {
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
                error: Some(err),
                runs: Vec::new(),
            }),
        ),
    }
}

async fn get_agent_run_by_id(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let server_url = state.server_url.clone();
    let runs_dir = state.agent_runs_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        get_agent_run_for_workbench(&server_url, &runs_dir, id)
    })
    .await
    .map_err(|err| err.to_string())
    .and_then(|inner| inner);

    match result {
        Ok(run) => agent_run_ok(run),
        Err(err) => (
            StatusCode::NOT_FOUND,
            Json(AgentRunResponse {
                ok: false,
                error: Some(err),
                run: None,
            }),
        ),
    }
}

fn list_agent_runs_for_workbench(
    server_url: &str,
    runs_dir: &PathBuf,
) -> Result<Vec<AgentRunSummary>, String> {
    list_agent_runs(server_url)
        .or_else(|_| AgentRun::list_from_dir(runs_dir).map_err(|err| err.to_string()))
}

fn get_agent_run_for_workbench(
    server_url: &str,
    runs_dir: &PathBuf,
    id: Uuid,
) -> Result<AgentRun, String> {
    get_agent_run(server_url, id)
        .or_else(|_| AgentRun::load_from_runs_dir(runs_dir, id).map_err(|err| err.to_string()))
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
        finalize_agent_run(&mut run, &agent_run_path, &collab_path, &server_url)
    })
    .await
    .unwrap_or_else(|err| Err(genegis_agent::AgentError::Message(err.to_string())));

    agent_run_response(result, &state.agent_run_path)
}

async fn plan_agent(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AgentRunRequest>,
) -> impl IntoResponse {
    let prompt = body.prompt.trim().to_string();
    if prompt.is_empty() {
        return agent_error_response("prompt is required", None);
    }

    let agent_run_path = state.agent_run_path.clone();
    let collab_path = state.collab_path.clone();
    let server_url = state.server_url.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut run = AgentOrchestrator::new()
            .with_config(AgentRunConfig::rule_based_offline().plan_only())
            .run(&prompt)?;
        finalize_agent_run(&mut run, &agent_run_path, &collab_path, &server_url)
    })
    .await
    .unwrap_or_else(|err| Err(genegis_agent::AgentError::Message(err.to_string())));

    agent_run_response(result, &state.agent_run_path)
}

async fn execute_agent(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let agent_run_path = state.agent_run_path.clone();
    let collab_path = state.collab_path.clone();
    let server_url = state.server_url.clone();
    let result = tokio::task::spawn_blocking(move || {
        let plan = PlanResult::load_from_path(DEFAULT_AGENT_PLAN_PATH)
            .map_err(|err| genegis_agent::AgentError::Message(err.to_string()))?;
        let mut run = AgentOrchestrator::new()
            .with_config(AgentRunConfig::rule_based_offline().with_link_collab_on_failure(true))
            .execute_plan(plan)?;
        finalize_agent_run(&mut run, &agent_run_path, &collab_path, &server_url)
    })
    .await
    .unwrap_or_else(|err| Err(genegis_agent::AgentError::Message(err.to_string())));

    agent_run_response(result, &state.agent_run_path)
}

async fn retry_agent(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let agent_run_path = state.agent_run_path.clone();
    let collab_path = state.collab_path.clone();
    let server_url = state.server_url.clone();
    let result = tokio::task::spawn_blocking(move || {
        retry_agent_run(&agent_run_path, &collab_path, &server_url)
    })
    .await
    .map_err(|err| genegis_agent::AgentError::Message(err.to_string()))
    .and_then(|inner| inner);

    agent_run_response(result, &state.agent_run_path)
}

fn retry_agent_run(
    agent_run_path: &PathBuf,
    collab_path: &PathBuf,
    server_url: &str,
) -> Result<AgentRun, genegis_agent::AgentError> {
    if let Ok(plan) = PlanResult::load_from_path(DEFAULT_AGENT_PLAN_PATH) {
        let mut run = AgentOrchestrator::new()
            .with_config(AgentRunConfig::rule_based_offline().with_link_collab_on_failure(true))
            .execute_plan(plan)?;
        return finalize_agent_run(&mut run, agent_run_path, collab_path, server_url);
    }

    let latest = AgentRun::load_from_path(agent_run_path)?;
    let mut run = AgentOrchestrator::new()
        .with_config(AgentRunConfig::rule_based_offline().with_link_collab_on_failure(true))
        .run(&latest.prompt)?;
    finalize_agent_run(&mut run, agent_run_path, collab_path, server_url)
}

fn agent_run_response(
    result: Result<AgentRun, genegis_agent::AgentError>,
    agent_run_path: &PathBuf,
) -> (StatusCode, Json<AgentRunResponse>) {
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
                run: AgentRun::load_from_path(agent_run_path).ok(),
            }),
        ),
    }
}

fn agent_error_response(
    message: &str,
    run: Option<AgentRun>,
) -> (StatusCode, Json<AgentRunResponse>) {
    (
        StatusCode::BAD_REQUEST,
        Json(AgentRunResponse {
            ok: false,
            error: Some(message.into()),
            run,
        }),
    )
}

fn finalize_agent_run(
    run: &mut AgentRun,
    agent_run_path: &PathBuf,
    collab_path: &PathBuf,
    server_url: &str,
) -> Result<AgentRun, genegis_agent::AgentError> {
    if !run.verification_passed && !run.plan_only {
        let _ = link_agent_failure_comment(run, collab_path);
    }

    let mut session = load_collab_from_disk(collab_path);
    session
        .record_agent_run_provenance(
            run.id,
            run.workflow_id.as_deref(),
            &run.planner_mode,
            run.plan_only,
            run.verification_passed,
            run.verify_attempts,
            &run.prompt,
        )
        .map_err(|err| genegis_agent::AgentError::Message(err.to_string()))?;
    save_collab_session(&session, collab_path);

    run.save_to_path(agent_run_path)?;
    let _ = push_agent_run(server_url, run);
    Ok(run.clone())
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
    "<html><body><h1>GeneGIS Workbench</h1><p>Static UI not found.</p></body></html>".into()
}
