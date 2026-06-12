use genegis_analysis::{run_ask_pipeline, spawn_nagoya_gpu_preview};

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![run_ask, launch_gpu_preview])
        .run(tauri::generate_context!())
        .expect("error while running GeneGIS desktop");
}
