use genegis_render::{ChoroplethMap, run_choropleth_window};

use crate::error::AnalysisError;
use crate::nagoya::run_nagoya_population_density_from_catalog;

/// Build the Nagoya choropleth map for GPU rendering.
pub fn nagoya_choropleth_map() -> Result<ChoroplethMap, AnalysisError> {
    let analysis = run_nagoya_population_density_from_catalog()?;
    let mut map = ChoroplethMap::default();
    for feature in &analysis.features {
        map.push_feature(feature.rings.clone(), feature.color);
    }
    Ok(map)
}

/// Launch the native WebGPU choropleth preview on a background thread.
pub fn spawn_nagoya_gpu_preview() -> Result<(), AnalysisError> {
    std::thread::Builder::new()
        .name("genegis-gpu-preview".into())
        .spawn(|| {
            match nagoya_choropleth_map() {
                Ok(map) => run_choropleth_window(map),
                Err(err) => eprintln!("GPU preview failed: {err}"),
            }
        })
        .map_err(|err| AnalysisError::Message(format!("failed to spawn GPU preview: {err}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nagoya_choropleth_map_has_sixteen_wards() {
        let map = nagoya_choropleth_map().expect("map");
        assert_eq!(map.features.len(), 16);
    }
}
