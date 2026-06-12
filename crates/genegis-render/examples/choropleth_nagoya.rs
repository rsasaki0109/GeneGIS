//! WebGPU choropleth preview — Nagoya ward population density (Phase 2 alpha).

use genegis_analysis::{default_nagoya_data_path, run_nagoya_population_density};
use genegis_render::{ChoroplethMap, run_choropleth_window};

fn main() {
    let analysis = run_nagoya_population_density(default_nagoya_data_path()).expect("analysis");

    let mut map = ChoroplethMap::default();
    for feature in &analysis.features {
        map.push_feature(feature.rings.clone(), feature.color);
    }

    assert_eq!(map.features.len(), 16, "expected 16 Nagoya wards");
    run_choropleth_window(map);
}
