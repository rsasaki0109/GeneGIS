use std::path::{Path, PathBuf};

use chrono::Utc;
use genegis_analysis::{
    run_gpu_scene_acceptance_workflow, GpuAcceptanceVerdict, GpuSceneAcceptanceRequest,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

fn main() {
    if let Err(error) = run() {
        eprintln!("GPU acceptance failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let manifest_path = PathBuf::from(args.next().unwrap_or_else(|| {
        "examples/nagoya-population-density/data/nagoya-scene-fixture-manifest.json".into()
    }));
    let output_path = args.next().map(PathBuf::from);
    let measured_frames = args
        .next()
        .map(|value| value.parse::<u32>())
        .transpose()
        .map_err(|error| format!("invalid frame count: {error}"))?
        .unwrap_or(120);
    if args.next().is_some() {
        return Err("usage: gpu_scene_acceptance [manifest.json] [receipt.json] [frames]".into());
    }
    let manifest: Value = serde_json::from_slice(
        &std::fs::read(&manifest_path)
            .map_err(|error| format!("read {}: {error}", manifest_path.display()))?,
    )
    .map_err(|error| format!("parse {}: {error}", manifest_path.display()))?;
    let request = GpuSceneAcceptanceRequest {
        copc_path: field(&manifest, "/copc/path")?.into(),
        lod1_path: field(&manifest, "/lod1/path")?.into(),
        copc_digest: field(&manifest, "/copc/sha256")?.into(),
        lod1_digest: field(&manifest, "/lod1/sha256")?.into(),
        build_digest: sha256_file(Path::new("Cargo.lock"))?,
        executable_digest: sha256_file(
            &std::env::current_exe().map_err(|error| format!("current executable: {error}"))?,
        )?,
        build_profile: std::env::var("GENEGIS_BUILD_PROFILE")
            .map_err(|_| "GENEGIS_BUILD_PROFILE must be set by the acceptance runner")?,
        observed_at: Utc::now().to_rfc3339(),
        os: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        cpu: std::env::var("PROCESSOR_IDENTIFIER")
            .unwrap_or_else(|_| "processor-identity-unavailable".into()),
        width: 1280,
        height: 720,
        measured_frames,
        first_frame_seconds_maximum: number(
            &manifest,
            "/acceptance_contract/first_frame_seconds_maximum",
        )?,
        steady_state_fps_minimum: number(
            &manifest,
            "/acceptance_contract/steady_state_fps_minimum",
        )?,
    };
    let receipt = run_gpu_scene_acceptance_workflow(request).map_err(|error| error.to_string())?;
    let json = serde_json::to_string_pretty(&receipt)
        .map_err(|error| format!("serialize receipt: {error}"))?;
    if let Some(path) = output_path {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("create {}: {error}", parent.display()))?;
        }
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| format!("create new receipt {}: {error}", path.display()))?;
        file.write_all(format!("{json}\n").as_bytes())
            .map_err(|error| format!("write {}: {error}", path.display()))?;
        println!("{}", path.display());
    } else {
        println!("{json}");
    }
    if receipt.verdict == GpuAcceptanceVerdict::Fail {
        std::process::exit(2);
    }
    Ok(())
}

fn field<'a>(manifest: &'a Value, pointer: &str) -> Result<&'a str, String> {
    manifest
        .pointer(pointer)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("manifest field {pointer} is missing or not a string"))
}

fn number(manifest: &Value, pointer: &str) -> Result<f64, String> {
    manifest
        .pointer(pointer)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("manifest field {pointer} is missing or not a number"))
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}
