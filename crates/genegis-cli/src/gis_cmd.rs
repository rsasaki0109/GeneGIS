//! `genegis gis` — general-purpose GIS from the command line (RFC 0007).

use std::collections::BTreeMap;
use std::path::Path;

use genegis_toolkit::{
    export::{export, ExportFormat, MapOptions},
    import::ImportOptions,
    import_through_workflow, ops,
    planner::{self, LlmConfig, PlannerContext, PlannerMode},
    run_plan, Layer,
};

const USAGE: &str = "\
genegis gis — bring your own data (RFC 0007)

  genegis gis import <file> [--crs EPSG:…] [--encoding shift_jis] [--table name] [--out <file>]
      Import through Command + Workflow and print the receipt. --out converts
      (.geojson .csv .gpkg .parquet .pdf).

  genegis gis ask \"<question>\" --layer <file>[=EPSG:…] … [--point lon,lat]
                  [--mode auto|rule|llm] [--plan-only] [--out <file>]
      Plan the question over the given layers, execute it with verification,
      and print the receipt.

  genegis gis ops
      List the operation catalog.";

pub fn handle_gis(args: &[String]) {
    let result = match args.first().map(String::as_str) {
        Some("import") => import(&args[1..]),
        Some("ask") => ask(&args[1..]),
        Some("ops") => {
            for spec in ops::catalog() {
                println!("{:<20} {}  — {}", spec.name, spec.title, spec.description);
            }
            Ok(())
        }
        _ => {
            println!("{USAGE}");
            Ok(())
        }
    };
    if let Err(error) = result {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

fn load(path: &str, crs: Option<&str>, options: &ImportOptions) -> Result<Layer, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let mut options = options.clone();
    if let Some(crs) = crs {
        options.crs = Some(crs.to_string());
    }
    let filename = Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path);
    let (layer, receipt) =
        import_through_workflow(filename, &bytes, &options).map_err(|e| format!("{path}: {e}"))?;
    eprintln!(
        "imported {} → {} ({} features, {}, {:?}) workflow {}",
        path,
        layer.id(),
        layer.features.len(),
        layer.crs,
        layer.crs_status,
        receipt.workflow_digest
    );
    for note in &receipt.report.notes {
        eprintln!("  note: {note}");
    }
    if !receipt.report.skipped.is_empty() {
        eprintln!(
            "  skipped {} records (first: #{} {})",
            receipt.report.skipped.len(),
            receipt.report.skipped[0].record,
            receipt.report.skipped[0].reason
        );
    }
    Ok(layer)
}

fn write_out(layer: &Layer, out: &str) -> Result<(), String> {
    let lower = out.to_lowercase();
    let format = if lower.ends_with(".geojson") || lower.ends_with(".json") {
        ExportFormat::Geojson
    } else if lower.ends_with(".csv") {
        ExportFormat::Csv
    } else if lower.ends_with(".gpkg") {
        ExportFormat::Geopackage
    } else if lower.ends_with(".parquet") {
        ExportFormat::Geoparquet
    } else if lower.ends_with(".pdf") {
        ExportFormat::Pdf
    } else {
        return Err(format!(
            "{out}: use .geojson, .csv, .gpkg, .parquet, or .pdf"
        ));
    };
    let file = export(layer, format, &MapOptions::default()).map_err(|e| e.to_string())?;
    std::fs::write(out, &file.bytes).map_err(|e| format!("{out}: {e}"))?;
    eprintln!(
        "wrote {out} ({} bytes, {})",
        file.bytes.len(),
        file.receipt.output_sha256
    );
    Ok(())
}

fn import(args: &[String]) -> Result<(), String> {
    let path = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .ok_or("usage: genegis gis import <file>")?;
    let options = ImportOptions {
        encoding: flag(args, "--encoding").map(str::to_string),
        table: flag(args, "--table").map(str::to_string),
        ..Default::default()
    };
    let layer = load(path, flag(args, "--crs"), &options)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&layer.summary()).map_err(|e| e.to_string())?
    );
    if let Some(out) = flag(args, "--out") {
        write_out(&layer, out)?;
    }
    Ok(())
}

fn ask(args: &[String]) -> Result<(), String> {
    let prompt = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .ok_or("usage: genegis gis ask \"<question>\" --layer <file>")?;
    let mut layers = BTreeMap::new();
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--layer" {
            let spec = args.get(i + 1).ok_or("--layer needs a file")?;
            let (path, crs) = match spec.rsplit_once('=') {
                Some((path, crs)) if crs.to_ascii_uppercase().starts_with("EPSG:") => {
                    (path, Some(crs))
                }
                _ => (spec.as_str(), None),
            };
            let layer = load(path, crs, &ImportOptions::default())?;
            layers.insert(layer.id(), layer);
            i += 2;
        } else {
            i += 1;
        }
    }
    if layers.is_empty() {
        return Err("give at least one --layer <file>".into());
    }
    let point = flag(args, "--point")
        .map(|p| {
            let parts: Vec<f64> = p.split(',').filter_map(|v| v.trim().parse().ok()).collect();
            if parts.len() == 2 {
                Ok([parts[0], parts[1]])
            } else {
                Err(format!("--point expects lon,lat, got {p}"))
            }
        })
        .transpose()?;
    let mode = match flag(args, "--mode").unwrap_or("auto") {
        "rule" => PlannerMode::Rule,
        "llm" => PlannerMode::Llm,
        _ => PlannerMode::Auto,
    };
    let context = PlannerContext {
        point,
        selected_layer: None,
    };
    let planned = planner::plan(prompt, &layers, &context, mode, &LlmConfig::from_env())
        .map_err(|e| e.to_string())?;
    eprintln!(
        "planner: {} (confidence {:.2})",
        planned.backend, planned.confidence
    );
    for line in &planned.rationale {
        eprintln!("  · {line}");
    }
    for assumption in &planned.plan.assumptions {
        eprintln!("  assumption: {assumption}");
    }
    if args.iter().any(|a| a == "--plan-only") {
        println!(
            "{}",
            serde_json::to_string_pretty(&planned.plan).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    let run = run_plan(&planned.plan, &layers).map_err(|e| e.to_string())?;
    for step in &run.steps {
        eprintln!(
            "step {} ({}) → {} features in {}",
            step.id, step.op, step.feature_count, step.crs
        );
        for check in &step.checks {
            eprintln!(
                "  {} {}: {}",
                if check.passed { "✓" } else { "✗" },
                check.id,
                check.detail
            );
        }
    }
    if run.output.features.len() <= 20 {
        // Measured fields (with units) and names; source metadata stays in the receipt.
        let shown: Vec<_> = run
            .output
            .fields
            .iter()
            .filter(|f| f.unit.is_some() || f.name.ends_with("name") || f.name.ends_with("名"))
            .collect();
        for feature in &run.output.features {
            let values: Vec<String> = shown
                .iter()
                .map(|f| {
                    let value = feature
                        .properties
                        .get(&f.name)
                        .map(|v| v.to_string())
                        .unwrap_or_default();
                    format!(
                        "{}={}{}",
                        f.name,
                        value,
                        f.unit.as_ref().map(|u| format!(" {u}")).unwrap_or_default()
                    )
                })
                .collect();
            eprintln!("  {}", values.join("  "));
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&run.receipt()).map_err(|e| e.to_string())?
    );
    if let Some(out) = flag(args, "--out") {
        write_out(&run.output, out)?;
    }
    Ok(())
}
