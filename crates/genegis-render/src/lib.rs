//! GPU-native rendering engine for GeneGIS (wgpu / WebGPU).

pub mod canvas;
pub mod choropleth;
pub mod tiled_lod;

pub use canvas::RenderCanvas;
pub use choropleth::{
    ChoroplethFeature, ChoroplethGpu, ChoroplethMap, ChoroplethMesh, ChoroplethTiledGpu,
    run_choropleth_window,
};
pub use tiled_lod::{
    lod_for_zoom, ChoroplethTileMesh, ChoroplethTiledLodMap, TiledLodConfig,
};

/// Phase 0 rendering capability marker.
pub const ENGINE: &str = "wgpu";
