//! GeneGIS CLI — Phase 1: ask, workflow run, execute, export.

use genegis_agent::{
    build_audit_bundle, get_agent_run, list_agent_runs, pull_latest_agent_run, push_agent_run,
    AgentOrchestrator, AgentRole, AgentRun, AgentRunConfig, AuditCollabSnapshot,
    DEFAULT_AGENT_RUNS_DIR, DEFAULT_AGENT_RUN_PATH, DEFAULT_SERVER_URL,
};
use genegis_ai::{plan_with_config, PlannerBackend, PlannerConfig};
use genegis_ai::{PlanResult, DEFAULT_AGENT_PLAN_PATH};
use genegis_analysis::run_ask_pipeline_with_config_and_origin;
use genegis_capsule::{
    create_approval, create_dsse_attestation, diff_capsules, ed25519_public_key,
    execute_ogc_verify_request, export_standard_bundle, review_capsule_with_diff,
    seal_nagoya_capsule, verify_approval, verify_dsse_attestation, verify_nagoya_capsule,
    AnalysisApproval, DsseEnvelope, SourceReview, TrustReview,
};
use genegis_catalog::{
    alpha_catalog, bind_stac_item, browse_alpha_stac_collection, endpoint_registry_path,
    fetch_stac_collection, import_stac_item_url, EndpointRegistry, FederatedCatalog, StacEndpoint,
    StacSearchRequest, LOCAL_COG_DEMO_ID, NAGOYA_WARDS_GEOPARQUET_ID, REMOTE_COG_DEMO_ID,
};
use genegis_collab::{pull_session, push_session, CollabSession, MapComment};
use genegis_contract::VerificationPolicy;
use genegis_core::{Command, CommandEnvelope, CommandOrigin};
use genegis_vector::{
    geoparquet_summary, read_geoparquet_uri, read_geoparquet_uri_with_options,
    GeoParquetReadOptions,
};
use genegis_workflow::{
    copc_change_detect_template, dashboard_export_template, external_stac_fetch_template,
    federated_stac_search_template, local_cog_metadata_template, nagoya_evacuation_template,
    nagoya_flood_exposure_template, nagoya_geoparquet_density_template, nagoya_geoparquet_template,
    nagoya_population_density_template, nagoya_xmin_city_template, remote_cog_metadata_template,
    remote_geoparquet_range_template, sentinel_ndvi_timeseries_template,
    stac_endpoint_registry_template,
};
use std::env;
use std::io::{self, IsTerminal, Read, Write};
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
        Some("tile") => handle_tile(&args[2..]),
        Some("pointcloud") => handle_pointcloud(&args[2..]),
        Some("plugin") => handle_plugin(&args[2..]),
        Some("catalog") => handle_catalog(&args[2..]),
        Some("vector") => handle_vector(&args[2..]),
        Some("collab") => handle_collab(&args[2..]),
        Some("agent") => handle_agent(&args[2..]),
        Some("capsule") => handle_capsule(&args[2..]),
        Some("workflow") => handle_workflow(&args[2..]),
        Some("demo") => handle_demo(&args[2..]),
        Some(cmd) => {
            eprintln!("Unknown command: {cmd}");
            print_help();
            process::exit(1);
        }
    }
}

fn handle_demo(args: &[String]) {
    let Some(action) = args.first().map(String::as_str) else {
        eprintln!("Usage: genegis demo frames [DIR]  # render RFC 0005 showcase PNGs");
        eprintln!("       genegis demo frames-3d [DIR] # render Phase 14 3D district orbit PNGs");
        process::exit(1);
    };
    if action != "frames" && action != "frames-3d" {
        eprintln!("Unknown demo action: {action}");
        process::exit(1);
    }
    let dir = args.get(1).cloned().unwrap_or_else(|| {
        if action == "frames-3d" {
            ".genegis/frames-3d".into()
        } else {
            ".genegis/frames".into()
        }
    });
    std::fs::create_dir_all(&dir).unwrap_or_else(|error| {
        eprintln!("create {dir} failed: {error}");
        process::exit(1);
    });
    let frames: Vec<(String, Vec<u8>)> = if action == "frames-3d" {
        genegis_analysis::render_district3d_frames()
            .unwrap_or_else(|error| {
                eprintln!("3D showcase render error: {error}");
                process::exit(1);
            })
            .into_iter()
            .map(|frame| (frame.name, frame.png))
            .collect()
    } else {
        std::env::set_var("GENEGIS_FRAMES_DIR", &dir);
        genegis_analysis::render_usecase_frames()
            .unwrap_or_else(|error| {
                eprintln!("Showcase render error: {error}");
                process::exit(1);
            })
            .into_iter()
            .map(|frame| (frame.name, frame.png))
            .collect()
    };
    for (name, png) in &frames {
        let path = PathBuf::from(&dir).join(format!("{name}.png"));
        std::fs::write(&path, png).unwrap_or_else(|error| {
            eprintln!("write {} failed: {error}", path.display());
            process::exit(1);
        });
        println!("{} ({} bytes)", path.display(), png.len());
    }
}

fn handle_capsule(args: &[String]) {
    let Some(action) = args.first().map(String::as_str) else {
        eprintln!(
            "Usage: genegis capsule seal PATH | verify PATH [--policy POLICY.json] | diff OLD NEW"
        );
        process::exit(1);
    };
    let Some(root) = args.get(1).map(PathBuf::from) else {
        eprintln!("capsule {action} requires a capsule directory path");
        process::exit(1);
    };
    match action {
        "seal" => {
            let result = run_ask_pipeline_with_config_and_origin(
                "名古屋市の人口密度を表示",
                &PlannerConfig::default(),
                CommandOrigin::Cli,
            )
            .unwrap_or_else(|error| {
                eprintln!("Capsule analysis failed: {error}");
                process::exit(1);
            });
            let manifest = seal_nagoya_capsule(&result, &root).unwrap_or_else(|error| {
                eprintln!("Capsule seal failed: {error}");
                process::exit(1);
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&manifest).expect("capsule manifest JSON")
            );
        }
        "verify" => {
            let policy_path = args
                .iter()
                .position(|argument| argument == "--policy")
                .and_then(|index| args.get(index + 1));
            let policy = policy_path.map(|path| {
                let bytes = std::fs::read(path).unwrap_or_else(|error| {
                    eprintln!("Failed to read policy {path}: {error}");
                    process::exit(1);
                });
                serde_json::from_slice::<VerificationPolicy>(&bytes).unwrap_or_else(|error| {
                    eprintln!("Invalid verification policy {path}: {error}");
                    process::exit(1);
                })
            });
            let verification =
                verify_nagoya_capsule(&root, policy.as_ref()).unwrap_or_else(|error| {
                    eprintln!("Capsule verify failed: {error}");
                    process::exit(1);
                });
            println!(
                "{}",
                serde_json::to_string_pretty(&verification).expect("capsule verification JSON")
            );
        }
        "diff" => {
            let Some(new_root) = args.get(2).map(PathBuf::from) else {
                eprintln!("Usage: genegis capsule diff OLD NEW");
                process::exit(1);
            };
            let report = diff_capsules(&root, &new_root).unwrap_or_else(|error| {
                eprintln!("Capsule diff failed: {error}");
                process::exit(1);
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&report).expect("semantic diff JSON")
            );
        }
        "approve" => {
            let reviewer = args
                .iter()
                .position(|argument| argument == "--reviewer")
                .and_then(|index| args.get(index + 1))
                .cloned()
                .unwrap_or_else(|| {
                    eprintln!("capsule approve requires --reviewer ID");
                    process::exit(1);
                });
            let output = args
                .iter()
                .position(|argument| argument == "--output" || argument == "-o")
                .and_then(|index| args.get(index + 1))
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("approval.json"));
            let approval = create_approval(&root, reviewer, chrono::Utc::now().to_rfc3339(), None)
                .unwrap_or_else(|error| {
                    eprintln!("Approval failed: {error}");
                    process::exit(1);
                });
            let bytes = serde_json::to_vec_pretty(&approval).expect("approval JSON");
            write_bytes(&output, &bytes, "approval");
        }
        "check-approval" => {
            let Some(approval_path) = args.get(2) else {
                eprintln!("Usage: genegis capsule check-approval PATH APPROVAL.json");
                process::exit(1);
            };
            let bytes = std::fs::read(approval_path).unwrap_or_else(|error| {
                eprintln!("Failed to read approval {approval_path}: {error}");
                process::exit(1);
            });
            let approval: AnalysisApproval =
                serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                    eprintln!("Invalid approval {approval_path}: {error}");
                    process::exit(1);
                });
            verify_approval(&root, &approval, None).unwrap_or_else(|error| {
                eprintln!("Approval check failed: {error}");
                process::exit(1);
            });
            println!("approval valid for {}", root.display());
        }
        "review" => {
            let semantic_diff = option_value(args, "--diff").map(|other| {
                diff_capsules(&root, &other).unwrap_or_else(|error| {
                    eprintln!("Capsule diff failed: {error}");
                    process::exit(1);
                })
            });
            let review =
                review_capsule_with_diff(&root, None, semantic_diff).unwrap_or_else(|error| {
                    eprintln!("Capsule review failed: {error}");
                    process::exit(1);
                });
            let force_tui = args.iter().any(|argument| argument == "--tui");
            if !force_tui
                && (args.iter().any(|argument| argument == "--json") || !io::stdout().is_terminal())
            {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&review).expect("trust review JSON")
                );
            } else if let Err(error) = run_trust_debugger(&review) {
                eprintln!("Trust Debugger failed: {error}");
                process::exit(1);
            }
        }
        "export-standards" => {
            let Some(output) = args.get(2).map(PathBuf::from) else {
                eprintln!("Usage: genegis capsule export-standards PATH OUTPUT_DIR");
                process::exit(1);
            };
            let report = export_standard_bundle(&root, &output).unwrap_or_else(|error| {
                eprintln!("Standards export failed: {error}");
                process::exit(1);
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&report).expect("standards export JSON")
            );
        }
        "attest" => {
            let key_path = required_option(args, "--key");
            let keyid = option_value(args, "--key-id").unwrap_or_else(|| "ed25519-local".into());
            let output = option_value(args, "--output")
                .or_else(|| option_value(args, "-o"))
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("genegis-attestation.json"));
            let secret = read_hex_key(Path::new(&key_path));
            let envelope = create_dsse_attestation(&root, &secret, keyid).unwrap_or_else(|error| {
                eprintln!("Attestation failed: {error}");
                process::exit(1);
            });
            write_bytes(
                &output,
                &serde_json::to_vec_pretty(&envelope).expect("DSSE JSON"),
                "DSSE attestation",
            );
            eprintln!(
                "Ed25519 public key: {}",
                encode_hex(&ed25519_public_key(&secret))
            );
        }
        "verify-attestation" => {
            let Some(envelope_path) = args.get(2) else {
                eprintln!(
                    "Usage: genegis capsule verify-attestation PATH ENVELOPE --public-key FILE"
                );
                process::exit(1);
            };
            let public_key = read_hex_key(Path::new(&required_option(args, "--public-key")));
            let bytes = std::fs::read(envelope_path).unwrap_or_else(|error| {
                eprintln!("Failed to read attestation {envelope_path}: {error}");
                process::exit(1);
            });
            let envelope: DsseEnvelope = serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                eprintln!("Invalid DSSE envelope: {error}");
                process::exit(1);
            });
            let statement =
                verify_dsse_attestation(&root, &envelope, &public_key).unwrap_or_else(|error| {
                    eprintln!("Attestation verification failed: {error}");
                    process::exit(1);
                });
            println!(
                "{}",
                serde_json::to_string_pretty(&statement).expect("in-toto Statement JSON")
            );
        }
        "execute-ogc" => {
            let request: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&root).unwrap_or_else(|error| {
                    eprintln!(
                        "Failed to read OGC execute request {}: {error}",
                        root.display()
                    );
                    process::exit(1);
                }))
                .unwrap_or_else(|error| {
                    eprintln!("Invalid OGC execute request: {error}");
                    process::exit(1);
                });
            let result = execute_ogc_verify_request(&request).unwrap_or_else(|error| {
                eprintln!("OGC process execution failed: {error}");
                process::exit(1);
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&result).expect("OGC result JSON")
            );
        }
        _ => {
            eprintln!("Unknown capsule command: {action}");
            process::exit(1);
        }
    }
}

fn option_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|argument| argument == name)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn required_option(args: &[String], name: &str) -> String {
    option_value(args, name).unwrap_or_else(|| {
        eprintln!("{name} requires a value");
        process::exit(1);
    })
}

fn read_hex_key(path: &Path) -> [u8; 32] {
    let text = std::fs::read_to_string(path).unwrap_or_else(|error| {
        eprintln!("Failed to read key {}: {error}", path.display());
        process::exit(1);
    });
    let text = text.trim();
    if text.len() != 64 {
        eprintln!(
            "Key {} must contain exactly 64 hexadecimal characters",
            path.display()
        );
        process::exit(1);
    }
    let mut bytes = [0_u8; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).unwrap_or_else(|_| {
            eprintln!("Key {} is not hexadecimal", path.display());
            process::exit(1);
        });
    }
    bytes
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn run_trust_debugger(review: &TrustReview) -> Result<(), String> {
    use crossterm::cursor::{Hide, MoveTo, Show};
    use crossterm::event::{self, Event, KeyCode};
    use crossterm::execute;
    use crossterm::terminal::{
        disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
        LeaveAlternateScreen,
    };

    let mut stdout = io::stdout();
    enable_raw_mode().map_err(|error| error.to_string())?;
    execute!(stdout, EnterAlternateScreen, Hide).map_err(|error| error.to_string())?;
    let mut pane = 0usize;
    let mut selected = 0usize;
    let mut source_preview = None;
    let interaction = (|| -> Result<(), String> {
        loop {
            execute!(stdout, MoveTo(0, 0), Clear(ClearType::All))
                .map_err(|error| error.to_string())?;
            let (width, height) = crossterm::terminal::size().map_err(|e| e.to_string())?;
            for (row, line) in
                trust_debugger_lines(review, pane, selected, source_preview.as_deref())
                    .into_iter()
                    .take(height as usize)
                    .enumerate()
            {
                execute!(stdout, MoveTo(0, row as u16)).map_err(|e| e.to_string())?;
                write!(stdout, "{}", truncate_line(&line, width as usize))
                    .map_err(|e| e.to_string())?;
            }
            stdout.flush().map_err(|error| error.to_string())?;
            let Event::Key(key) = event::read().map_err(|error| error.to_string())? else {
                continue;
            };
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Tab | KeyCode::Right => {
                    pane = (pane + 1) % 7;
                    selected = 0;
                    source_preview = None;
                }
                KeyCode::BackTab | KeyCode::Left => {
                    pane = (pane + 6) % 7;
                    selected = 0;
                    source_preview = None;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    selected = (selected + 1).min(review_pane_len(review, pane).saturating_sub(1));
                }
                KeyCode::Up | KeyCode::Char('k') => selected = selected.saturating_sub(1),
                KeyCode::Char(number @ '1'..='7') => {
                    pane = number.to_digit(10).expect("digit") as usize - 1;
                    selected = 0;
                    source_preview = None;
                }
                KeyCode::Char('o') if pane == 2 => {
                    source_preview = review.sources.get(selected).map(preview_source);
                }
                KeyCode::Enter if pane == 5 => {
                    if let Some(node) = failure_target_node(review, selected) {
                        pane = 3;
                        selected = review
                            .workflow_nodes
                            .iter()
                            .position(|candidate| candidate.stable_id == node)
                            .unwrap_or(0);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    })();
    let _ = disable_raw_mode();
    let _ = execute!(stdout, Show, LeaveAlternateScreen);
    interaction
}

fn trust_debugger_lines(
    review: &TrustReview,
    pane: usize,
    selected: usize,
    source_preview: Option<&str>,
) -> Vec<String> {
    let trust = review
        .verification
        .as_ref()
        .map(|value| format!("{:?}", value.trust.level).to_uppercase())
        .unwrap_or_else(|| "INVALID".into());
    let tabs = [
        "Claims",
        "Contracts",
        "Sources",
        "Workflow",
        "Artifacts",
        "Failures",
        "Diff",
    ];
    let mut lines = vec![
        format!("GeneGIS Trust Debugger  trust={trust}"),
        tabs.iter()
            .enumerate()
            .map(|(index, label)| {
                if index == pane {
                    format!("[{label}]")
                } else {
                    format!(" {label} ")
                }
            })
            .collect::<Vec<_>>()
            .join("  "),
        format!(
            "result={}  workflow={}",
            review.identities.result_digest, review.identities.workflow_digest
        ),
        format!(
            "policy={}  verification={}",
            review.identities.policy_digest, review.identities.verification_graph_digest
        ),
        String::new(),
    ];
    match pane {
        0 => {
            for (index, claim) in review.claims.iter().enumerate() {
                lines.push(format!(
                    "{} {} {}  independence={} error={:?}/{:?}ppm",
                    marker(index, selected),
                    if claim.passed { "PASS" } else { "FAIL" },
                    claim.check_id,
                    claim.independence,
                    claim.observed_error_ppm,
                    claim.maximum_error_ppm
                ));
            }
            if let Some(claim) = review.claims.get(selected) {
                lines.push(String::new());
                lines.push(format!("Claim: {}", claim.claim));
                lines.push(format!("Verifier: {}", claim.verifier));
                lines.push(format!("Depends on: {}", claim.depends_on.join(", ")));
                lines.push(format!(
                    "Workflow nodes: {}",
                    claim.workflow_nodes.join(", ")
                ));
            }
        }
        1 => {
            for (index, contract) in review.contracts.iter().enumerate() {
                lines.push(format!(
                    "{} {} schema={} valid={} compatibility={:?}",
                    marker(index, selected),
                    contract.contract_id,
                    contract.schema_version,
                    contract.valid,
                    contract.compatibility
                ));
            }
        }
        2 => {
            for (index, source) in review.sources.iter().enumerate() {
                lines.push(format!(
                    "{} {}  checksum={}  version={}",
                    marker(index, selected),
                    source.source_id,
                    source.checksum_status,
                    source.version.as_deref().unwrap_or("unknown")
                ));
            }
            if let Some(source) = review.sources.get(selected) {
                lines.push(String::new());
                lines.push(format!("URI: {}", source.uri));
                lines.push(format!(
                    "License: {}",
                    source.license.as_deref().unwrap_or("unknown")
                ));
                lines.push(format!(
                    "Expected: {}",
                    source.expected_checksum.as_deref().unwrap_or("unknown")
                ));
                lines.push(format!(
                    "Observed: {}",
                    source.observed_checksum.as_deref().unwrap_or("unknown")
                ));
            }
            if let Some(preview) = source_preview {
                lines.push(String::new());
                lines.push("Source preview:".into());
                lines.extend(preview.lines().map(ToOwned::to_owned));
            }
        }
        3 => {
            for (index, node) in review.workflow_nodes.iter().enumerate() {
                lines.push(format!(
                    "{} {}  {}  ← {}",
                    marker(index, selected),
                    node.stable_id,
                    node.operation,
                    node.depends_on.join(", ")
                ));
            }
            if let Some(node) = review.workflow_nodes.get(selected) {
                lines.push(String::new());
                lines.push(format!("Parameters: {}", node.parameters));
                let checks = review
                    .claims
                    .iter()
                    .filter(|claim| claim.workflow_nodes.contains(&node.stable_id))
                    .map(|claim| claim.check_id.as_str())
                    .collect::<Vec<_>>();
                lines.push(format!("Verification claims: {}", checks.join(", ")));
            }
        }
        4 => {
            for (index, artifact) in review.artifacts.iter().enumerate() {
                lines.push(format!(
                    "{} {}  {}  {} bytes  {}",
                    marker(index, selected),
                    artifact.role,
                    artifact.path,
                    artifact.bytes,
                    artifact.sha256
                ));
            }
        }
        5 => {
            if let Some(error) = &review.integrity_error {
                lines.push(format!("{} INTEGRITY {error}", marker(0, selected)));
            }
            let offset = usize::from(review.integrity_error.is_some());
            for (index, failure) in review.failures.iter().enumerate() {
                lines.push(format!(
                    "{} {:?}/{} {}: {}",
                    marker(index + offset, selected),
                    failure.gate,
                    failure.code,
                    failure.subject,
                    failure.detail
                ));
                if index + offset == selected {
                    lines.push(format!("  Nodes: {}", failure.affected_nodes.join(", ")));
                    lines.push(format!("  Remediation: {}", failure.remediation));
                }
            }
            if review.integrity_error.is_none() && review.failures.is_empty() {
                lines.push("No failures.".into());
            }
            lines.push(String::new());
            lines.push("Enter jumps to the affected Workflow node.".into());
        }
        _ => {
            if let Some(diff) = &review.semantic_diff {
                lines.push(format!(
                    "{} → {}",
                    diff.old_result_digest, diff.new_result_digest
                ));
                for (index, change) in diff.changes.iter().enumerate() {
                    lines.push(format!(
                        "{} {:?}/{:?} {} {}",
                        marker(index, selected),
                        change.category,
                        change.kind,
                        change.subject_role,
                        change.path
                    ));
                }
                lines.push(format!(
                    "Unclassified changes: {}",
                    diff.unclassified_changes
                ));
            } else {
                lines.push("No comparison loaded. Use --diff OTHER_CAPSULE.".into());
            }
        }
    }
    lines.push(String::new());
    lines.push("←/→ or Tab pane  ↑/↓ or j/k select  1–7 jump  o preview source  Enter trace failure  q quit".into());
    lines
}

fn review_pane_len(review: &TrustReview, pane: usize) -> usize {
    match pane {
        0 => review.claims.len(),
        1 => review.contracts.len(),
        2 => review.sources.len(),
        3 => review.workflow_nodes.len(),
        4 => review.artifacts.len(),
        5 => review.failures.len() + usize::from(review.integrity_error.is_some()),
        _ => review
            .semantic_diff
            .as_ref()
            .map_or(0, |diff| diff.changes.len()),
    }
}

fn failure_target_node(review: &TrustReview, selected: usize) -> Option<&str> {
    let offset = usize::from(review.integrity_error.is_some());
    let failure = selected
        .checked_sub(offset)
        .and_then(|index| review.failures.get(index))?;
    failure
        .affected_nodes
        .first()
        .map(String::as_str)
        .or_else(|| {
            review
                .claims
                .iter()
                .find(|claim| claim.check_id == failure.subject)
                .and_then(|claim| claim.workflow_nodes.first())
                .map(String::as_str)
        })
}

fn preview_source(source: &SourceReview) -> String {
    if source.uri.starts_with("http://") || source.uri.starts_with("https://") {
        return format!(
            "Remote source (not fetched by the offline debugger): {}",
            source.uri
        );
    }
    let raw_path = source.uri.strip_prefix("file://").unwrap_or(&source.uri);
    let path = Path::new(raw_path);
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return format!(
            "Local source is not available from this environment: {}",
            path.display()
        );
    };
    if !metadata.file_type().is_file() {
        return format!("Source preview requires a regular file: {}", path.display());
    }
    let Ok(file) = std::fs::File::open(path) else {
        return format!("Source cannot be opened: {}", path.display());
    };
    let mut bytes = Vec::new();
    if file.take(16 * 1024).read_to_end(&mut bytes).is_err() {
        return format!("Source cannot be read: {}", path.display());
    }
    String::from_utf8(bytes).unwrap_or_else(|_| {
        format!(
            "Binary source: {} bytes sampled from {}",
            metadata.len().min(16 * 1024),
            path.display()
        )
    })
}

fn marker(index: usize, selected: usize) -> &'static str {
    if index == selected {
        ">"
    } else {
        " "
    }
}

fn truncate_line(line: &str, width: usize) -> String {
    line.chars().take(width.saturating_sub(1)).collect()
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

    let result =
        match run_ask_pipeline_with_config_and_origin(&prompt, &planner_config, CommandOrigin::Cli)
        {
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
        benchmark_pipeline, benchmark_render_mesh, run_all_benchmarks,
        run_cross_engine_equivalence, run_external_benchmark, BenchmarkReport, DEFAULT_ITERATIONS,
        DEFAULT_WARMUP,
    };

    let json_output = args.iter().any(|a| a == "--json");
    if args.iter().any(|argument| argument == "trust-ux-aggregate") {
        let inputs = args
            .windows(2)
            .filter(|pair| pair[0] == "--input")
            .map(|pair| PathBuf::from(&pair[1]))
            .collect::<Vec<_>>();
        if inputs.is_empty() {
            eprintln!("trust-ux-aggregate requires at least one --input SESSION.json");
            process::exit(1);
        }
        let sessions = inputs
            .iter()
            .map(|path| {
                let bytes = std::fs::read(path).unwrap_or_else(|error| {
                    eprintln!("Failed to read {}: {error}", path.display());
                    process::exit(1);
                });
                serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                    eprintln!("Invalid Trust UX session {}: {error}", path.display());
                    process::exit(1);
                })
            })
            .collect::<Vec<genegis_testkit::TrustUxSessionReport>>();
        let report = genegis_testkit::aggregate_trust_ux_sessions(&sessions);
        if let Some(output) = option_value(args, "--output").or_else(|| option_value(args, "-o")) {
            write_bytes(
                Path::new(&output),
                &serde_json::to_vec_pretty(&report).expect("Trust UX aggregate JSON"),
                "Trust UX aggregate report",
            );
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&report).expect("Trust UX aggregate JSON")
        );
        return;
    }
    if args.iter().any(|argument| argument == "trust-ux") {
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            eprintln!("Trust UX study requires an interactive terminal");
            process::exit(1);
        }
        let reviewer = required_option(args, "--reviewer-code");
        let session_kind = if args.iter().any(|argument| argument == "--human") {
            genegis_testkit::TrustUxSessionKind::Human
        } else {
            genegis_testkit::TrustUxSessionKind::Automated
        };
        let facilitator = match session_kind {
            genegis_testkit::TrustUxSessionKind::Human => {
                Some(required_option(args, "--facilitator-code"))
            }
            genegis_testkit::TrustUxSessionKind::Automated => {
                option_value(args, "--facilitator-code")
            }
        };
        let output = option_value(args, "--output")
            .or_else(|| option_value(args, "-o"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(format!("phase-12-trust-ux-{reviewer}.json")));
        let report =
            run_trust_ux_session(reviewer, facilitator, session_kind).unwrap_or_else(|error| {
                eprintln!("Trust UX session failed: {error}");
                process::exit(1);
            });
        write_bytes(
            &output,
            &serde_json::to_vec_pretty(&report).expect("Trust UX session JSON"),
            "Trust UX session report",
        );
        println!(
            "Trust UX session: {}/{} answered, aborts={}, output={}",
            report
                .results
                .iter()
                .filter(|result| !result.aborted)
                .count(),
            genegis_testkit::trust_ux_task_corpus().len(),
            report
                .results
                .iter()
                .filter(|result| result.aborted)
                .count(),
            output.display()
        );
        return;
    }
    if args.iter().any(|argument| argument == "review") {
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            eprintln!("Reviewer timing requires an interactive terminal");
            process::exit(1);
        }
        let reviewer = required_option(args, "--reviewer");
        let output = option_value(args, "--output")
            .or_else(|| option_value(args, "-o"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("phase-11-review-timing.json"));
        let report = run_reviewer_timing_session(reviewer).unwrap_or_else(|error| {
            eprintln!("Reviewer timing failed: {error}");
            process::exit(1);
        });
        write_bytes(
            &output,
            &serde_json::to_vec_pretty(&report).expect("review timing JSON"),
            "review timing report",
        );
        println!(
            "Review timing: {}/{} correct, median {:.3}s, gate={}",
            report.correct, report.total, report.median_seconds, report.passed
        );
        return;
    }
    if args.iter().any(|argument| argument == "external") {
        let report = run_external_benchmark().unwrap_or_else(|error| {
            eprintln!("External benchmark error: {error}");
            process::exit(1);
        });
        if json_output {
            println!(
                "{}",
                serde_json::to_string_pretty(&report).expect("external benchmark JSON")
            );
        } else {
            println!(
                "External strict-artifact adapter: {}/{} passed, false accepts={}",
                report.passed,
                report.cases.len(),
                report.false_accepts
            );
        }
        return;
    }
    if args.iter().any(|argument| argument == "equivalence") {
        let report = run_cross_engine_equivalence().unwrap_or_else(|error| {
            eprintln!("Equivalence error: {error}");
            process::exit(1);
        });
        if json_output {
            println!(
                "{}",
                serde_json::to_string_pretty(&report).expect("equivalence JSON")
            );
        } else {
            println!(
                "Cross-engine equivalence: {}/{} passed, false accepts={}, max delta={} ppm",
                report.passed,
                report.cases.len(),
                report.false_accepts,
                report.maximum_delta_ppm
            );
        }
        return;
    }
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

fn run_trust_ux_session(
    reviewer_id: String,
    facilitator_id: Option<String>,
    session_kind: genegis_testkit::TrustUxSessionKind,
) -> Result<genegis_testkit::TrustUxSessionReport, String> {
    use crossterm::cursor::{Hide, MoveTo, Show};
    use crossterm::event::{self, Event, KeyCode};
    use crossterm::execute;
    use crossterm::terminal::{
        disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
        LeaveAlternateScreen,
    };
    use genegis_testkit::{
        seal_trust_ux_session, trust_ux_corpus_digest, trust_ux_task_corpus,
        validate_trust_ux_session, TrustUxSessionReport, TrustUxTaskResult,
        TRUST_UX_CORPUS_VERSION,
    };

    let tasks = trust_ux_task_corpus();
    let started_at = chrono::Utc::now().to_rfc3339();
    let mut results = Vec::new();
    let mut stdout = io::stdout();
    enable_raw_mode().map_err(|error| error.to_string())?;
    execute!(stdout, EnterAlternateScreen, Hide).map_err(|error| error.to_string())?;
    let interaction = (|| -> Result<(), String> {
        for (task_index, task) in tasks.iter().enumerate() {
            loop {
                execute!(stdout, MoveTo(0, 0), Clear(ClearType::All))
                    .map_err(|error| error.to_string())?;
                let (width, _) = crossterm::terminal::size().map_err(|error| error.to_string())?;
                let lines = [
                    format!(
                        "GeneGIS Map-first Trust UX  task {}/{}  reviewer={reviewer_id}",
                        task_index + 1,
                        tasks.len()
                    ),
                    String::new(),
                    "The task is hidden and the timer is stopped.".into(),
                    "Start from the map; open evidence with 1/2/3, then press a to answer.".into(),
                    "Press Space or Enter when ready. Press q to record an abort.".into(),
                ];
                for (row, line) in lines.into_iter().enumerate() {
                    execute!(stdout, MoveTo(0, row as u16)).map_err(|e| e.to_string())?;
                    write!(stdout, "{}", truncate_line(&line, width as usize))
                        .map_err(|e| e.to_string())?;
                }
                stdout.flush().map_err(|error| error.to_string())?;
                let Event::Key(key) = event::read().map_err(|error| error.to_string())? else {
                    continue;
                };
                match key.code {
                    KeyCode::Char(' ') | KeyCode::Enter => break,
                    KeyCode::Char('q') | KeyCode::Esc => {
                        results.push(aborted_trust_ux_result(
                            &task.task_id,
                            0.0,
                            Vec::new(),
                            None,
                        ));
                        return Ok(());
                    }
                    _ => {}
                }
            }

            let started = std::time::Instant::now();
            let mut opened_card: Option<usize> = None;
            let mut opened_card_ids = Vec::new();
            let mut interaction_count = 0_u32;
            let mut interactions_to_decisive = None;
            let mut answering = false;
            let mut selected_answer = 0_usize;
            loop {
                execute!(stdout, MoveTo(0, 0), Clear(ClearType::All))
                    .map_err(|error| error.to_string())?;
                let (width, height) = crossterm::terminal::size().map_err(|e| e.to_string())?;
                let mut lines = vec![
                    format!(
                        "GeneGIS Map-first Trust UX  task {}/{}  category={}",
                        task_index + 1,
                        tasks.len(),
                        task.category
                    ),
                    format!("Map: {}", task.map_title),
                    String::new(),
                ];
                lines.extend(task.map_lines.iter().cloned());
                lines.push(String::new());
                lines.push("Evidence: [1] Source  [2] Contract/workflow  [3] I/O/artifact".into());
                if let Some(card_index) = opened_card {
                    let card = &task.evidence_cards[card_index];
                    lines.push(format!("┌ {} ─────────────────────────", card.title));
                    lines.push(format!("│ {}", card.detail));
                    lines.push("└────────────────────────────────────────".into());
                } else {
                    lines.push("No evidence card opened; the map remains the primary view.".into());
                }
                lines.push(String::new());
                if answering {
                    lines.push("Choose the diagnosis:".into());
                    lines.extend(
                        task.answer_choices
                            .iter()
                            .enumerate()
                            .map(|(index, choice)| {
                                format!("{} {}", marker(index, selected_answer), choice.label)
                            }),
                    );
                    lines.push("↑/↓ or j/k select  Enter submit  1/2/3 evidence  q abort".into());
                } else {
                    lines.push("1/2/3 open evidence  a answer  q record abort".into());
                }
                for (row, line) in lines.into_iter().take(height as usize).enumerate() {
                    execute!(stdout, MoveTo(0, row as u16)).map_err(|e| e.to_string())?;
                    write!(stdout, "{}", truncate_line(&line, width as usize))
                        .map_err(|e| e.to_string())?;
                }
                stdout.flush().map_err(|error| error.to_string())?;
                let Event::Key(key) = event::read().map_err(|error| error.to_string())? else {
                    continue;
                };
                match key.code {
                    KeyCode::Char(character @ '1'..='3') => {
                        let card_index = character as usize - '1' as usize;
                        opened_card = Some(card_index);
                        opened_card_ids.push(task.evidence_cards[card_index].card_id.clone());
                        interaction_count = interaction_count.saturating_add(1);
                        if task.evidence_cards[card_index].card_id == task.decisive_card_id
                            && interactions_to_decisive.is_none()
                        {
                            interactions_to_decisive = Some(interaction_count);
                        }
                    }
                    KeyCode::Char('a') => answering = true,
                    KeyCode::Down | KeyCode::Char('j') if answering => {
                        selected_answer =
                            (selected_answer + 1).min(task.answer_choices.len().saturating_sub(1));
                    }
                    KeyCode::Up | KeyCode::Char('k') if answering => {
                        selected_answer = selected_answer.saturating_sub(1);
                    }
                    KeyCode::Enter if answering => {
                        let answer_id = task.answer_choices[selected_answer].answer_id.clone();
                        results.push(TrustUxTaskResult {
                            task_id: task.task_id.clone(),
                            correct: answer_id == task.expected_answer_id,
                            answer_id: Some(answer_id),
                            elapsed_seconds: started.elapsed().as_secs_f64(),
                            interaction_count,
                            opened_card_ids: opened_card_ids.clone(),
                            interactions_to_decisive_evidence: interactions_to_decisive,
                            aborted: false,
                        });
                        break;
                    }
                    KeyCode::Char('q') | KeyCode::Esc => {
                        results.push(aborted_trust_ux_result(
                            &task.task_id,
                            started.elapsed().as_secs_f64(),
                            opened_card_ids.clone(),
                            interactions_to_decisive,
                        ));
                        return Ok(());
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    })();
    let _ = disable_raw_mode();
    let _ = execute!(stdout, Show, LeaveAlternateScreen);
    interaction?;

    let report = seal_trust_ux_session(TrustUxSessionReport {
        schema_version: "0.1.0".into(),
        session_kind,
        reviewer_id,
        facilitator_id,
        runner_identity: format!(
            "genegis-cli/{} map-first-trust-ux-v1",
            env!("CARGO_PKG_VERSION")
        ),
        corpus_version: TRUST_UX_CORPUS_VERSION.into(),
        corpus_digest: trust_ux_corpus_digest(),
        started_at,
        finished_at: chrono::Utc::now().to_rfc3339(),
        results,
        report_digest: String::new(),
    });
    validate_trust_ux_session(&report)?;
    Ok(report)
}

fn aborted_trust_ux_result(
    task_id: &str,
    elapsed_seconds: f64,
    opened_card_ids: Vec<String>,
    interactions_to_decisive_evidence: Option<u32>,
) -> genegis_testkit::TrustUxTaskResult {
    genegis_testkit::TrustUxTaskResult {
        task_id: task_id.into(),
        answer_id: None,
        correct: false,
        elapsed_seconds,
        interaction_count: opened_card_ids.len() as u32,
        opened_card_ids,
        interactions_to_decisive_evidence,
        aborted: true,
    }
}

fn run_reviewer_timing_session(
    reviewer: String,
) -> Result<genegis_testkit::ReviewTimingReport, String> {
    use crossterm::cursor::{Hide, MoveTo, Show};
    use crossterm::event::{self, Event, KeyCode};
    use crossterm::execute;
    use crossterm::terminal::{
        disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
        LeaveAlternateScreen,
    };
    use genegis_testkit::{
        review_median_seconds, review_task_corpus, ReviewTaskResult, ReviewTimingReport,
    };

    let tasks = review_task_corpus();
    let mut results = Vec::new();
    let mut stdout = io::stdout();
    enable_raw_mode().map_err(|error| error.to_string())?;
    execute!(stdout, EnterAlternateScreen, Hide).map_err(|error| error.to_string())?;
    let interaction = (|| -> Result<(), String> {
        for (task_index, task) in tasks.iter().enumerate() {
            loop {
                execute!(stdout, MoveTo(0, 0), Clear(ClearType::All))
                    .map_err(|error| error.to_string())?;
                let (width, _) = crossterm::terminal::size().map_err(|e| e.to_string())?;
                let lines = [
                    format!(
                        "GeneGIS reviewer timing  task {}/{}  reviewer={reviewer}",
                        task_index + 1,
                        tasks.len()
                    ),
                    String::new(),
                    "The failure remains hidden and the timer has not started.".into(),
                    "Press Space or Enter when ready. Press q to abort.".into(),
                ];
                for (row, line) in lines.into_iter().enumerate() {
                    execute!(stdout, MoveTo(0, row as u16)).map_err(|e| e.to_string())?;
                    write!(stdout, "{}", truncate_line(&line, width as usize))
                        .map_err(|e| e.to_string())?;
                }
                stdout.flush().map_err(|error| error.to_string())?;
                let Event::Key(key) = event::read().map_err(|error| error.to_string())? else {
                    continue;
                };
                match key.code {
                    KeyCode::Char(' ') | KeyCode::Enter => break,
                    KeyCode::Char('q') | KeyCode::Esc => {
                        return Err("session aborted; no timing report written".into());
                    }
                    _ => {}
                }
            }
            let started = std::time::Instant::now();
            let mut selected = 0usize;
            loop {
                execute!(stdout, MoveTo(0, 0), Clear(ClearType::All))
                    .map_err(|error| error.to_string())?;
                let (width, height) = crossterm::terminal::size().map_err(|e| e.to_string())?;
                let mut lines = vec![
                    format!(
                        "GeneGIS reviewer timing  task {}/{}  reviewer={reviewer}",
                        task_index + 1,
                        tasks.len()
                    ),
                    String::new(),
                    format!("Failure: {}/{}", task.failure_code, task.subject),
                    task.detail.clone(),
                    String::new(),
                    "Select the first Workflow node you would inspect:".into(),
                ];
                lines.extend(
                    task.choices
                        .iter()
                        .enumerate()
                        .map(|(index, choice)| format!("{} {}", marker(index, selected), choice)),
                );
                lines.push(String::new());
                lines.push("↑/↓ or j/k select  Enter submit  q abort".into());
                for (row, line) in lines.into_iter().take(height as usize).enumerate() {
                    execute!(stdout, MoveTo(0, row as u16)).map_err(|e| e.to_string())?;
                    write!(stdout, "{}", truncate_line(&line, width as usize))
                        .map_err(|e| e.to_string())?;
                }
                stdout.flush().map_err(|error| error.to_string())?;
                let Event::Key(key) = event::read().map_err(|error| error.to_string())? else {
                    continue;
                };
                match key.code {
                    KeyCode::Down | KeyCode::Char('j') => {
                        selected = (selected + 1).min(task.choices.len().saturating_sub(1));
                    }
                    KeyCode::Up | KeyCode::Char('k') => selected = selected.saturating_sub(1),
                    KeyCode::Enter => {
                        let selected_node = task.choices[selected].clone();
                        results.push(ReviewTaskResult {
                            task_id: task.task_id.clone(),
                            correct: selected_node == task.expected_node,
                            selected_node,
                            expected_node: task.expected_node.clone(),
                            elapsed_seconds: started.elapsed().as_secs_f64(),
                        });
                        break;
                    }
                    KeyCode::Char('q') | KeyCode::Esc => {
                        return Err("session aborted; no timing report written".into());
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    })();
    let _ = disable_raw_mode();
    let _ = execute!(stdout, Show, LeaveAlternateScreen);
    interaction?;

    let median_seconds = review_median_seconds(&results);
    let correct = results.iter().filter(|result| result.correct).count();
    let total = results.len();
    Ok(ReviewTimingReport {
        schema_version: "0.1.0".into(),
        reviewer,
        runner_identity: format!(
            "genegis-cli/{} trust-debugger-v1",
            env!("CARGO_PKG_VERSION")
        ),
        corpus_version: "phase-11-seeded-failures-v1".into(),
        results,
        median_seconds,
        correct,
        total,
        passed: correct == total && median_seconds <= 120.0,
    })
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
                eprintln!(
                    "Usage: genegis storage fetch URL [--range START-END] [--json] [-o FILE]"
                );
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

fn handle_tile(args: &[String]) {
    const TILE_USAGE: &str = "Usage: genegis tile export --dataset nagoya-density [-o OUT.pmtiles] [--min-zoom 7] [--max-zoom 11]";
    match args.first().map(String::as_str) {
        Some("export") => {
            let dataset = option_value(args, "--dataset").unwrap_or_else(|| {
                eprintln!("--dataset is required (supported: nagoya-density)");
                process::exit(1);
            });
            if dataset != "nagoya-density" {
                eprintln!("Unknown dashboard dataset: {dataset}");
                process::exit(1);
            }
            let output = option_value(args, "-o")
                .or_else(|| option_value(args, "--out"))
                .unwrap_or_else(|| ".genegis/dashboard.pmtiles".to_string());
            let minimum_zoom = option_value(args, "--min-zoom")
                .map(|value| {
                    value.parse::<u8>().unwrap_or_else(|_| {
                        eprintln!("--min-zoom must be an integer zoom level");
                        process::exit(1);
                    })
                })
                .unwrap_or(7);
            let maximum_zoom = option_value(args, "--max-zoom")
                .map(|value| {
                    value.parse::<u8>().unwrap_or_else(|_| {
                        eprintln!("--max-zoom must be an integer zoom level");
                        process::exit(1);
                    })
                })
                .unwrap_or(11);
            let result = genegis_analysis::run_nagoya_population_density(
                genegis_analysis::default_nagoya_data_path(),
            )
            .unwrap_or_else(|error| {
                eprintln!("Dashboard export error: {error}");
                process::exit(1);
            });
            if let Some(parent) = std::path::Path::new(&output).parent() {
                if !parent.as_os_str().is_empty() {
                    let _ = std::fs::create_dir_all(parent);
                }
            }
            let report = match genegis_analysis::export_dashboard_pmtiles(
                &result,
                &output,
                &genegis_analysis::DashboardExportOptions {
                    minimum_zoom,
                    maximum_zoom,
                    ..Default::default()
                },
            ) {
                Ok(report) => report,
                Err(err) => {
                    eprintln!("Dashboard export error: {err}");
                    process::exit(1);
                }
            };
            println!("{}", serde_json::to_string_pretty(&report).expect("json"));
            if !report.verification_passed {
                process::exit(1);
            }
        }
        _ => {
            eprintln!("{TILE_USAGE}");
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
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&info.summary_json()).expect("json")
                    );
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
        let repo_plugins = Path::new(&manifest_dir).join("../../plugins");
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

            let prompt = collect_prompt(&args[1..]);
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

            if link_collab
                && !run.verification_passed
                && !run.plan_only
                && link_agent_failure_comment(&mut run)
            {
                eprintln!("Collab comment linked to agent run {}", run.id);
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
            let prompt = collect_prompt(&args[1..]);
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
                DEFAULT_AGENT_PLAN_PATH, run.id
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
                    eprintln!("No pending plan at {}: {err}", DEFAULT_AGENT_PLAN_PATH);
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
            let Some(id) = args
                .get(1)
                .and_then(|value| uuid::Uuid::parse_str(value).ok())
            else {
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
                    serde_json::to_string_pretty(&session.branches().expect("branches"))
                        .expect("json")
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
                            serde_json::to_string_pretty(&session.branches().expect("branches"))
                                .expect("json")
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
                serde_json::to_string_pretty(&session.summary_json().expect("summary"))
                    .expect("json")
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

fn handle_catalog(args: &[String]) {
    match args.first().map(String::as_str) {
        Some("endpoint") => handle_stac_endpoint_registry(&args[1..]),
        Some("stac") => match args.get(1).map(String::as_str) {
            Some("list") => {
                let collection = browse_alpha_stac_collection(&alpha_catalog());
                println!(
                    "{}",
                    serde_json::to_string_pretty(&collection.summary_json()).expect("json")
                );
            }
            Some("get") => {
                let Some(id) = args.get(2) else {
                    eprintln!("Usage: genegis catalog stac get ITEM_ID");
                    process::exit(1);
                };
                let item = bind_stac_item(&alpha_catalog(), id).unwrap_or_else(|err| {
                    eprintln!("STAC item error: {err}");
                    process::exit(1);
                });
                println!("{}", serde_json::to_string_pretty(&item).expect("json"));
            }
            Some("fetch") => {
                let Some(url) = args.get(2) else {
                    eprintln!("Usage: genegis catalog stac fetch URL");
                    process::exit(1);
                };
                let collection = fetch_stac_collection(url).unwrap_or_else(|err| {
                    eprintln!("STAC fetch error: {err}");
                    process::exit(1);
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&collection.summary_json()).expect("json")
                );
            }
            Some("import") => {
                let Some(url) = args.get(2) else {
                    eprintln!("Usage: genegis catalog stac import ITEM_URL");
                    process::exit(1);
                };
                let record = import_stac_item_url(url).unwrap_or_else(|err| {
                    eprintln!("STAC import error: {err}");
                    process::exit(1);
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&record.summary_json()).expect("json")
                );
            }
            Some("search") => handle_federated_stac_search(&args[2..]),
            _ => {
                eprintln!("Usage: genegis catalog stac list");
                eprintln!("       genegis catalog stac get ITEM_ID");
                eprintln!("       genegis catalog stac fetch URL");
                eprintln!("       genegis catalog stac import ITEM_URL");
                eprintln!("       genegis catalog stac search --endpoint ID=URL [OPTIONS]");
                eprintln!("       genegis catalog endpoint add|list|remove");
                process::exit(1);
            }
        },
        _ => {
            eprintln!("Usage: genegis catalog stac list");
            eprintln!("       genegis catalog stac get ITEM_ID");
            eprintln!("       genegis catalog stac fetch URL");
            eprintln!("       genegis catalog stac import ITEM_URL");
            eprintln!("       genegis catalog stac search --endpoint ID=URL [OPTIONS]");
            eprintln!("       genegis catalog endpoint add|list|remove");
            process::exit(1);
        }
    }
}

fn handle_stac_endpoint_registry(args: &[String]) {
    let path = catalog_registry_path_arg(args);
    let mut registry = EndpointRegistry::load(&path).unwrap_or_else(|error| {
        eprintln!("Endpoint registry error: {error}");
        process::exit(1);
    });

    match args.first().map(String::as_str) {
        Some("list") => {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "path": path,
                    "schema_version": registry.schema_version,
                    "updated_at": registry.updated_at,
                    "endpoints": registry.endpoints,
                    "command_count": registry.command_history.len(),
                    "workflow_count": registry.workflows.len(),
                    "provenance_count": registry.provenance.entries.len(),
                }))
                .expect("endpoint registry json")
            );
        }
        Some("add") => {
            let Some(id) = args.get(1) else {
                print_endpoint_registry_usage();
                process::exit(1);
            };
            let Some(url) = args.get(2) else {
                print_endpoint_registry_usage();
                process::exit(1);
            };
            let title = catalog_named_option(args, "--title")
                .map(str::to_string)
                .unwrap_or_else(|| id.clone());
            let (auth_kind, auth_env, auth_header) = parse_endpoint_authentication_options(args);
            let command = Command::RegisterStacEndpoint {
                endpoint_id: id.clone(),
                title,
                url: url.clone(),
                auth_kind,
                auth_env,
                auth_header,
            };
            registry
                .apply(
                    CommandEnvelope::new(CommandOrigin::Cli, command),
                    stac_endpoint_registry_template("register", id),
                )
                .and_then(|_| registry.save(&path))
                .unwrap_or_else(|error| {
                    eprintln!("Endpoint registry error: {error}");
                    process::exit(1);
                });
            println!(
                "{}",
                serde_json::to_string_pretty(registry.get(id).expect("registered endpoint"))
                    .expect("endpoint json")
            );
        }
        Some("remove") => {
            let Some(id) = args.get(1) else {
                print_endpoint_registry_usage();
                process::exit(1);
            };
            registry
                .apply(
                    CommandEnvelope::new(
                        CommandOrigin::Cli,
                        Command::RemoveStacEndpoint {
                            endpoint_id: id.clone(),
                        },
                    ),
                    stac_endpoint_registry_template("remove", id),
                )
                .and_then(|_| registry.save(&path))
                .unwrap_or_else(|error| {
                    eprintln!("Endpoint registry error: {error}");
                    process::exit(1);
                });
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "removed": id,
                    "path": path,
                }))
                .expect("remove json")
            );
        }
        _ => {
            print_endpoint_registry_usage();
            process::exit(1);
        }
    }
}

fn catalog_registry_path_arg(args: &[String]) -> PathBuf {
    catalog_named_option(args, "--registry")
        .map(PathBuf::from)
        .unwrap_or_else(endpoint_registry_path)
}

fn catalog_named_option<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|argument| argument == name)
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
}

fn parse_endpoint_authentication_options(
    args: &[String],
) -> (String, Option<String>, Option<String>) {
    let bearer = catalog_named_option(args, "--auth-bearer-env");
    let header = catalog_named_option(args, "--auth-header-env");
    match (bearer, header) {
        (Some(_), Some(_)) => {
            eprintln!("Choose only one authentication option");
            process::exit(1);
        }
        (Some(env_var), None) => ("bearer_env".into(), Some(env_var.into()), None),
        (None, Some(spec)) => {
            let (header, env_var) = spec.split_once('=').unwrap_or_else(|| {
                eprintln!("Invalid --auth-header-env; expected HEADER=ENV_VAR");
                process::exit(1);
            });
            (
                "header_env".into(),
                Some(env_var.into()),
                Some(header.into()),
            )
        }
        (None, None) => ("anonymous".into(), None, None),
    }
}

fn print_endpoint_registry_usage() {
    eprintln!(
        "Usage: genegis catalog endpoint add ID URL [--title TITLE] [--auth-bearer-env ENV_VAR | --auth-header-env HEADER=ENV_VAR] [--registry FILE]"
    );
    eprintln!("       genegis catalog endpoint list [--registry FILE]");
    eprintln!("       genegis catalog endpoint remove ID [--registry FILE]");
}

fn handle_federated_stac_search(args: &[String]) {
    let mut catalog = FederatedCatalog::new();
    let mut request = StacSearchRequest::default();
    let registry_path = catalog_registry_path_arg(args);
    let mut registry = EndpointRegistry::load(&registry_path).unwrap_or_else(|error| {
        eprintln!("Endpoint registry error: {error}");
        process::exit(1);
    });
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--endpoint" => {
                let value = require_catalog_option(args, index, "--endpoint ID=URL");
                let (id, url) = value.split_once('=').unwrap_or_else(|| {
                    eprintln!("Invalid --endpoint {value:?}; expected ID=URL");
                    process::exit(1);
                });
                if id.is_empty() || url.is_empty() {
                    eprintln!("Invalid --endpoint {value:?}; ID and URL must not be empty");
                    process::exit(1);
                }
                catalog.register(StacEndpoint::new(id, url));
                index += 2;
            }
            "--endpoint-id" => {
                let id = require_catalog_option(args, index, "--endpoint-id ID");
                let endpoint = registry.get(id).cloned().unwrap_or_else(|| {
                    eprintln!("Endpoint registry error: endpoint {id:?} not found");
                    process::exit(1);
                });
                catalog.register(endpoint);
                index += 2;
            }
            "--bbox" => {
                let value = require_catalog_option(args, index, "--bbox MINX,MINY,MAXX,MAXY");
                request.bbox = Some(parse_catalog_bbox(value));
                index += 2;
            }
            "--datetime" => {
                request.datetime =
                    Some(require_catalog_option(args, index, "--datetime VALUE").to_string());
                index += 2;
            }
            "--collection" => {
                request
                    .collections
                    .push(require_catalog_option(args, index, "--collection ID").to_string());
                index += 2;
            }
            "--limit" => {
                let value = require_catalog_option(args, index, "--limit COUNT");
                request.limit = Some(value.parse::<usize>().unwrap_or_else(|_| {
                    eprintln!("Invalid --limit {value:?}; expected a non-negative integer");
                    process::exit(1);
                }));
                index += 2;
            }
            "--registry" => {
                require_catalog_option(args, index, "--registry FILE");
                index += 2;
            }
            unknown => {
                eprintln!("Unknown STAC search option: {unknown}");
                print_stac_search_usage();
                process::exit(1);
            }
        }
    }

    if catalog.endpoints().is_empty() {
        catalog = registry.federated_catalog(&[]).unwrap_or_else(|error| {
            eprintln!("Endpoint registry error: {error}");
            process::exit(1);
        });
        if catalog.endpoints().is_empty() {
            eprintln!("No endpoints configured; add one or pass --endpoint ID=URL");
            print_stac_search_usage();
            process::exit(1);
        }
    }

    let endpoint_ids: Vec<_> = catalog
        .endpoints()
        .iter()
        .map(|endpoint| endpoint.id.clone())
        .collect();
    let envelope = CommandEnvelope::new(
        CommandOrigin::Cli,
        Command::SearchFederatedStac {
            endpoint_ids: endpoint_ids.clone(),
            bbox: request.bbox,
            datetime: request.datetime.clone(),
            collections: request.collections.clone(),
            limit: request.limit,
        },
    );
    let workflow = federated_stac_search_template(&endpoint_ids);
    let result = catalog.search(&request);
    registry
        .record_search(envelope, workflow, &result)
        .and_then(|_| registry.save(&registry_path))
        .unwrap_or_else(|error| {
            eprintln!("Endpoint registry provenance error: {error}");
            process::exit(1);
        });
    println!(
        "{}",
        serde_json::to_string_pretty(&result).expect("federated search json")
    );

    if result.successful_endpoints() == 0 {
        process::exit(1);
    }
}

fn require_catalog_option<'a>(args: &'a [String], index: usize, usage: &str) -> &'a str {
    args.get(index + 1).map(String::as_str).unwrap_or_else(|| {
        eprintln!("Missing value: {usage}");
        process::exit(1);
    })
}

fn parse_catalog_bbox(value: &str) -> [f64; 4] {
    let values: Vec<_> = value
        .split(',')
        .map(|part| part.trim().parse::<f64>())
        .collect();
    if values.len() != 4 || values.iter().any(Result::is_err) {
        eprintln!("Invalid --bbox {value:?}; expected MINX,MINY,MAXX,MAXY");
        process::exit(1);
    }
    let bbox = [
        *values[0].as_ref().expect("validated"),
        *values[1].as_ref().expect("validated"),
        *values[2].as_ref().expect("validated"),
        *values[3].as_ref().expect("validated"),
    ];
    if bbox[0] > bbox[2] || bbox[1] > bbox[3] {
        eprintln!("Invalid --bbox {value:?}; minimums must not exceed maximums");
        process::exit(1);
    }
    bbox
}

fn print_stac_search_usage() {
    eprintln!(
        "Usage: genegis catalog stac search [--endpoint ID=URL | --endpoint-id ID] [--registry FILE] [--bbox MINX,MINY,MAXX,MAXY] [--datetime VALUE] [--collection ID] [--limit COUNT]"
    );
}

fn handle_vector(args: &[String]) {
    match args.first().map(String::as_str) {
        Some("geoparquet") => match args.get(1).map(String::as_str) {
            Some("info") => {
                let Some(path) = args.get(2) else {
                    eprintln!(
                        "Usage: genegis vector geoparquet info PATH|URL [--row-group INDEX ... | --all-row-groups]"
                    );
                    process::exit(1);
                };
                if genegis_storage::is_remote_uri(path) {
                    let all_row_groups = args.iter().any(|arg| arg == "--all-row-groups");
                    let selected = parse_row_group_options(&args[3..]);
                    let options = GeoParquetReadOptions {
                        row_groups: if all_row_groups { None } else { Some(selected) },
                    };
                    let command_row_groups = options.row_groups.clone();
                    let report =
                        read_geoparquet_uri_with_options(path, options).unwrap_or_else(|err| {
                            eprintln!("GeoParquet error: {err}");
                            process::exit(1);
                        });
                    let command = CommandEnvelope::new(
                        CommandOrigin::Cli,
                        Command::ReadRemoteGeoParquet {
                            uri: path.clone(),
                            row_groups: command_row_groups.clone(),
                        },
                    );
                    let workflow =
                        remote_geoparquet_range_template(path, command_row_groups.as_deref());
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "command": command,
                            "workflow": workflow,
                            "report": report,
                        }))
                        .expect("geoparquet operation receipt")
                    );
                    return;
                }
                let dataset = read_geoparquet_uri(path).unwrap_or_else(|err| {
                    eprintln!("GeoParquet error: {err}");
                    process::exit(1);
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&geoparquet_summary(&dataset)).expect("json")
                );
            }
            Some("build-fixture") => {
                let status = process::Command::new("cargo")
                    .args([
                        "run",
                        "-p",
                        "genegis-vector",
                        "--example",
                        "write_nagoya_geoparquet",
                    ])
                    .status();
                match status {
                    Ok(status) if status.success() => {}
                    Ok(status) => {
                        eprintln!("Fixture build failed with status: {status}");
                        process::exit(status.code().unwrap_or(1));
                    }
                    Err(err) => {
                        eprintln!("Fixture build failed: {err}");
                        process::exit(1);
                    }
                }
            }
            _ => {
                eprintln!(
                    "Usage: genegis vector geoparquet info PATH|URL [--row-group INDEX ... | --all-row-groups]"
                );
                eprintln!("       genegis vector geoparquet build-fixture");
                process::exit(1);
            }
        },
        _ => {
            eprintln!(
                "Usage: genegis vector geoparquet info PATH|URL [--row-group INDEX ... | --all-row-groups]"
            );
            eprintln!("       genegis vector geoparquet build-fixture");
            process::exit(1);
        }
    }
}

fn parse_row_group_options(args: &[String]) -> Vec<usize> {
    let mut selected = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--row-group" => {
                let value = args.get(index + 1).unwrap_or_else(|| {
                    eprintln!("Missing --row-group INDEX");
                    process::exit(1);
                });
                selected.push(value.parse().unwrap_or_else(|_| {
                    eprintln!("Invalid row group index {value:?}");
                    process::exit(1);
                }));
                index += 2;
            }
            "--all-row-groups" => index += 1,
            unknown => {
                eprintln!("Unknown GeoParquet option: {unknown}");
                process::exit(1);
            }
        }
    }
    selected
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
                        run_cog_execute(REMOTE_COG_DEMO_ID);
                    } else {
                        print_workflow_json(&remote_cog_metadata_template());
                    }
                }
                "local-cog-demo" => {
                    if execute {
                        run_cog_execute(LOCAL_COG_DEMO_ID);
                    } else {
                        print_workflow_json(&local_cog_metadata_template());
                    }
                }
                "nagoya-geoparquet" => {
                    if execute {
                        run_geoparquet_execute(NAGOYA_WARDS_GEOPARQUET_ID);
                    } else {
                        print_workflow_json(&nagoya_geoparquet_template());
                    }
                }
                "nagoya-geoparquet-density" => {
                    if execute {
                        run_geoparquet_density_execute();
                    } else {
                        print_workflow_json(&nagoya_geoparquet_density_template());
                    }
                }
                "external-stac-demo" => {
                    if execute {
                        run_external_stac_execute();
                    } else {
                        print_workflow_json(&external_stac_fetch_template());
                    }
                }
                "dashboard-export-demo" => {
                    if execute {
                        run_dashboard_export_execute();
                    } else {
                        print_workflow_json(&dashboard_export_template());
                    }
                }
                "nagoya-flood-exposure" => {
                    if execute {
                        run_flood_exposure_execute();
                    } else {
                        print_workflow_json(&nagoya_flood_exposure_template());
                    }
                }
                "nagoya-xmin-city" => {
                    if execute {
                        run_xmin_city_execute();
                    } else {
                        print_workflow_json(&nagoya_xmin_city_template());
                    }
                }
                "nagoya-evacuation-access" => {
                    if execute {
                        run_evacuation_execute();
                    } else {
                        print_workflow_json(&nagoya_evacuation_template());
                    }
                }
                "sentinel-ndvi-timeseries" => {
                    if execute {
                        run_ndvi_execute();
                    } else {
                        print_workflow_json(&sentinel_ndvi_timeseries_template());
                    }
                }
                "copc-change-detect" => {
                    if execute {
                        run_change_execute();
                    } else {
                        print_workflow_json(&copc_change_detect_template());
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
                "Usage: genegis workflow run [nagoya-density|remote-cog-demo|local-cog-demo|nagoya-geoparquet|nagoya-geoparquet-density|external-stac-demo|dashboard-export-demo|
                                             nagoya-flood-exposure|nagoya-xmin-city] [--execute] [--html] [--png] [-o FILE]"
            );
            process::exit(1);
        }
    }
}

fn run_xmin_city_execute() {
    let plan_prompt = "名古屋市の15分都市アクセシビリティを表示";
    let plan = match genegis_ai::plan_with_config(plan_prompt, &Default::default()) {
        Ok(plan) => plan,
        Err(err) => {
            eprintln!("Planner error: {err}");
            process::exit(1);
        }
    };
    let result = match genegis_analysis::execute_from_plan_with_origin(
        plan_prompt,
        &plan,
        genegis_core::CommandOrigin::Cli,
    ) {
        Ok(result) => result,
        Err(err) => {
            eprintln!("Workflow error: {err}");
            process::exit(1);
        }
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&result.summary).expect("json")
    );
}

fn run_change_execute() {
    let plan_prompt = "2時期の点群から建物・植生の変化を抽出して検証";
    let plan = match genegis_ai::plan_with_config(plan_prompt, &Default::default()) {
        Ok(plan) => plan,
        Err(err) => {
            eprintln!("Planner error: {err}");
            process::exit(1);
        }
    };
    let result = match genegis_analysis::execute_from_plan_with_origin(
        plan_prompt,
        &plan,
        genegis_core::CommandOrigin::Cli,
    ) {
        Ok(result) => result,
        Err(err) => {
            eprintln!("Workflow error: {err}");
            process::exit(1);
        }
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&result.summary).expect("json")
    );
}

fn run_ndvi_execute() {
    let plan_prompt = "名古屋周辺のNDVI時系列をSentinel-2から作成して検証";
    let plan = match genegis_ai::plan_with_config(plan_prompt, &Default::default()) {
        Ok(plan) => plan,
        Err(err) => {
            eprintln!("Planner error: {err}");
            process::exit(1);
        }
    };
    let result = match genegis_analysis::execute_from_plan_with_origin(
        plan_prompt,
        &plan,
        genegis_core::CommandOrigin::Cli,
    ) {
        Ok(result) => result,
        Err(err) => {
            eprintln!("Workflow error: {err}");
            process::exit(1);
        }
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&result.summary).expect("json")
    );
}

fn run_evacuation_execute() {
    let plan_prompt = "名古屋市の洪水浸水リスクと避難所アクセシビリティを表示";
    let plan = match genegis_ai::plan_with_config(plan_prompt, &Default::default()) {
        Ok(plan) => plan,
        Err(err) => {
            eprintln!("Planner error: {err}");
            process::exit(1);
        }
    };
    let result = match genegis_analysis::execute_from_plan_with_origin(
        plan_prompt,
        &plan,
        genegis_core::CommandOrigin::Cli,
    ) {
        Ok(result) => result,
        Err(err) => {
            eprintln!("Workflow error: {err}");
            process::exit(1);
        }
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&result.summary).expect("json")
    );
}

fn run_flood_exposure_execute() {
    let plan_prompt = "名古屋市の洪水浸水リスクと人口曝露を表示";
    let plan = match genegis_ai::plan_with_config(plan_prompt, &Default::default()) {
        Ok(plan) => plan,
        Err(err) => {
            eprintln!("Planner error: {err}");
            process::exit(1);
        }
    };
    let result = match genegis_analysis::execute_from_plan_with_origin(
        plan_prompt,
        &plan,
        genegis_core::CommandOrigin::Cli,
    ) {
        Ok(result) => result,
        Err(err) => {
            eprintln!("Workflow error: {err}");
            process::exit(1);
        }
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&result.summary).expect("json")
    );
}

fn run_dashboard_export_execute() {
    let plan_prompt = "名古屋市の人口密度をPMTilesダッシュボードに書き出し";
    let plan = match genegis_ai::plan_with_config(plan_prompt, &Default::default()) {
        Ok(plan) => plan,
        Err(err) => {
            eprintln!("Planner error: {err}");
            process::exit(1);
        }
    };
    let result = match genegis_analysis::execute_from_plan(plan_prompt, &plan) {
        Ok(result) => result,
        Err(err) => {
            eprintln!("Workflow error: {err}");
            process::exit(1);
        }
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&result.summary).expect("json")
    );
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
  genegis bench equivalence --json                 Native/DuckDB/GDAL 20+ case corpus
  genegis storage fetch URL [--range START-END]    HTTP range-read smoke fetch
  genegis raster info PATH                         COG / GeoTIFF metadata JSON (local or URL)
  genegis tile export --dataset nagoya-density     Verified PMTiles dashboard bundle + provenance
  genegis workflow run nagoya-flood-exposure --execute  Flood × population exposure overlay (UC-1)
  genegis workflow run nagoya-xmin-city --execute   15-minute-city accessibility scores (UC-4)
  genegis workflow run nagoya-evacuation-access --execute  Flood-penalized evacuation routing (UC-1)
  genegis workflow run sentinel-ndvi-timeseries --execute NDVI time series from STAC epochs (UC-3)
  genegis workflow run copc-change-detect --execute   Point-cloud epoch change detection (UC-5)
  genegis pointcloud info PATH|URL                 COPC metadata JSON (local or HTTP range-read)
  genegis plugin list [DIR]                        List plugin manifests (default: ./plugins)
  genegis plugin info BUNDLE_DIR                   Show one plugin manifest + effective caps
  genegis plugin load BUNDLE_DIR                   Capability-gated WASM load smoke
  genegis catalog stac list                        Browse alpha STAC collection summary
  genegis catalog stac get ITEM_ID                 Export one catalog dataset as STAC Item
  genegis catalog stac fetch URL                   Fetch external STAC Collection summary
  genegis catalog stac import ITEM_URL             Import STAC Item into catalog overlay
  genegis catalog endpoint add|list|remove         Persist named STAC endpoints + provenance
  genegis catalog stac search --endpoint-id ID     Search registered federated STAC APIs
  genegis vector geoparquet info PATH              GeoParquet metadata JSON
  genegis vector geoparquet info URL --row-group 0 HTTP-range selected row-group read
  genegis vector geoparquet build-fixture          Write Nagoya wards GeoParquet fixture
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
  genegis capsule seal PATH                        Seal the north-star result into an open directory
  genegis capsule verify PATH [--policy FILE]      Verify capsule digests and trust offline
  genegis capsule diff OLD NEW                     Classify semantic capsule changes
  genegis capsule approve PATH --reviewer ID       Bind reviewer approval to all semantic digests
  genegis capsule check-approval PATH FILE         Reject stale approval objects
  genegis capsule review PATH [--diff OTHER] [--tui|--json]  Trust Debugger or stable review JSON
  genegis capsule export-standards PATH OUT        PROV, RO-Crate, OpenLineage, in-toto, OGC
  genegis capsule attest PATH --key FILE           Create Ed25519 DSSE/in-toto attestation
  genegis capsule verify-attestation PATH FILE --public-key FILE  Verify DSSE offline
  genegis capsule execute-ogc REQUEST.json         Execute local OGC API Processes fixture
  genegis bench equivalence [--json]               Native/DuckDB/GDAL conformance corpus
  genegis bench external [--json]                  GeoBenchX-derived strict artifact adapter
  genegis bench review --reviewer ID [-o FILE]     Interactive Gate-B diagnosis timing
  genegis bench trust-ux --human --reviewer-code ID --facilitator-code ID [-o FILE]
                                                    Phase-12 map-first human Trust UX session
  genegis bench trust-ux-aggregate --input FILE... [-o FILE]
                                                    Aggregate Gate-E sessions; automation excluded
  genegis workflow run nagoya-density              Print workflow graph JSON
  genegis workflow run nagoya-density --execute    Run MVP analysis pipeline
  genegis workflow run remote-cog-demo             Print remote COG metadata workflow JSON
  genegis workflow run remote-cog-demo --execute   Probe catalog COG over HTTP range-read
  genegis workflow run local-cog-demo --execute    Probe bundled local COG metadata (offline)
  genegis workflow run nagoya-geoparquet           Print GeoParquet verification workflow JSON
  genegis workflow run nagoya-geoparquet --execute Read bundled GeoParquet + verify 16 wards
  genegis workflow run nagoya-geoparquet-density --execute GeoParquet density + DuckDB verify
  genegis workflow run external-stac-demo --execute Fetch bundled sample STAC collection
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

fn run_cog_execute(dataset_id: &str) {
    let uri = match alpha_catalog().require(dataset_id) {
        Ok(record) => record.uri.clone(),
        Err(err) => {
            eprintln!("Catalog error: {err}");
            process::exit(1);
        }
    };

    match genegis_raster::read_cog_uri(&uri) {
        Ok(info) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&info.summary_json()).expect("json")
            );
        }
        Err(err) => {
            eprintln!("Raster error: {err}");
            process::exit(1);
        }
    }
}

fn run_geoparquet_execute(dataset_id: &str) {
    let uri = match alpha_catalog().require(dataset_id) {
        Ok(record) => record.uri.clone(),
        Err(err) => {
            eprintln!("Catalog error: {err}");
            process::exit(1);
        }
    };

    let dataset = match read_geoparquet_uri(&uri) {
        Ok(dataset) => dataset,
        Err(err) => {
            eprintln!("GeoParquet error: {err}");
            process::exit(1);
        }
    };

    let summary = geoparquet_summary(&dataset);
    println!("{}", serde_json::to_string_pretty(&summary).expect("json"));
    if summary
        .get("feature_count")
        .and_then(|value| value.as_u64())
        != Some(16)
    {
        eprintln!("GeoParquet verification: failed (expected 16 features)");
        process::exit(1);
    }
    eprintln!("GeoParquet verification: passed");
}

fn run_geoparquet_density_execute() {
    let result = match run_ask_pipeline_with_config_and_origin(
        "名古屋 GeoParquet 人口密度を表示",
        &PlannerConfig::default(),
        CommandOrigin::Cli,
    ) {
        Ok(result) => result,
        Err(err) => {
            eprintln!("Analysis failed: {err}");
            process::exit(1);
        }
    };
    eprintln!("DuckDB verification: passed");
    println!(
        "{}",
        serde_json::to_string_pretty(&result.summary).expect("json")
    );
}

fn run_external_stac_execute() {
    let url = genegis_catalog::repo_root()
        .join("examples/stac/sample-collection.json")
        .to_string_lossy()
        .into_owned();
    match fetch_stac_collection(&url) {
        Ok(collection) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&collection.summary_json()).expect("json")
            );
        }
        Err(err) => {
            eprintln!("STAC fetch error: {err}");
            process::exit(1);
        }
    }
}

fn run_nagoya_execute(export_html: bool, export_png: bool, output: Option<&Path>) {
    let result = match run_ask_pipeline_with_config_and_origin(
        "名古屋市の人口密度を表示",
        &PlannerConfig::default(),
        CommandOrigin::Cli,
    ) {
        Ok(r) => r,
        Err(err) => {
            eprintln!("Analysis failed: {err}");
            process::exit(1);
        }
    };
    eprintln!("DuckDB verification: passed");
    println!(
        "{}",
        serde_json::to_string_pretty(&result.summary).expect("json")
    );
    write_exports(export_html, export_png, output, &result.html, &result.png);
}
