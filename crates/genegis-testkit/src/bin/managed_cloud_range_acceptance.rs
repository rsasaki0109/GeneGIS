use std::io::Write;
use std::path::PathBuf;

use chrono::Utc;
use genegis_core::PerformanceMatrixProfile;
use genegis_testkit::collect_managed_cloud_range_receipt;

fn main() {
    if let Err(error) = run() {
        eprintln!("Managed-cloud range acceptance failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let profile_path = required_path(&mut args, "profile.json")?;
    let source_url = args
        .next()
        .ok_or_else(|| "missing required argument: source-url".to_string())?;
    let output_path = required_path(&mut args, "output.json")?;
    if args.next().is_some() {
        return Err(
            "usage: managed_cloud_range_acceptance <profile.json> <source-url> <output.json>"
                .into(),
        );
    }
    if output_path.exists() {
        return Err(format!(
            "output already exists and will not be overwritten: {}",
            output_path.display()
        ));
    }
    let profile: PerformanceMatrixProfile = serde_json::from_slice(
        &std::fs::read(&profile_path)
            .map_err(|error| format!("read {}: {error}", profile_path.display()))?,
    )
    .map_err(|error| format!("parse {}: {error}", profile_path.display()))?;
    let receipt = collect_managed_cloud_range_receipt(
        &profile,
        &source_url,
        Utc::now().to_rfc3339(),
        format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_else(|_| "unknown-cpu".into()),
    )?;
    let bytes = serde_json::to_vec_pretty(&receipt).map_err(|error| error.to_string())?;
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    let mut output = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&output_path)
        .map_err(|error| format!("create {}: {error}", output_path.display()))?;
    output
        .write_all(&bytes)
        .and_then(|_| output.write_all(b"\n"))
        .map_err(|error| format!("write {}: {error}", output_path.display()))?;
    println!("{}", output_path.display());
    println!("{}", receipt.receipt_digest);
    Ok(())
}

fn required_path(args: &mut impl Iterator<Item = String>, label: &str) -> Result<PathBuf, String> {
    args.next()
        .map(PathBuf::from)
        .ok_or_else(|| format!("missing required argument: {label}"))
}
