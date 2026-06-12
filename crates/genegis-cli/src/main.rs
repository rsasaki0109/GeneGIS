//! GeneGIS CLI — Phase 1: ask, workflow run, execute, export.

use genegis_ai::{PlannerBackend, PlannerConfig, plan_with_config};
use genegis_analysis::{
    default_nagoya_data_path, export_html_map, export_png_map, run_ask_pipeline_with_config,
    run_nagoya_population_density,
};
use genegis_catalog::{alpha_catalog, REMOTE_COG_DEMO_ID};
use genegis_agent::{
    build_audit_bundle, get_agent_run, list_agent_runs, pull_latest_agent_run, push_agent_run,
    AgentOrchestrator, AgentRun, AgentRunConfig, AgentRole, AuditCollabSnapshot,
    DEFAULT_AGENT_RUN_PATH, DEFAULT_AGENT_RUNS_DIR, DEFAULT_SERVER_URL,
};
use genegis_ai::{PlanResult, DEFAULT_AGENT_PLAN_PATH};
use genegis_collab::{pull_session, push_session, CollabSession, MapComment};
use genegis_query::verify_nagoya_densities;
use genegis_workflow::{nagoya_population_density_template, remote_cog_metadata_template};
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
        Some("pointcloud") => handle_pointcloud(&args[2..]),
        Some("plugin") => handle_plugin(&args[2..]),
        Some("collab") => handle_collab(&args[2..]),
        Some("agent") => handle_agent(&args[2..]),
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

fn handle_pointcloud(args: &[String]) {
    match args.first().map(String::as_str) {
        Some("info") => {
            let Some(path) = args.get(1) else {
                eprintln!("Usage: genegis pointcloud info PATH|URL");
                process::exit(1);
            };
            match genegis_pointcloud::read_copc_uri(path) {
                Ok(info) => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&info.summary_json()).expect("json")
                    );
                }
                Err(err) => {
                    eprintln!("Point cloud error: {err}");
                    process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("Usage: genegis pointcloud info PATH|URL");
            process::exit(1);
        }
    }
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

fn default_plugin_root() -> PathBuf {
    let cwd_plugins = PathBuf::from("plugins");
    if cwd_plugins.is_dir() {
        return cwd_plugins;
    }

    if let Ok(manifest_dir) = env::var("CARGO_MANIFEST_DIR") {
        let repo_plugins = Path::new(&manifest_dir)
            .join("../../plugins");
        if repo_plugins.is_dir() {
            return repo_plugins;
        }
    }

    cwd_plugins
}

fn handle_plugin(args: &[String]) {
    let host = genegis_plugin_host::PluginHost::new();
    match args.first().map(String::as_str) {
        Some("list") => {
            let root = args
                .get(1)
                .map(PathBuf::from)
                .unwrap_or_else(default_plugin_root);
            match host.discover_plugins(&root) {
                Ok(entries) => {
                    let summaries: Vec<_> =
                        entries.iter().map(|entry| entry.summary_json()).collect();
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&summaries).expect("json")
                    );
                }
                Err(err) => {
                    eprintln!("Plugin error: {err}");
                    process::exit(1);
                }
            }
        }
        Some("info") => {
            let Some(bundle) = args.get(1) else {
                eprintln!("Usage: genegis plugin info BUNDLE_DIR");
                process::exit(1);
            };
            match host.discover_bundle(bundle) {
                Ok(entry) => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&entry.summary_json()).expect("json")
                    );
                }
                Err(err) => {
                    eprintln!("Plugin error: {err}");
                    process::exit(1);
                }
            }
        }
        Some("load") => {
            let Some(bundle) = args.get(1) else {
                eprintln!("Usage: genegis plugin load BUNDLE_DIR");
                process::exit(1);
            };
            match host.load_bundle(bundle) {
                Ok(loaded) => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "id": loaded.entry.manifest.id,
                            "bundle_dir": loaded.entry.bundle_dir,
                            "wasm_bytes": loaded.wasm_bytes.len(),
                            "effective_capabilities": loaded.entry.effective_capabilities,
                            "status": "loaded",
                        }))
                        .expect("json")
                    );
                }
                Err(err) => {
                    eprintln!("Plugin error: {err}");
                    process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("Usage: genegis plugin list [DIR]");
            eprintln!("       genegis plugin info BUNDLE_DIR");
            eprintln!("       genegis plugin load BUNDLE_DIR");
            process::exit(1);
        }
    }
}

fn default_collab_path() -> PathBuf {
    PathBuf::from(".genegis/collab.json")
}

fn default_server_url() -> String {
    std::env::var("GENEGIS_SERVER_URL").unwrap_or_else(|_| DEFAULT_SERVER_URL.into())
}

fn collab_input_path(args: &[String]) -> PathBuf {
    args.iter()
        .position(|a| a == "--input" || a == "-i")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(default_collab_path)
}

fn collab_output_path(args: &[String]) -> PathBuf {
    args.iter()
        .position(|a| a == "--output" || a == "-o")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(default_collab_path)
}

fn collab_server_url(args: &[String]) -> String {
    args.iter()
        .position(|a| a == "--url")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(default_server_url)
}

fn load_collab_session() -> CollabSession {
    let path = default_collab_path();
    if path.is_file() {
        if let Ok(json) = std::fs::read_to_string(&path) {
            if let Ok(session) = CollabSession::import_json(&json) {
                return session;
            }
        }
    }
    CollabSession::demo_nagoya()
}

fn save_collab_session(session: &CollabSession) {
    let path = default_collab_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match session.export_json() {
        Ok(json) => {
            if let Err(err) = std::fs::write(&path, json) {
                eprintln!("Warning: failed to write {}: {err}", path.display());
            }
        }
        Err(err) => eprintln!("Warning: failed to export collab session: {err}"),
    }
}

fn handle_agent(args: &[String]) {
    match args.first().map(String::as_str) {
        Some("run") => {
            let plan_only = args.iter().any(|a| a == "--plan-only" || a == "--plan");
            let json_output = args.iter().any(|a| a == "--json");
            let push_to_server = args.iter().any(|a| a == "--push");
            let link_collab = args.iter().any(|a| a == "--link-collab");
            let planner_config = planner_config_from_args(args);
            let verify_retries = args
                .iter()
                .position(|a| a == "--verify-retries")
                .and_then(|i| args.get(i + 1))
                .and_then(|value| value.parse().ok())
                .unwrap_or(2);
            let output = args
                .iter()
                .position(|a| a == "--output" || a == "-o")
                .and_then(|i| args.get(i + 1))
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(DEFAULT_AGENT_RUN_PATH));

            let prompt = collect_prompt(args);
            if prompt.is_empty() {
                eprintln!(
                    "Usage: genegis agent run \"名古屋市の人口密度を表示\" [--plan-only] [--planner rule|llm] [--verify-retries N] [--push] [--link-collab] [--json] [-o FILE]"
                );
                process::exit(1);
            }

            let mut config = AgentRunConfig::rule_based_offline()
                .with_planner(planner_config)
                .with_verify_retries(verify_retries)
                .with_link_collab_on_failure(link_collab);
            if plan_only {
                config = config.plan_only();
            }

            let mut run = match AgentOrchestrator::new().with_config(config).run(&prompt) {
                Ok(run) => run,
                Err(err) => {
                    eprintln!("Agent error: {err}");
                    process::exit(1);
                }
            };

            if link_collab && !run.verification_passed && !run.plan_only {
                if link_agent_failure_comment(&mut run) {
                    eprintln!("Collab comment linked to agent run {}", run.id);
                }
            }

            if let Err(err) = run.save_to_path(&output) {
                eprintln!("Failed to write {}: {err}", output.display());
                process::exit(1);
            }

            if push_to_server {
                let server_url = collab_server_url(args);
                match push_agent_run(&server_url, &run) {
                    Ok(_) => eprintln!("Pushed agent run to {server_url}"),
                    Err(err) => {
                        eprintln!("Failed to push agent run: {err}");
                        process::exit(1);
                    }
                }
            }

            eprintln!(
                "Agent run {} · {} steps · verification {} · attempts {}",
                run.id,
                run.steps.len(),
                if run.verification_passed {
                    "passed"
                } else if run.plan_only {
                    "skipped (plan-only)"
                } else {
                    "failed"
                },
                run.verify_attempts
            );
            eprintln!("Trace: {}", output.display());

            if json_output {
                match run.trace_json() {
                    Ok(json) => println!("{json}"),
                    Err(err) => {
                        eprintln!("Failed to serialize trace: {err}");
                        process::exit(1);
                    }
                }
            } else {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&run.summary).expect("json")
                );
            }

            if !run.verification_passed && !run.plan_only {
                process::exit(1);
            }
        }
        Some("pull") => {
            let server_url = collab_server_url(args);
            let output = agent_output_path(args);
            let run = match pull_latest_agent_run(&server_url) {
                Ok(run) => run,
                Err(err) => {
                    eprintln!("Agent pull error: {err}");
                    process::exit(1);
                }
            };
            if let Err(err) = run.save_to_path(&output) {
                eprintln!("Failed to write {}: {err}", output.display());
                process::exit(1);
            }
            eprintln!("Pulled agent run {} to {}", run.id, output.display());
        }
        Some("push") => {
            let server_url = collab_server_url(args);
            let input = agent_input_path(args);
            let run = match AgentRun::load_from_path(&input) {
                Ok(run) => run,
                Err(err) => {
                    eprintln!("Failed to read {}: {err}", input.display());
                    process::exit(1);
                }
            };
            match push_agent_run(&server_url, &run) {
                Ok(saved) => eprintln!("Pushed agent run {} to {server_url}", saved.id),
                Err(err) => {
                    eprintln!("Agent push error: {err}");
                    process::exit(1);
                }
            }
        }
        Some("plan") => {
            let planner_config = planner_config_from_args(args);
            let output = agent_output_path(args);
            let prompt = collect_prompt(args);
            if prompt.is_empty() {
                eprintln!(
                    "Usage: genegis agent plan \"名古屋市の人口密度を表示\" [--planner rule|llm] [-o FILE]"
                );
                process::exit(1);
            }

            let run = match AgentOrchestrator::new()
                .with_config(
                    AgentRunConfig::rule_based_offline()
                        .with_planner(planner_config)
                        .plan_only(),
                )
                .run(&prompt)
            {
                Ok(run) => run,
                Err(err) => {
                    eprintln!("Agent plan error: {err}");
                    process::exit(1);
                }
            };

            if let Err(err) = run.save_to_path(&output) {
                eprintln!("Failed to write {}: {err}", output.display());
                process::exit(1);
            }
            eprintln!(
                "Pending plan saved to {} · run {}",
                DEFAULT_AGENT_PLAN_PATH,
                run.id
            );
            eprintln!("Approve with: genegis agent execute");
            println!(
                "{}",
                serde_json::to_string_pretty(&run.summary).expect("json")
            );
        }
        Some("execute") => {
            let output = agent_output_path(args);
            let push_to_server = args.iter().any(|a| a == "--push");
            let link_collab = args.iter().any(|a| a == "--link-collab");
            let verify_retries = args
                .iter()
                .position(|a| a == "--verify-retries")
                .and_then(|i| args.get(i + 1))
                .and_then(|value| value.parse().ok())
                .unwrap_or(2);

            let plan = match PlanResult::load_from_path(DEFAULT_AGENT_PLAN_PATH) {
                Ok(plan) => plan,
                Err(err) => {
                    eprintln!(
                        "No pending plan at {}: {err}",
                        DEFAULT_AGENT_PLAN_PATH
                    );
                    eprintln!("Run: genegis agent plan \"名古屋市の人口密度を表示\"");
                    process::exit(1);
                }
            };

            let mut run = match AgentOrchestrator::new()
                .with_config(
                    AgentRunConfig::rule_based_offline()
                        .with_verify_retries(verify_retries)
                        .with_link_collab_on_failure(link_collab),
                )
                .execute_plan(plan)
            {
                Ok(run) => run,
                Err(err) => {
                    eprintln!("Agent execute error: {err}");
                    process::exit(1);
                }
            };

            if link_collab && !run.verification_passed {
                let _ = link_agent_failure_comment(&mut run);
            }

            if let Err(err) = run.save_to_path(&output) {
                eprintln!("Failed to write {}: {err}", output.display());
                process::exit(1);
            }

            if push_to_server {
                let server_url = collab_server_url(args);
                if let Err(err) = push_agent_run(&server_url, &run) {
                    eprintln!("Failed to push agent run: {err}");
                    process::exit(1);
                }
            }

            eprintln!(
                "Agent run {} · verification {}",
                run.id,
                if run.verification_passed {
                    "passed"
                } else {
                    "failed"
                }
            );

            if !run.verification_passed {
                process::exit(1);
            }
        }
        Some("list") => {
            let server_url = collab_server_url(args);
            let runs_dir = agent_runs_dir_from_args(args);
            let runs = list_agent_runs(&server_url)
                .or_else(|_| AgentRun::list_from_dir(&runs_dir))
                .unwrap_or_else(|err| {
                    eprintln!("Agent list error: {err}");
                    process::exit(1);
                });
            match serde_json::to_string_pretty(&runs) {
                Ok(json) => println!("{json}"),
                Err(err) => {
                    eprintln!("Failed to serialize runs: {err}");
                    process::exit(1);
                }
            }
        }
        Some("export-audit") => {
            let output = args
                .iter()
                .position(|a| a == "--output" || a == "-o")
                .and_then(|i| args.get(i + 1))
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(".genegis/audit-bundle.json"));
            let session = load_collab_session();
            let runs_dir = agent_runs_dir_from_args(args);
            let collab = AuditCollabSnapshot {
                summary: session
                    .summary_json()
                    .unwrap_or_else(|err| serde_json::json!({ "error": err.to_string() })),
                comments: session
                    .comments_json()
                    .unwrap_or_else(|_| serde_json::json!([])),
                provenance: session
                    .provenance_json()
                    .unwrap_or_else(|_| serde_json::json!([])),
            };
            let bundle = build_audit_bundle(&collab, &runs_dir, DEFAULT_AGENT_RUN_PATH)
                .unwrap_or_else(|err| {
                    eprintln!("Audit export error: {err}");
                    process::exit(1);
                });
            if let Some(parent) = output.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let json = serde_json::to_string_pretty(&bundle).expect("json");
            if let Err(err) = std::fs::write(&output, json) {
                eprintln!("Failed to write {}: {err}", output.display());
                process::exit(1);
            }
            eprintln!("Audit bundle written to {}", output.display());
        }
        Some("get") => {
            let server_url = collab_server_url(args);
            let runs_dir = agent_runs_dir_from_args(args);
            let Some(id) = args.get(1).and_then(|value| uuid::Uuid::parse_str(value).ok()) else {
                eprintln!("Usage: genegis agent get RUN_ID [--url URL]");
                process::exit(1);
            };
            let run = get_agent_run(&server_url, id)
                .or_else(|_| AgentRun::load_from_runs_dir(&runs_dir, id))
                .unwrap_or_else(|err| {
                    eprintln!("Agent get error: {err}");
                    process::exit(1);
                });
            match run.trace_json() {
                Ok(json) => println!("{json}"),
                Err(err) => {
                    eprintln!("Failed to serialize run: {err}");
                    process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("Usage: genegis agent run \"PROMPT\" [--plan-only] [--planner rule|llm] [--verify-retries N] [--push] [--link-collab] [--json] [-o FILE]");
            eprintln!("       genegis agent plan \"PROMPT\" [--planner rule|llm]");
            eprintln!("       genegis agent execute [--push] [--link-collab] [--verify-retries N] [-o FILE]");
            eprintln!("       genegis agent list [--url URL]");
            eprintln!("       genegis agent get RUN_ID [--url URL]");
            eprintln!("       genegis agent export-audit [-o .genegis/audit-bundle.json]");
            eprintln!("       genegis agent pull [--url URL] [-o FILE]");
            eprintln!("       genegis agent push [--url URL] [-i FILE]");
            process::exit(1);
        }
    }
}

fn agent_input_path(args: &[String]) -> PathBuf {
    args.iter()
        .position(|a| a == "--input" || a == "-i")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_AGENT_RUN_PATH))
}

fn agent_output_path(args: &[String]) -> PathBuf {
    args.iter()
        .position(|a| a == "--output" || a == "-o")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_AGENT_RUN_PATH))
}

fn agent_runs_dir_from_args(args: &[String]) -> PathBuf {
    args.iter()
        .position(|a| a == "--runs-dir")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_AGENT_RUNS_DIR))
}

fn link_agent_failure_comment(run: &mut AgentRun) -> bool {
    let Some(verify_step) = run
        .steps
        .iter()
        .rev()
        .find(|step| step.role == AgentRole::Verifier)
    else {
        return false;
    };
    let body = format!(
        "Agent verification failed after {} attempt(s) for prompt: {}",
        run.verify_attempts.max(1),
        run.prompt
    );
    let mut session = load_collab_session();
    match session.add_agent_comment(run.id, verify_step.id, "agent", body) {
        Ok(comment) => {
            save_collab_session(&session);
            run.collab_comment_ids.push(comment.id);
            true
        }
        Err(err) => {
            eprintln!("Collab link error: {err}");
            false
        }
    }
}

fn handle_collab(args: &[String]) {
    match args.first().map(String::as_str) {
        Some("comment") => match args.get(1).map(String::as_str) {
            Some("list") => {
                let session = load_collab_session();
                println!("{}", session.comments_json().expect("comments"));
            }
            Some("add") => {
                let Some(body) = args.get(2) else {
                    eprintln!("Usage: genegis collab comment add \"TEXT\" [--author NAME]");
                    process::exit(1);
                };
                let author = args
                    .iter()
                    .position(|a| a == "--author")
                    .and_then(|i| args.get(i + 1))
                    .map(String::as_str)
                    .unwrap_or("cli");
                let mut session = load_collab_session();
                match session.add_comment(MapComment::new(author, body)) {
                    Ok(_) => {
                        save_collab_session(&session);
                        println!("{}", session.comments_json().expect("comments"));
                    }
                    Err(err) => {
                        eprintln!("Collab error: {err}");
                        process::exit(1);
                    }
                }
            }
            _ => {
                eprintln!("Usage: genegis collab comment list|add");
                process::exit(1);
            }
        },
        Some("branch") => match args.get(1).map(String::as_str) {
            Some("list") => {
                let session = load_collab_session();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&session.branches().expect("branches")).expect("json")
                );
            }
            Some("create") => {
                let Some(name) = args.get(2) else {
                    eprintln!("Usage: genegis collab branch create NAME [--from BRANCH]");
                    process::exit(1);
                };
                let from = args
                    .iter()
                    .position(|a| a == "--from")
                    .and_then(|i| args.get(i + 1))
                    .map(String::as_str);
                let mut session = load_collab_session();
                match session.create_branch(name, from) {
                    Ok(_) => {
                        save_collab_session(&session);
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&session.branches().expect("branches")).expect("json")
                        );
                    }
                    Err(err) => {
                        eprintln!("Collab error: {err}");
                        process::exit(1);
                    }
                }
            }
            _ => {
                eprintln!("Usage: genegis collab branch list|create");
                process::exit(1);
            }
        },
        Some("export") => {
            let output = args
                .iter()
                .position(|a| a == "--output" || a == "-o")
                .and_then(|i| args.get(i + 1))
                .map(PathBuf::from)
                .unwrap_or_else(default_collab_path);
            let session = load_collab_session();
            let json = match session.export_json() {
                Ok(json) => json,
                Err(err) => {
                    eprintln!("Collab error: {err}");
                    process::exit(1);
                }
            };
            if let Some(parent) = output.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(err) = std::fs::write(&output, json) {
                eprintln!("Failed to write {}: {err}", output.display());
                process::exit(1);
            }
            println!("{}", output.display());
        }
        Some("summary") => {
            let session = load_collab_session();
            println!(
                "{}",
                serde_json::to_string_pretty(&session.summary_json().expect("summary")).expect("json")
            );
        }
        Some("provenance") => match args.get(1).map(String::as_str) {
            Some("list") => {
                let session = load_collab_session();
                println!("{}", session.provenance_json().expect("provenance"));
            }
            _ => {
                eprintln!("Usage: genegis collab provenance list");
                process::exit(1);
            }
        },
        Some("pull") => {
            let url = collab_server_url(args);
            let output = collab_output_path(args);
            match pull_session(&url) {
                Ok(session) => {
                    let json = match session.export_json() {
                        Ok(json) => json,
                        Err(err) => {
                            eprintln!("Collab error: {err}");
                            process::exit(1);
                        }
                    };
                    if let Some(parent) = output.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if let Err(err) = std::fs::write(&output, json) {
                        eprintln!("Failed to write {}: {err}", output.display());
                        process::exit(1);
                    }
                    println!(
                        "pulled from {url} -> {} ({} comments)",
                        output.display(),
                        session.comments().expect("comments").len()
                    );
                }
                Err(err) => {
                    eprintln!("Collab pull failed: {err}");
                    process::exit(1);
                }
            }
        }
        Some("push") => {
            let url = collab_server_url(args);
            let input = collab_input_path(args);
            let json = match std::fs::read_to_string(&input) {
                Ok(json) => json,
                Err(err) => {
                    eprintln!("Failed to read {}: {err}", input.display());
                    process::exit(1);
                }
            };
            let session = match CollabSession::import_json(&json) {
                Ok(session) => session,
                Err(err) => {
                    eprintln!("Collab error: {err}");
                    process::exit(1);
                }
            };
            match push_session(&url, &session) {
                Ok(updated) => {
                    println!(
                        "pushed {} -> {url} ({} comments)",
                        input.display(),
                        updated.comments().expect("comments").len()
                    );
                }
                Err(err) => {
                    eprintln!("Collab push failed: {err}");
                    process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("Usage: genegis collab comment list|add");
            eprintln!("       genegis collab branch list|create");
            eprintln!("       genegis collab provenance list");
            eprintln!("       genegis collab export [-o FILE]");
            eprintln!("       genegis collab summary");
            eprintln!("       genegis collab pull [--url URL] [-o FILE]");
            eprintln!("       genegis collab push [--url URL] [-i FILE]");
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

            match name {
                "nagoya-density" => {
                    if execute {
                        run_nagoya_execute(export_html, export_png, output.as_deref());
                    } else {
                        print_workflow_json(&nagoya_population_density_template());
                    }
                }
                "remote-cog-demo" => {
                    if execute {
                        run_remote_cog_execute();
                    } else {
                        print_workflow_json(&remote_cog_metadata_template());
                    }
                }
                _ => {
                    eprintln!("Unknown workflow: {name}");
                    process::exit(1);
                }
            }
        }
        _ => {
            eprintln!(
                "Usage: genegis workflow run [nagoya-density|remote-cog-demo] [--execute] [--html] [--png] [-o FILE]"
            );
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
  genegis pointcloud info PATH|URL                 COPC metadata JSON (local or HTTP range-read)
  genegis plugin list [DIR]                        List plugin manifests (default: ./plugins)
  genegis plugin info BUNDLE_DIR                   Show one plugin manifest + effective caps
  genegis plugin load BUNDLE_DIR                   Capability-gated WASM load smoke
  genegis collab comment list                      List map-anchored review comments
  genegis collab comment add "..." [--author NAME] Add a comment
  genegis collab branch list|create NAME           List or create project branches
  genegis collab provenance list                   List workspace provenance entries
  genegis collab export [-o .genegis/collab.json]  Export collab document JSON
  genegis collab pull [--url URL] [-o FILE]        Pull session from GeneGIS Server
  genegis collab push [--url URL] [-i FILE]        Push session to GeneGIS Server
  genegis agent run "名古屋市の人口密度を表示"       Plan → execute → verify with agent trace
  genegis agent run "..." --plan-only              Planner step only (human gate)
  genegis agent run "..." --verify-retries 2       DuckDB verify retry policy
  genegis agent run "..." --push --link-collab     Push trace + link collab on failure
  genegis agent run "..." --json -o .genegis/agent-run.json  Export trace JSON
  genegis agent plan "名古屋市の人口密度を表示"         Human gate — save pending plan JSON
  genegis agent execute [--push] [--link-collab]   Approve pending plan → execute → verify
  genegis agent list [--url URL]                   List agent run history
  genegis agent get RUN_ID [--url URL]             Fetch one agent run trace
  genegis agent export-audit [-o FILE]             Export collab provenance + agent run index
  genegis agent pull [--url URL] [-o FILE]         Pull latest run from GeneGIS Server
  genegis agent push [--url URL] [-i FILE]         Push run trace to GeneGIS Server
  genegis workflow run nagoya-density              Print workflow graph JSON
  genegis workflow run nagoya-density --execute    Run MVP analysis pipeline
  genegis workflow run remote-cog-demo             Print remote COG metadata workflow JSON
  genegis workflow run remote-cog-demo --execute   Probe catalog COG over HTTP range-read
  genegis workflow run nagoya-density -x --html    Execute + write HTML map
  genegis workflow run nagoya-density -x --png     Execute + write PNG map
  genegis version
  genegis help

LLM planner env: GENEGIS_LLM_API_KEY, GENEGIS_LLM_BASE_URL, GENEGIS_LLM_MODEL

North star: 「名古屋市の人口密度を表示」
"#
    );
}

fn print_workflow_json(workflow: &genegis_workflow::GeoWorkflow) {
    match serde_json::to_string_pretty(workflow) {
        Ok(json) => println!("{json}"),
        Err(err) => {
            eprintln!("Failed to serialize workflow: {err}");
            process::exit(1);
        }
    }
}

fn run_remote_cog_execute() {
    let uri = match alpha_catalog().require(REMOTE_COG_DEMO_ID) {
        Ok(record) => record.uri.clone(),
        Err(err) => {
            eprintln!("Catalog error: {err}");
            process::exit(1);
        }
    };

    match genegis_raster::read_cog_uri(&uri) {
        Ok(info) => {
            println!("{}", serde_json::to_string_pretty(&info.summary_json()).expect("json"));
        }
        Err(err) => {
            eprintln!("Raster error: {err}");
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
