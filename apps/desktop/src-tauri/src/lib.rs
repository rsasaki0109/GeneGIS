use genegis_agent::{
    AgentOrchestrator, AgentRun, AgentRunConfig, AgentRunSummary, DEFAULT_AGENT_RUN_PATH,
    DEFAULT_AGENT_RUNS_DIR,
};
use genegis_ai::{PlanResult, DEFAULT_AGENT_PLAN_PATH};
use genegis_analysis::{run_ask_pipeline, spawn_gpu_preview_for_workflow};
use genegis_collab::{CollabSession, MapComment};
use genegis_plugin_host::PluginHost;
use serde::Serialize;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Serialize)]
struct CollabSyncMeta {
    source: String,
    synced: bool,
    error: Option<String>,
}

#[derive(Serialize)]
struct PluginsResponse {
    ok: bool,
    error: Option<String>,
    plugin_root: String,
    plugins: Vec<serde_json::Value>,
}

#[derive(Serialize)]
struct CollabResponse {
    ok: bool,
    summary: serde_json::Value,
    comments: serde_json::Value,
    provenance: serde_json::Value,
    sync: CollabSyncMeta,
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

#[tauri::command]
fn run_ask(prompt: String) -> Result<genegis_analysis::AskPipelineResult, String> {
    run_ask_pipeline(prompt.trim()).map_err(|e| e.to_string())
}

#[tauri::command]
fn launch_gpu_preview(workflow_id: Option<String>) -> Result<String, String> {
    spawn_gpu_preview_for_workflow(workflow_id.as_deref().unwrap_or("nagoya-density"))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn list_plugins() -> Result<PluginsResponse, String> {
    let plugin_root = resolve_plugin_root();
    let host = PluginHost::new();
    match host.discover_plugins(&plugin_root) {
        Ok(entries) => {
            let plugins = entries
                .iter()
                .map(|entry| entry.summary_json())
                .collect();
            Ok(PluginsResponse {
                ok: true,
                error: None,
                plugin_root: plugin_root.display().to_string(),
                plugins,
            })
        }
        Err(err) => Ok(PluginsResponse {
            ok: false,
            error: Some(err.to_string()),
            plugin_root: plugin_root.display().to_string(),
            plugins: Vec::new(),
        }),
    }
}

#[tauri::command]
fn collab_snapshot() -> Result<CollabResponse, String> {
    let session = load_collab_session();
    Ok(collab_response(&session))
}

#[tauri::command]
fn collab_add_comment(author: String, body: String) -> Result<CollabResponse, String> {
    let author = author.trim();
    let body = body.trim();
    if author.is_empty() || body.is_empty() {
        return Err("author and body are required".into());
    }

    let mut session = load_collab_session();
    session
        .add_comment(MapComment::new(author, body))
        .map_err(|err| err.to_string())?;
    save_collab_session(&session)?;
    Ok(collab_response(&session))
}

#[tauri::command]
fn agent_runs_latest() -> Result<AgentRunResponse, String> {
    let path = PathBuf::from(DEFAULT_AGENT_RUN_PATH);
    if !path.is_file() {
        return Ok(AgentRunResponse {
            ok: true,
            error: None,
            run: None,
        });
    }
    let run = AgentRun::load_from_path(&path).map_err(|err| err.to_string())?;
    Ok(AgentRunResponse {
        ok: true,
        error: None,
        run: Some(run),
    })
}

#[tauri::command]
fn agent_runs_list() -> Result<AgentRunListResponse, String> {
    let runs = AgentRun::list_from_dir(DEFAULT_AGENT_RUNS_DIR).map_err(|err| err.to_string())?;
    Ok(AgentRunListResponse {
        ok: true,
        error: None,
        runs,
    })
}

#[tauri::command]
fn agent_run_get(id: String) -> Result<AgentRunResponse, String> {
    let id = Uuid::parse_str(id.trim()).map_err(|err| err.to_string())?;
    let run = AgentRun::load_from_runs_dir(DEFAULT_AGENT_RUNS_DIR, id).map_err(|err| err.to_string())?;
    Ok(AgentRunResponse {
        ok: true,
        error: None,
        run: Some(run),
    })
}

#[tauri::command]
fn agent_plan(prompt: String) -> Result<AgentRunResponse, String> {
    let prompt = prompt.trim().to_string();
    if prompt.is_empty() {
        return Err("prompt is required".into());
    }

    let mut run = AgentOrchestrator::new()
        .with_config(AgentRunConfig::rule_based_offline().plan_only())
        .run(&prompt)
        .map_err(|err| err.to_string())?;
    finalize_agent_run(&mut run)?;
    Ok(AgentRunResponse {
        ok: true,
        error: None,
        run: Some(run),
    })
}

#[tauri::command]
fn agent_execute() -> Result<AgentRunResponse, String> {
    let plan = PlanResult::load_from_path(DEFAULT_AGENT_PLAN_PATH).map_err(|err| err.to_string())?;
    let mut run = AgentOrchestrator::new()
        .with_config(AgentRunConfig::rule_based_offline())
        .execute_plan(plan)
        .map_err(|err| err.to_string())?;
    finalize_agent_run(&mut run)?;
    Ok(AgentRunResponse {
        ok: true,
        error: None,
        run: Some(run),
    })
}

#[tauri::command]
fn agent_retry() -> Result<AgentRunResponse, String> {
    agent_execute().or_else(|_| {
        let latest = AgentRun::load_from_path(DEFAULT_AGENT_RUN_PATH).map_err(|err| err.to_string())?;
        let mut run = AgentOrchestrator::new()
            .with_config(AgentRunConfig::rule_based_offline().with_verify_retries(2))
            .run(&latest.prompt)
            .map_err(|err| err.to_string())?;
        finalize_agent_run(&mut run)?;
        Ok(AgentRunResponse {
            ok: true,
            error: None,
            run: Some(run),
        })
    })
}

fn finalize_agent_run(run: &mut AgentRun) -> Result<(), String> {
    let collab_path = default_collab_path();
    let mut session = load_collab_session();
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
        .map_err(|err| err.to_string())?;
    save_collab_session(&session)?;

    run.save_to_path(DEFAULT_AGENT_RUN_PATH)
        .map_err(|err| err.to_string())?;
    let runs_dir = PathBuf::from(DEFAULT_AGENT_RUNS_DIR);
    if let Some(parent) = runs_dir.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::create_dir_all(&runs_dir);
    let _ = run.save_to_path(runs_dir.join(format!("{}.json", run.id)));
    Ok(())
}

fn collab_response(session: &CollabSession) -> CollabResponse {
    CollabResponse {
        ok: true,
        summary: session
            .summary_json()
            .unwrap_or_else(|err| serde_json::json!({ "error": err.to_string() })),
        comments: session.comments_json().unwrap_or_else(|_| serde_json::json!([])),
        provenance: session.provenance_json().unwrap_or_else(|_| serde_json::json!([])),
        sync: CollabSyncMeta {
            source: "local".into(),
            synced: true,
            error: None,
        },
    }
}

fn default_collab_path() -> PathBuf {
    PathBuf::from(".genegis/collab.json")
}

fn load_collab_session() -> CollabSession {
    let path = default_collab_path();
    if path.is_file() {
        if let Ok(json) = std::fs::read_to_string(&path) {
            if let Ok(session) = CollabSession::import_json(&json) {
                return session;
            }
        }
    }
    CollabSession::demo_nagoya()
}

fn save_collab_session(session: &CollabSession) -> Result<(), String> {
    let path = default_collab_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let json = session.export_json().map_err(|err| err.to_string())?;
    std::fs::write(&path, json).map_err(|err| err.to_string())?;
    Ok(())
}

fn resolve_plugin_root() -> PathBuf {
    let cwd_plugins = PathBuf::from("plugins");
    if cwd_plugins.is_dir() {
        return cwd_plugins;
    }

    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let repo_plugins = Path::new(&manifest_dir).join("../../../plugins");
        if repo_plugins.is_dir() {
            return repo_plugins;
        }
    }

    cwd_plugins
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            run_ask,
            launch_gpu_preview,
            list_plugins,
            collab_snapshot,
            collab_add_comment,
            agent_runs_latest,
            agent_runs_list,
            agent_run_get,
            agent_plan,
            agent_execute,
            agent_retry,
        ])
        .run(tauri::generate_context!())
        .expect("error while running GeneGIS desktop");
}
