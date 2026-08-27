use std::path::PathBuf;

use genegis_analysis::{verify_gpu_scene_acceptance_receipt, GpuSceneAcceptanceReceipt};

fn main() {
    if let Err(error) = run() {
        eprintln!("GPU receipt verification failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let path = PathBuf::from(
        args.next()
            .ok_or_else(|| "usage: verify_gpu_receipt <receipt.json>".to_string())?,
    );
    if args.next().is_some() {
        return Err("usage: verify_gpu_receipt <receipt.json>".into());
    }
    let receipt: GpuSceneAcceptanceReceipt = serde_json::from_slice(
        &std::fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))?,
    )
    .map_err(|error| format!("parse {}: {error}", path.display()))?;
    verify_gpu_scene_acceptance_receipt(&receipt).map_err(|error| error.to_string())?;
    println!("{}", receipt.receipt_digest);
    Ok(())
}
