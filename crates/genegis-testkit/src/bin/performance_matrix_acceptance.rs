use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use std::time::Instant;

use chrono::Utc;
use genegis_analysis::{
    evaluate_performance_matrix_workflow, run_ask_pipeline, verify_gpu_scene_acceptance_receipt,
    GpuSceneAcceptanceReceipt,
};
use genegis_core::{
    DeploymentClass, PerformanceDimension, PerformanceEnvironment, PerformanceMatrixProfile,
    PerformanceMatrixVerdict, PerformanceMeasurement, PerformanceMeasurementStatus,
};
use genegis_testkit::NORTH_STAR_PROMPT;
use genegis_testkit::{verify_managed_cloud_range_receipt, ManagedCloudRangeReceipt};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const CPU_ITERATIONS: u32 = 5;
const CONCURRENCY: u32 = 4;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GpuSampleSet {
    schema_version: String,
    minimum_samples: usize,
    aggregation: GpuAggregation,
    receipts: Vec<GpuSampleReference>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GpuAggregation {
    first_frame: String,
    steady_state_fps: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GpuSampleReference {
    path: String,
    receipt_digest: String,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Performance matrix acceptance failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let profile_path = required_path(&mut args, "profile.json")?;
    let gpu_sample_set_path = required_path(&mut args, "gpu-sample-set.json")?;
    let output_path = required_path(&mut args, "output.json")?;
    let managed_network_receipt_path = args.next().map(PathBuf::from);
    if args.next().is_some() {
        return Err(
            "usage: performance_matrix_acceptance <profile.json> <gpu-sample-set.json> <output.json> [managed-network-receipt.json]"
                .into(),
        );
    }
    if output_path.exists() {
        return Err(format!(
            "output already exists and will not be overwritten: {}",
            output_path.display()
        ));
    }
    let profile: PerformanceMatrixProfile = read_json(&profile_path)?;
    let managed_network_receipt = match (
        profile.deployment_class,
        managed_network_receipt_path.as_deref(),
    ) {
        (DeploymentClass::ManagedCloud, Some(path)) => {
            let receipt: ManagedCloudRangeReceipt = read_json(path)?;
            verify_managed_cloud_range_receipt(&receipt)?;
            Some(receipt)
        }
        (DeploymentClass::ManagedCloud, None) => {
            return Err("managed-cloud profile requires a sealed managed-network receipt".into());
        }
        (_, Some(_)) => {
            return Err(
                "managed-network receipt is admissible only for a managed-cloud profile".into(),
            );
        }
        (_, None) => None,
    };
    let gpu_sample_set_bytes = std::fs::read(&gpu_sample_set_path)
        .map_err(|error| format!("read {}: {error}", gpu_sample_set_path.display()))?;
    let gpu_sample_set: GpuSampleSet = serde_json::from_slice(&gpu_sample_set_bytes)
        .map_err(|error| format!("parse {}: {error}", gpu_sample_set_path.display()))?;
    let gpu_receipts = load_gpu_sample_set(&gpu_sample_set)?;
    let gpu_receipt = &gpu_receipts[0];
    let gpu_evidence_digest = format!("sha256:{:x}", Sha256::digest(&gpu_sample_set_bytes));
    if gpu_receipt.build_digest != profile.build_digest {
        return Err("GPU receipt and performance profile build digests differ".into());
    }
    if let Some(network) = &managed_network_receipt {
        if network.profile_id != profile.id
            || network.dataset_digest != profile.dataset_digest
            || network.build_digest != profile.build_digest
            || network.os != gpu_receipt.os
            || network.cpu != gpu_receipt.cpu
        {
            return Err("managed-network, GPU, profile, or runtime identity differs".into());
        }
    }

    run_north_star_once()?;
    let cpu_samples = measure_cpu()?;
    let concurrency_samples = measure_concurrency()?;
    let population_path =
        Path::new("examples/nagoya-population-density/data/nagoya-population-2020.json");
    let population = std::fs::read(population_path)
        .map_err(|error| format!("read {}: {error}", population_path.display()))?;
    let population_digest = format!("sha256:{:x}", Sha256::digest(&population));
    if population_digest != profile.dataset_digest {
        return Err(format!(
            "profile dataset digest does not match {}",
            population_path.display()
        ));
    }

    let observed_at = Utc::now().to_rfc3339();
    let gpu_first_frame_p95_ms = p95(&gpu_receipts
        .iter()
        .map(|receipt| receipt.benchmark.first_frame_ns)
        .collect::<Vec<_>>()) as f64
        / 1_000_000.0;
    let gpu_minimum_fps = gpu_receipts
        .iter()
        .map(|receipt| receipt.benchmark.steady_state_fps)
        .fold(f64::INFINITY, f64::min);
    let network_profile = match profile.deployment_class {
        DeploymentClass::LocalFirst => "local-filesystem-no-http",
        DeploymentClass::ManagedCloud => "managed-cloud-http-range",
        DeploymentClass::AirGapped => "air-gapped-network-disabled",
    };
    let base_environment = PerformanceEnvironment {
        os: gpu_receipt.os.clone(),
        cpu: gpu_receipt.cpu.clone(),
        gpu: Some(format!(
            "{} ({})",
            gpu_receipt.benchmark.adapter, gpu_receipt.benchmark.backend
        )),
        network_profile: network_profile.into(),
        logical_concurrency: 1,
        build_digest: profile.build_digest.clone(),
    };
    let mut measurements = Vec::with_capacity(profile.metrics.len());
    for budget in &profile.metrics {
        let fixture_digest = budget
            .fixture_digest
            .clone()
            .unwrap_or_else(|| profile.dataset_digest.clone());
        let mut environment = base_environment.clone();
        let mut evidence_digest = None;
        let status = match budget.dimension {
            PerformanceDimension::Cpu => measured(p95_ms(&cpu_samples), CPU_ITERATIONS),
            PerformanceDimension::Gpu if budget.id == "gpu.first_frame_p95_ms" => {
                measured(gpu_first_frame_p95_ms, gpu_receipts.len() as u32)
            }
            PerformanceDimension::Gpu if budget.id == "gpu.steady_state_fps" => {
                measured(gpu_minimum_fps, gpu_receipts.len() as u32)
            }
            PerformanceDimension::Dataset => measured(population.len() as f64, 1),
            PerformanceDimension::Network
                if profile.deployment_class == DeploymentClass::ManagedCloud =>
            {
                measured(
                    managed_network_receipt
                        .as_ref()
                        .expect("managed receipt admitted above")
                        .requests
                        .len() as f64,
                    1,
                )
            }
            PerformanceDimension::Network => measured(0.0, 1),
            PerformanceDimension::Concurrency => {
                environment.logical_concurrency = CONCURRENCY;
                measured(
                    p95_ms(&concurrency_samples),
                    concurrency_samples.len() as u32,
                )
            }
            _ => {
                return Err(format!("unsupported performance metric: {}", budget.id));
            }
        };
        if budget.dimension == PerformanceDimension::Gpu {
            evidence_digest = Some(gpu_evidence_digest.clone());
        } else if budget.dimension == PerformanceDimension::Network {
            evidence_digest = managed_network_receipt
                .as_ref()
                .map(|receipt| receipt.receipt_digest.clone());
        }
        measurements.push(PerformanceMeasurement {
            metric_id: budget.id.clone(),
            fixture_digest,
            evidence_digest,
            observed_at: observed_at.clone(),
            environment,
            status,
        });
    }
    let receipt = evaluate_performance_matrix_workflow(profile, measurements)
        .map_err(|error| error.to_string())?;
    let bytes = serde_json::to_vec_pretty(&receipt)
        .map_err(|error| format!("serialize matrix receipt: {error}"))?;
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    use std::io::Write;
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
    match receipt.matrix.verdict {
        PerformanceMatrixVerdict::Pass => Ok(()),
        PerformanceMatrixVerdict::Fail => std::process::exit(2),
        PerformanceMatrixVerdict::Pending => std::process::exit(3),
    }
}

fn load_gpu_sample_set(
    sample_set: &GpuSampleSet,
) -> Result<Vec<GpuSceneAcceptanceReceipt>, String> {
    if sample_set.schema_version != "1.0.0"
        || sample_set.minimum_samples < 5
        || sample_set.receipts.len() < sample_set.minimum_samples
        || sample_set.aggregation.first_frame != "nearest_rank_p95"
        || sample_set.aggregation.steady_state_fps != "minimum"
    {
        return Err("GPU sample set schema, size, or aggregation policy is invalid".into());
    }
    let mut receipts = Vec::with_capacity(sample_set.receipts.len());
    for reference in &sample_set.receipts {
        let receipt: GpuSceneAcceptanceReceipt = read_json(Path::new(&reference.path))?;
        verify_gpu_scene_acceptance_receipt(&receipt).map_err(|error| error.to_string())?;
        if receipt.receipt_digest != reference.receipt_digest
            || receipt.verdict != genegis_analysis::GpuAcceptanceVerdict::Pass
        {
            return Err(format!(
                "GPU sample reference is not a sealed pass: {}",
                reference.path
            ));
        }
        receipts.push(receipt);
    }
    let identity = &receipts[0];
    if receipts.iter().any(|receipt| {
        receipt.build_digest != identity.build_digest
            || receipt.executable_digest != identity.executable_digest
            || receipt.build_profile != "release"
            || receipt.copc_digest != identity.copc_digest
            || receipt.lod1_digest != identity.lod1_digest
            || receipt.os != identity.os
            || receipt.cpu != identity.cpu
            || receipt.benchmark.adapter != identity.benchmark.adapter
            || receipt.benchmark.backend != identity.benchmark.backend
    }) {
        return Err("GPU sample receipts do not share one artifact and hardware identity".into());
    }
    Ok(receipts)
}

fn required_path(args: &mut impl Iterator<Item = String>, label: &str) -> Result<PathBuf, String> {
    args.next()
        .map(PathBuf::from)
        .ok_or_else(|| format!("missing required argument: {label}"))
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    serde_json::from_slice(
        &std::fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?,
    )
    .map_err(|error| format!("parse {}: {error}", path.display()))
}

fn measured(value: f64, iterations: u32) -> PerformanceMeasurementStatus {
    PerformanceMeasurementStatus::Measured { value, iterations }
}

fn run_north_star_once() -> Result<(), String> {
    let result = run_ask_pipeline(NORTH_STAR_PROMPT).map_err(|error| error.to_string())?;
    if !result.duckdb_verified {
        return Err("north-star pipeline did not produce DuckDB verification".into());
    }
    Ok(())
}

fn measure_cpu() -> Result<Vec<u64>, String> {
    let mut samples = Vec::with_capacity(CPU_ITERATIONS as usize);
    for _ in 0..CPU_ITERATIONS {
        let started = Instant::now();
        run_north_star_once()?;
        samples.push(nanos(started));
    }
    Ok(samples)
}

fn measure_concurrency() -> Result<Vec<u64>, String> {
    let barrier = Arc::new(Barrier::new(CONCURRENCY as usize));
    let handles = (0..CONCURRENCY)
        .map(|_| {
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                let started = Instant::now();
                run_north_star_once()?;
                Ok::<u64, String>(nanos(started))
            })
        })
        .collect::<Vec<_>>();
    handles
        .into_iter()
        .map(|handle| {
            handle
                .join()
                .map_err(|_| "concurrent north-star worker panicked".to_string())?
        })
        .collect()
}

fn p95_ms(samples: &[u64]) -> f64 {
    p95(samples) as f64 / 1_000_000.0
}

fn p95(samples: &[u64]) -> u64 {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let rank = ((sorted.len() as f64) * 0.95).ceil().max(1.0) as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

fn nanos(started: Instant) -> u64 {
    started.elapsed().as_nanos().min(u64::MAX as u128) as u64
}
