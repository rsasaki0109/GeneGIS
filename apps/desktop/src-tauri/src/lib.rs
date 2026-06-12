use genegis_analysis::{run_ask_pipeline, spawn_nagoya_gpu_preview};
use genegis_plugin_host::PluginHost;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
struct PluginsResponse {
    ok: bool,
    error: Option<String>,
    plugin_root: String,
    plugins: Vec<serde_json::Value>,
}

#[tauri::command]
fn run_ask(prompt: String) -> Result<genegis_analysis::AskPipelineResult, String> {
    run_ask_pipeline(prompt.trim()).map_err(|e| e.to_string())
}

#[tauri::command]
fn launch_gpu_preview() -> Result<String, String> {
    spawn_nagoya_gpu_preview()
        .map(|()| "WebGPU choropleth preview launched".into())
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
            list_plugins
        ])
        .run(tauri::generate_context!())
        .expect("error while running GeneGIS desktop");
}
