//! Spatial analysis engine — workflow execution and operators.

pub mod error;
pub mod export;
pub mod nagoya;
pub mod pipeline;
pub mod preview;
pub mod result;

pub use error::AnalysisError;
pub use export::{export_html_map, export_map_svg, export_png_map, ExportError};
pub use nagoya::{
    default_nagoya_data_path, default_nagoya_dataset_id, run_nagoya_population_density,
    run_nagoya_population_density_for_dataset, run_nagoya_population_density_from_catalog,
};
pub use pipeline::{
    build_ask_result, execute_from_plan, run_analysis_for_plan, run_ask_pipeline,
    run_ask_pipeline_with_config, verify_analysis_densities, AskPipelineResult,
};
pub use preview::{nagoya_choropleth_map, spawn_nagoya_gpu_preview};
pub use result::{AnalysisResult, DensityFeature, VerificationCheck, VerificationReport};
