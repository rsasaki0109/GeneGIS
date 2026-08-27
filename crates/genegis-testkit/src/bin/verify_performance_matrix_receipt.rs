use std::path::{Path, PathBuf};

use genegis_analysis::{evaluate_performance_matrix_workflow, PerformanceMatrixWorkflowReceipt};
use genegis_core::PerformanceMatrixProfile;
use serde::de::DeserializeOwned;

fn main() {
    if let Err(error) = run() {
        eprintln!("Performance matrix receipt verification failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let profile_path = required_path(&mut args, "profile.json")?;
    let receipt_path = required_path(&mut args, "receipt.json")?;
    if args.next().is_some() {
        return Err(
            "usage: verify_performance_matrix_receipt <profile.json> <receipt.json>".into(),
        );
    }

    let profile: PerformanceMatrixProfile = read_json(&profile_path)?;
    let persisted: PerformanceMatrixWorkflowReceipt = read_json(&receipt_path)?;
    if persisted.command_id.trim().is_empty() {
        return Err("persisted command_id is empty".into());
    }

    let recomputed =
        evaluate_performance_matrix_workflow(profile, persisted.matrix.measurements.clone())
            .map_err(|error| format!("receipt recomputation failed: {error}"))?;
    if recomputed.workflow_digest != persisted.workflow_digest {
        return Err(format!(
            "workflow digest mismatch: persisted {:?}, recomputed {:?}",
            persisted.workflow_digest, recomputed.workflow_digest
        ));
    }
    if recomputed.matrix != persisted.matrix {
        return Err("persisted matrix does not match its independently recomputed receipt".into());
    }

    println!(
        "verified {}: profile={}, verdict={:?}, receipt_digest={}",
        receipt_path.display(),
        persisted.matrix.profile_id,
        persisted.matrix.verdict,
        persisted.matrix.receipt_digest
    );
    Ok(())
}

fn required_path(args: &mut impl Iterator<Item = String>, label: &str) -> Result<PathBuf, String> {
    args.next()
        .map(PathBuf::from)
        .ok_or_else(|| format!("missing required argument: {label}"))
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid JSON in {}: {error}", path.display()))
}
