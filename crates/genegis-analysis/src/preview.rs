use genegis_catalog::{alpha_catalog, LOCAL_COG_DEMO_ID, NAGOYA_WARDS_DENSITY_ID, REMOTE_COG_DEMO_ID};
use genegis_geometry::PolygonRing;
use genegis_render::{ChoroplethMap, run_choropleth_window};
use genegis_style::ColorRgba;

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

/// Build a raster preview map from a COG pixel window (Phase 8 beta).
pub fn cog_raster_preview_map(uri: &str) -> Result<ChoroplethMap, AnalysisError> {
    let cols = 32u32;
    let rows = 32u32;
    let pixels = genegis_raster::read_cog_window_uri(uri, 0, 0, rows, cols)
        .map_err(|err| AnalysisError::Message(err.to_string()))?;

    let mut map = ChoroplethMap::default();
    let cell_w = 1.0 / cols as f64;
    let cell_h = 1.0 / rows as f64;

    for row in 0..rows {
        for col in 0..cols {
            let value = pixels[(row * cols + col) as usize];
            let g = value as f32 / 255.0;
            let color = ColorRgba::new(g, g * 0.85, g * 0.7, 1.0);
            let x0 = col as f64 * cell_w;
            let y0 = row as f64 * cell_h;
            let ring = PolygonRing::new(vec![
                (x0, y0),
                (x0 + cell_w, y0),
                (x0 + cell_w, y0 + cell_h),
                (x0, y0 + cell_h),
                (x0, y0),
            ]);
            map.push_feature(vec![ring], color);
        }
    }

    Ok(map)
}

/// Launch the native WebGPU choropleth preview on a background thread.
pub fn spawn_nagoya_gpu_preview() -> Result<(), AnalysisError> {
    spawn_gpu_map(nagoya_choropleth_map())
}

/// Launch a WebGPU raster preview for a COG URI.
pub fn spawn_cog_gpu_preview(uri: &str) -> Result<(), AnalysisError> {
    let uri = uri.to_string();
    spawn_gpu_map(cog_raster_preview_map(&uri))
}

/// Launch workflow-aware GPU preview (Nagoya choropleth or COG raster grid).
pub fn spawn_gpu_preview_for_workflow(workflow_id: &str) -> Result<String, AnalysisError> {
    match workflow_id {
        "nagoya-density" => {
            spawn_nagoya_gpu_preview()?;
            Ok("WebGPU choropleth preview launched".into())
        }
        "remote-cog-demo" => {
            let uri = alpha_catalog()
                .require(REMOTE_COG_DEMO_ID)
                .map_err(|err| AnalysisError::Message(err.to_string()))?
                .uri
                .clone();
            spawn_cog_gpu_preview(&uri)?;
            Ok("WebGPU raster preview launched (remote COG window)".into())
        }
        "local-cog-demo" => {
            let uri = alpha_catalog()
                .require(LOCAL_COG_DEMO_ID)
                .map_err(|err| AnalysisError::Message(err.to_string()))?
                .uri
                .clone();
            spawn_cog_gpu_preview(&uri)?;
            Ok("WebGPU raster preview launched (local COG window)".into())
        }
        other => Err(AnalysisError::Message(format!(
            "GPU preview not supported for workflow {other}"
        ))),
    }
}

fn spawn_gpu_map(build: Result<ChoroplethMap, AnalysisError>) -> Result<(), AnalysisError> {
    std::thread::Builder::new()
        .name("genegis-gpu-preview".into())
        .spawn(move || match build {
            Ok(map) => run_choropleth_window(map),
            Err(err) => eprintln!("GPU preview failed: {err}"),
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

    #[test]
    fn local_cog_preview_map_has_grid_cells() {
        let uri = alpha_catalog()
            .require(LOCAL_COG_DEMO_ID)
            .expect("record")
            .uri
            .clone();
        let map = cog_raster_preview_map(&uri).expect("map");
        assert_eq!(map.features.len(), 32 * 32);
    }
}
