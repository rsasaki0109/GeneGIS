use genegis_analysis::run_ask_pipeline;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
struct PlaygroundBundle<'a> {
    schema_version: u32,
    demo_id: &'static str,
    generated_by: &'static str,
    execution_mode: &'static str,
    prompt_aliases: [&'static str; 2],
    map_asset: &'static str,
    command: &'a genegis_core::CommandEnvelope,
    workflow: &'a genegis_workflow::GeoWorkflow,
    provenance: &'a genegis_core::ProvenanceStore,
    dataset: &'a genegis_catalog::DatasetRecord,
    stac_item: &'a genegis_catalog::StacItem,
    verification: &'a genegis_analysis::VerificationReport,
    summary: &'a serde_json::Value,
    confidence: f32,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("public/demo"));
    std::fs::create_dir_all(&output_dir)?;

    let result = run_ask_pipeline("名古屋市の人口密度を表示")?;
    let bundle = PlaygroundBundle {
        schema_version: 1,
        demo_id: "nagoya-density",
        generated_by: "genegis-analysis",
        execution_mode: "verified_replay",
        prompt_aliases: [
            "名古屋市の人口密度を表示",
            "Show population density in Nagoya",
        ],
        map_asset: "/demo/nagoya-density.png",
        command: &result.command,
        workflow: &result.workflow,
        provenance: &result.provenance,
        dataset: &result.dataset,
        stac_item: &result.stac_item,
        verification: &result.verification,
        summary: &result.summary,
        confidence: result.confidence,
    };

    write_json(&output_dir.join("nagoya-density.json"), &bundle)?;
    std::fs::write(output_dir.join("nagoya-density.png"), &result.png)?;
    println!(
        "Exported verified playground bundle to {}",
        output_dir.display()
    );
    Ok(())
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = serde_json::to_vec_pretty(value)?;
    std::fs::write(path, bytes)?;
    Ok(())
}
