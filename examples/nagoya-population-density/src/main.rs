//! MVP demo runner — 「名古屋市の人口密度を表示」

use genegis_analysis::{
    default_nagoya_data_path, export_html_map, export_png_map, run_nagoya_population_density,
};
use genegis_query::verify_nagoya_densities;
use std::path::PathBuf;

fn main() {
    let data = default_nagoya_data_path();
    let result = run_nagoya_population_density(data).expect("analysis");

    let rows: Vec<_> = result
        .features
        .iter()
        .map(|f| {
            (
                f.ward_name.clone(),
                f.population,
                f.area_km2,
                f.density_per_km2,
            )
        })
        .collect();
    assert!(verify_nagoya_densities(&rows).expect("duckdb"));

    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("output/nagoya-density.html");
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).expect("mkdir");
    }
    std::fs::write(&out, export_html_map(&result, "名古屋市 人口密度")).expect("write html");

    let png_out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("output/nagoya-density.png");
    std::fs::write(&png_out, export_png_map(&result, "名古屋市 人口密度").expect("write png"))
        .expect("write png file");

    println!("GeneGIS MVP demo complete");
    println!("  wards: {}", result.features.len());
    println!("  verification: all checks passed");
    println!("  html: {}", out.display());
    println!("  png: {}", png_out.display());
    println!(
        "  workflow steps: {}",
        result.workflow.steps.len()
    );
}
