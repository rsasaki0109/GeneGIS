//! GPU-native rendering engine for GeneGIS (wgpu / WebGPU).

pub mod canvas;
pub mod choropleth;
pub mod city_scene;
mod gpu_benchmark;
pub mod scene3d;
pub mod tiled_lod;

pub use canvas::RenderCanvas;
pub use choropleth::{
    run_choropleth_window, ChoroplethFeature, ChoroplethGpu, ChoroplethMap, ChoroplethMesh,
    ChoroplethTiledGpu,
};
pub use city_scene::{
    plan_city_scene_frame, verify_city_scene_frame_plan, CityLayer, CityLayerKind,
    CitySceneFramePlan, CitySceneManifest, CityScenePlanError, CityStreamBudget, CityTile,
    SharedSpatialViewState,
};
pub use gpu_benchmark::{benchmark_headless_gpu, HeadlessGpuBenchmark};
pub use scene3d::{
    benchmark_scene3d_headless, run_scene3d_window, BuildingLod1, OrbitCamera, Scene3d,
    Scene3dBenchmark, Scene3dError, ScenePoi, SceneSource,
};
pub use tiled_lod::{lod_for_zoom, ChoroplethTileMesh, ChoroplethTiledLodMap, TiledLodConfig};

/// Phase 0 rendering capability marker.
pub const ENGINE: &str = "wgpu";
