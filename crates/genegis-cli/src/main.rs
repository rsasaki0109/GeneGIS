//! GeneGIS CLI — Phase 1: ask, workflow run, execute, export.

use genegis_ai::{PlannerBackend, PlannerConfig, plan_with_config};
use genegis_analysis::{
    default_nagoya_data_path, export_html_map, export_png_map, run_ask_pipeline_with_config,
    run_nagoya_population_density,
};
use genegis_query::verify_nagoya_densities;
use genegis_workflow::nagoya_population_density_template;
use std::env;
use std::path::{Path, PathBuf};
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(String::as_str) {
        None | Some("help") | Some("--help") | Some("-h") => print_help(),
        Some("version") | Some("--version") | Some("-V") => {
            println!("genegis {}", env!("CARGO_PKG_VERSION"));
        }
        Some("ask") => handle_ask(&args[2..]),
        Some("bench") => handle_bench(&args[2..]),
        Some("storage") => handle_storage(&args[2..]),
        Some("raster") => handle_raster(&args[2..]),
        Some("workflow") => handle_workflow(&args[2..]),
        Some(cmd) => {
            eprintln!("Unknown command: {cmd}");
            print_help();
            process::exit(1);
        }
    }
}

fn handle_ask(args: &[String]) {
    let plan_only = args.iter().any(|a| a == "--plan-only" || a == "--plan");
    let export_png = args.iter().any(|a| a == "--png");
    let export_html = !args.iter().any(|a| a == "--no-html");
    let planner_config = planner_config_from_args(args);
    let output = args
        .iter()
        .position(|a| a == "--output" || a == "-o")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);

    let prompt = collect_prompt(args);
    if prompt.is_empty() {
        eprintln!(
            "Usage: genegis ask \"名古屋市の人口密度を表示\" [--plan-only] [--planner rule|llm] [--html] [--png] [-o FILE]"
        );
        process::exit(1);
    }

    if plan_only {
        let plan = match plan_with_config(&prompt, &planner_config) {
            Ok(p) => p,
            Err(err) => {
                eprintln!("Intent error: {err}");
                process::exit(1);
            }
        };
        match serde_json::to_string_pretty(&plan) {
            Ok(json) => println!("{json}"),
            Err(err) => {
                eprintln!("Failed to serialize plan: {err}");
                process::exit(1);
            }
        }
        return;
    }

    let result = match run_ask_pipeline_with_config(&prompt, &planner_config) {
        Ok(r) => r,
        Err(err) => {
            eprintln!("Pipeline error: {err}");
            process::exit(1);
        }
    };

    eprintln!(
        "Intent resolved: {} (confidence {:.0}%)",
        result.workflow_id,
        result.confidence * 100.0
    );
    for note in &result.ambiguities {
        eprintln!("  note: {note}");
    }

    eprintln!("Workflow: {} steps", result.workflow_steps);
    eprintln!("DuckDB verification: passed");
    println!(
        "{}",
        serde_json::to_string_pretty(&result.summary).expect("json")
    );

    write_exports(
        export_html,
        export_png,
        output.as_deref(),
        &result.html,
        &result.png,
    );
}

fn write_exports(
    export_html: bool,
    export_png: bool,
    output: Option<&Path>,
    html: &str,
    png: &[u8],
) {
    if let Some(path) = output {
        match path.extension().and_then(|ext| ext.to_str()) {
            Some("png") => write_bytes(path, png, "PNG"),
            Some("html") | Some("htm") => write_bytes(path, html.as_bytes(), "HTML"),
            _ => {
                if export_png && !export_html {
                    write_bytes(path, png, "PNG");
                } else {
                    write_bytes(path, html.as_bytes(), "HTML");
                }
            }
        }
        return;
    }

    if export_html {
        write_bytes(Path::new("nagoya-density.html"), html.as_bytes(), "HTML");
    }
    if export_png {
        write_bytes(Path::new("nagoya-density.png"), png, "PNG");
    }
}

fn write_bytes(path: &Path, bytes: &[u8], label: &str) {
    if let Err(err) = std::fs::write(path, bytes) {
        eprintln!("Failed to write {label}: {err}");
        process::exit(1);
    }
    eprintln!("Wrote {}", path.display());
}

fn collect_prompt(args: &[String]) -> String {
    let mut parts = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--output" | "-o" | "--planner" => {
                i += 2;
            }
            arg if arg.starts_with('-') => {
                i += 1;
            }
            arg => {
                parts.push(arg);
                i += 1;
            }
        }
    }
    parts
        .join(" ")
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

fn planner_config_from_args(args: &[String]) -> PlannerConfig {
    let mut config = PlannerConfig::default();
    if let Some(index) = args.iter().position(|arg| arg == "--planner") {
        let Some(value) = args.get(index + 1) else {
            eprintln!("--planner requires rule or llm");
            process::exit(1);
        };
        match PlannerBackend::parse(value) {
            Some(backend) => config.backend = backend,
            None => {
                eprintln!("Unknown planner: {value} (expected rule or llm)");
                process::exit(1);
            }
        }
    }
    config
}

fn handle_bench(args: &[String]) {
    use genegis_testkit::{
        benchmark_pipeline, benchmark_render_mesh, run_all_benchmarks, BenchmarkReport,
        DEFAULT_ITERATIONS, DEFAULT_WARMUP,
    };

    let json_output = args.iter().any(|a| a == "--json");
    let mut warmup = DEFAULT_WARMUP;
    let mut iterations = DEFAULT_ITERATIONS;
    let mut target = "all";

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => i += 1,
            "--warmup" => {
                warmup = parse_u32_arg(args, i, "warmup");
                i += 2;
            }
            "--iterations" | "-n" => {
                iterations = parse_u32_arg(args, i, "iterations");
                i += 2;
            }
            "pipeline" | "render" | "all" => {
                target = args[i].as_str();
                i += 1;
            }
            arg => {
                eprintln!("Unknown bench argument: {arg}");
                print_bench_help();
                process::exit(1);
            }
        }
    }

    let result: Result<BenchmarkReport, String> = match target {
        "pipeline" => benchmark_pipeline(warmup, iterations)
            .map(|sample| BenchmarkReport {
                samples: vec![sample],
            })
            .map_err(|err| err.to_string()),
        "render" => benchmark_render_mesh(warmup, iterations)
            .map(|sample| BenchmarkReport {
                samples: vec![sample],
            })
            .map_err(|err| err.to_string()),
        "all" => run_all_benchmarks(warmup, iterations).map_err(|err| err.to_string()),
        _ => unreachable!(),
    };

    let report = match result {
        Ok(report) => report,
        Err(err) => {
            eprintln!("Benchmark error: {err}");
            process::exit(1);
        }
    };

    if json_output {
        match report.to_json_pretty() {
            Ok(json) => println!("{json}"),
            Err(err) => {
                eprintln!("Failed to serialize benchmark report: {err}");
                process::exit(1);
            }
        }
        return;
    }

    for sample in &report.samples {
        print_benchmark_sample(sample);
    }
}

fn parse_u32_arg(args: &[String], index: usize, label: &str) -> u32 {
    let Some(value) = args.get(index + 1) else {
        eprintln!("--{label} requires a positive integer");
        process::exit(1);
    };
    match value.parse::<u32>() {
        Ok(n) if n > 0 => n,
        _ => {
            eprintln!("--{label} requires a positive integer");
            process::exit(1);
        }
    }
}

fn print_benchmark_sample(sample: &genegis_testkit::BenchmarkSample) {
    eprintln!(
        "{} (warmup {}, iterations {}): median {:.2} ms, mean {:.2} ms, min {:.2} ms, max {:.2} ms",
        sample.name,
        sample.warmup,
        sample.iterations,
        sample.median_ns as f64 / 1_000_000.0,
        sample.mean_ns as f64 / 1_000_000.0,
        sample.min_ns as f64 / 1_000_000.0,
        sample.max_ns as f64 / 1_000_000.0,
    );
}

fn print_bench_help() {
    eprintln!(
        r#"Usage:
  genegis bench [pipeline|render|all] [--warmup N] [--iterations N] [--json]

Examples:
  genegis bench
  genegis bench pipeline --iterations 20
  genegis bench render --json
"#
    );
}

fn handle_storage(args: &[String]) {
    use genegis_storage::{fetch_asset, ByteRange};

    match args.first().map(String::as_str) {
        Some("fetch") => {
            let json_output = args.iter().any(|a| a == "--json");
            let mut range: Option<ByteRange> = None;
            let mut output: Option<PathBuf> = None;
            let mut url: Option<String> = None;

            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--json" => i += 1,
                    "--range" => {
                        let value = args.get(i + 1).map(String::as_str).unwrap_or("");
                        range = Some(ByteRange::parse(value).unwrap_or_else(|err| {
                            eprintln!("Invalid range: {err}");
                            process::exit(1);
                        }));
                        i += 2;
                    }
                    "--output" | "-o" => {
                        output = args.get(i + 1).map(PathBuf::from);
                        i += 2;
                    }
                    arg if arg.starts_with('-') => {
                        eprintln!("Unknown storage fetch flag: {arg}");
                        print_storage_help();
                        process::exit(1);
                    }
                    arg => {
                        url = Some(arg.to_string());
                        i += 1;
                    }
                }
            }

            let Some(url) = url else {
                eprintln!("Usage: genegis storage fetch URL [--range START-END] [--json] [-o FILE]");
                process::exit(1);
            };

            let result = match fetch_asset(&url, range.as_ref()) {
                Ok(result) => result,
                Err(err) => {
                    eprintln!("Storage error: {err}");
                    process::exit(1);
                }
            };

            if let Some(path) = output {
                write_bytes(&path, &result.bytes, "asset");
            }

            if json_output {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&result.summary_json()).expect("json")
                );
                return;
            }

            eprintln!(
                "Fetched {} bytes from {}{}",
                result.byte_len,
                url,
                range
                    .as_ref()
                    .map(|r| format!(" (range {})", r.header_value()))
                    .unwrap_or_default()
            );
            if let Some(status) = result.status {
                eprintln!("HTTP status: {status}");
            }
        }
        _ => {
            print_storage_help();
            process::exit(1);
        }
    }
}

fn print_storage_help() {
    eprintln!(
        r#"Usage:
  genegis storage fetch URL [--range START-END] [--json] [-o FILE]

Examples:
  genegis storage fetch https://example.com/data.tif --range 0-65535 --json
  genegis storage fetch /path/to/local.parquet -o /tmp/copy.parquet
"#
    );
}

fn handle_raster(args: &[String]) {
    match args.first().map(String::as_str) {
        Some("info") => {
            let Some(path) = args.get(1) else {
                eprintln!("Usage: genegis raster info PATH");
                process::exit(1);
            };
            match genegis_raster::read_cog_uri(path) {
                Ok(info) => {
                    println!("{}", serde_json::to_string_pretty(&info.summary_json()).expect("json"));
                }
                Err(err) => {
                    eprintln!("Raster error: {err}");
                    process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("Usage: genegis raster info PATH");
            process::exit(1);
        }
    }
}

fn handle_workflow(args: &[String]) {
    match args.first().map(String::as_str) {
        Some("run") => {
            let name = args.get(1).map(String::as_str).unwrap_or("nagoya-density");
            let execute = args.iter().any(|a| a == "--execute" || a == "-x");
            let export_html = args.iter().any(|a| a == "--html");
            let export_png = args.iter().any(|a| a == "--png");
            let output = args
                .iter()
                .position(|a| a == "--output" || a == "-o")
                .and_then(|i| args.get(i + 1))
                .map(PathBuf::from);

            if name != "nagoya-density" {
                eprintln!("Unknown workflow: {name}");
                process::exit(1);
            }

            if execute {
                run_nagoya_execute(export_html, export_png, output.as_deref());
            } else {
                print_workflow_template();
            }
        }
        _ => {
            eprintln!("Usage: genegis workflow run [nagoya-density] [--execute] [--html] [--png] [-o FILE]");
            process::exit(1);
        }
    }
}

fn print_help() {
    println!(
        r#"GeneGIS CLI (Phase 1–3)

Usage:
  genegis ask "名古屋市の人口密度を表示"           Intent → execute + HTML map
  genegis ask "..." --plan-only                    Intent → workflow plan JSON
  genegis ask "..." --planner llm --plan-only        LLM planner (falls back to rules)
  genegis ask "..." --png                          Intent → execute + PNG map
  genegis ask "..." -o out.html                    Custom HTML output path
  genegis ask "..." -o out.png                     Custom PNG output path
  genegis bench [pipeline|render|all]              North-star performance benchmarks
  genegis bench pipeline --iterations 20 --json    JSON benchmark report
  genegis storage fetch URL [--range START-END]    HTTP range-read smoke fetch
  genegis raster info PATH                         COG / GeoTIFF metadata JSON (local or URL)
  genegis workflow run nagoya-density              Print workflow graph JSON
  genegis workflow run nagoya-density --execute    Run MVP analysis pipeline
  genegis workflow run nagoya-density -x --html    Execute + write HTML map
  genegis workflow run nagoya-density -x --png     Execute + write PNG map
  genegis version
  genegis help

LLM planner env: GENEGIS_LLM_API_KEY, GENEGIS_LLM_BASE_URL, GENEGIS_LLM_MODEL

North star: 「名古屋市の人口密度を表示」
"#
    );
}

fn print_workflow_template() {
    let workflow = nagoya_population_density_template();
    match serde_json::to_string_pretty(&workflow) {
        Ok(json) => println!("{json}"),
        Err(err) => {
            eprintln!("Failed to serialize workflow: {err}");
            process::exit(1);
        }
    }
}

fn run_nagoya_execute(export_html: bool, export_png: bool, output: Option<&Path>) {
    let data_path = default_nagoya_data_path();
    let result = match run_nagoya_population_density(data_path) {
        Ok(r) => r,
        Err(err) => {
            eprintln!("Analysis failed: {err}");
            process::exit(1);
        }
    };

    let rows: Vec<(String, u64, f64, f64)> = result
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

    match verify_nagoya_densities(&rows) {
        Ok(true) => eprintln!("DuckDB verification: passed"),
        Ok(false) => {
            eprintln!("DuckDB verification: failed");
            process::exit(1);
        }
        Err(err) => {
            eprintln!("DuckDB verification error: {err}");
            process::exit(1);
        }
    }

    let summary = serde_json::json!({
        "goal": result.workflow.goal,
        "ward_count": result.features.len(),
        "density_unit": result.verification.density_unit,
        "crs": result.verification.crs,
        "verification_passed": result.verification.checks.iter().all(|c| c.passed),
        "top_density_ward": result.features.iter()
            .max_by(|a, b| a.density_per_km2.partial_cmp(&b.density_per_km2).unwrap())
            .map(|f| serde_json::json!({
                "ward_name": f.ward_name,
                "density_per_km2": f.density_per_km2,
            })),
    });
    println!("{}", serde_json::to_string_pretty(&summary).expect("json"));

    let html = export_html_map(&result, "名古屋市 人口密度");
    let png = match export_png_map(&result, "名古屋市 人口密度") {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("PNG export failed: {err}");
            process::exit(1);
        }
    };
    write_exports(export_html, export_png, output, &html, &png);
}
