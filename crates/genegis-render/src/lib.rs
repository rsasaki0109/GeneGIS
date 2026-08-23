//! GPU-native rendering engine for GeneGIS (wgpu / WebGPU).

pub mod canvas;
pub mod choropleth;
mod gpu_benchmark;
pub mod tiled_lod;

pub use canvas::RenderCanvas;
pub use choropleth::{
    run_choropleth_window, ChoroplethFeature, ChoroplethGpu, ChoroplethMap, ChoroplethMesh,
    ChoroplethTiledGpu,
};
pub use gpu_benchmark::{benchmark_headless_gpu, HeadlessGpuBenchmark};
pub use tiled_lod::{lod_for_zoom, ChoroplethTileMesh, ChoroplethTiledLodMap, TiledLodConfig};

/// Phase 0 rendering capability marker.
pub const ENGINE: &str = "wgpu";
