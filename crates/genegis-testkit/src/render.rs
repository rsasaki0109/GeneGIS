use genegis_analysis::nagoya_choropleth_map;
use genegis_render::{ChoroplethMap, ChoroplethMesh};

use crate::error::TestkitError;
use crate::harness::{summarize, time_iterations, BenchmarkSample, DEFAULT_VIEWPORT};

/// Benchmark CPU-side choropleth mesh triangulation (no GPU window).
pub fn benchmark_render_mesh(
    warmup: u32,
    iterations: u32,
) -> Result<BenchmarkSample, TestkitError> {
    let map = load_nagoya_map()?;
    let (width, height) = DEFAULT_VIEWPORT;

    let durations = time_iterations(warmup, iterations, || {
        let mesh = ChoroplethMesh::build(&map, width, height);
        if mesh.is_empty() {
            return Err(TestkitError::Render("empty choropleth mesh".into()));
        }
        Ok(())
    })?;

    Ok(summarize("render_mesh", warmup, &durations))
}

fn load_nagoya_map() -> Result<ChoroplethMap, TestkitError> {
    let map = nagoya_choropleth_map().map_err(|err| TestkitError::Render(err.to_string()))?;
    if map.features.is_empty() {
        return Err(TestkitError::Render("choropleth map has no features".into()));
    }
    Ok(map)
}
