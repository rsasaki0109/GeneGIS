//! Hardware-bound COPC/LOD1 WebGPU acceptance through Command + Workflow.

use std::sync::Mutex;

use genegis_core::{
    Command, CommandBus, CommandEnvelope, CommandOrigin, InputSnapshot, Project, WorkflowExecution,
    WorkflowExecutionContext, WorkflowExecutionError, WorkflowExecutionEvent, WorkflowExecutor,
};
use genegis_crs::{CoordinateUnit, Crs, SourceSnapshot};
use genegis_render::{
    benchmark_scene3d_headless, BuildingLod1, OrbitCamera, Scene3d, Scene3dBenchmark, SceneSource,
};
use genegis_workflow::{scene3d_copc_lod1_template, GeoWorkflow};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::AnalysisError;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Lod1Fixture {
    schema_version: String,
    crs: Crs,
    coordinate_unit: CoordinateUnit,
    vertical_unit: String,
    source: serde_json::Value,
    buildings: Vec<Lod1Building>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Lod1Building {
    id: String,
    footprint: Vec<[f64; 2]>,
    base_z: f64,
    height: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuAcceptanceVerdict {
    Pass,
    Fail,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GpuSceneAcceptanceRequest {
    pub copc_path: String,
    pub lod1_path: String,
    pub copc_digest: String,
    pub lod1_digest: String,
    pub build_digest: String,
    pub executable_digest: String,
    pub build_profile: String,
    pub observed_at: String,
    pub os: String,
    pub cpu: String,
    pub width: u32,
    pub height: u32,
    pub measured_frames: u32,
    pub first_frame_seconds_maximum: f64,
    pub steady_state_fps_minimum: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GpuSceneAcceptanceReceipt {
    pub schema_version: String,
    pub command_id: String,
    pub workflow_digest: String,
    pub copc_digest: String,
    pub lod1_digest: String,
    pub build_digest: String,
    pub executable_digest: String,
    pub build_profile: String,
    pub observed_at: String,
    pub os: String,
    pub cpu: String,
    pub crs: String,
    pub coordinate_unit: String,
    pub vertical_unit: String,
    pub benchmark: Scene3dBenchmark,
    pub first_frame_seconds_maximum: f64,
    pub steady_state_fps_minimum: f64,
    pub verdict: GpuAcceptanceVerdict,
    pub regressions: Vec<String>,
    pub receipt_digest: String,
}

/// Recompute a persisted GPU receipt and reject identity, metric, verdict, or digest drift.
pub fn verify_gpu_scene_acceptance_receipt(
    receipt: &GpuSceneAcceptanceReceipt,
) -> Result<(), AnalysisError> {
    if receipt.schema_version != "1.2.0"
        || receipt.command_id.trim().is_empty()
        || receipt.workflow_digest.trim().is_empty()
        || !valid_digest(&receipt.copc_digest)
        || !valid_digest(&receipt.lod1_digest)
        || !valid_digest(&receipt.build_digest)
        || !valid_digest(&receipt.executable_digest)
        || !matches!(receipt.build_profile.as_str(), "debug" | "release")
        || receipt.observed_at.trim().is_empty()
        || receipt.os.trim().is_empty()
        || receipt.cpu.trim().is_empty()
        || receipt.crs != "EPSG:6675"
        || receipt.coordinate_unit != "metres"
        || receipt.vertical_unit != "metres"
        || receipt.benchmark.adapter.trim().is_empty()
        || receipt.benchmark.backend.trim().is_empty()
        || receipt.benchmark.upload_bytes == 0
        || receipt.benchmark.point_count != 40_949
        || receipt.benchmark.building_count != 3
        || receipt.benchmark.measured_frames == 0
        || !receipt.benchmark.steady_state_fps.is_finite()
        || receipt.benchmark.steady_state_fps <= 0.0
        || !receipt.first_frame_seconds_maximum.is_finite()
        || receipt.first_frame_seconds_maximum <= 0.0
        || !receipt.steady_state_fps_minimum.is_finite()
        || receipt.steady_state_fps_minimum <= 0.0
    {
        return Err(AnalysisError::Message(
            "GPU acceptance receipt identity or metrics are invalid".into(),
        ));
    }
    let mut expected_regressions = Vec::new();
    if receipt.benchmark.first_frame_ns as f64 / 1_000_000_000.0
        > receipt.first_frame_seconds_maximum
    {
        expected_regressions.push("gpu.first_frame_seconds".to_string());
    }
    if receipt.benchmark.steady_state_fps < receipt.steady_state_fps_minimum {
        expected_regressions.push("gpu.steady_state_fps".to_string());
    }
    let expected_verdict = if expected_regressions.is_empty() {
        GpuAcceptanceVerdict::Pass
    } else {
        GpuAcceptanceVerdict::Fail
    };
    if receipt.regressions != expected_regressions || receipt.verdict != expected_verdict {
        return Err(AnalysisError::Message(
            "GPU acceptance verdict does not match measured budgets".into(),
        ));
    }
    let expected_digest = gpu_receipt_digest(receipt)?;
    if receipt.receipt_digest != expected_digest {
        return Err(AnalysisError::Message(format!(
            "GPU acceptance receipt digest mismatch: expected {}, observed {}",
            receipt.receipt_digest, expected_digest
        )));
    }
    Ok(())
}

struct GpuAcceptanceExecutor {
    request: GpuSceneAcceptanceRequest,
    receipt: Mutex<Option<GpuSceneAcceptanceReceipt>>,
}

impl WorkflowExecutor for GpuAcceptanceExecutor {
    fn execute(
        &self,
        _workflow: &GeoWorkflow,
        context: &WorkflowExecutionContext,
    ) -> Result<WorkflowExecution, WorkflowExecutionError> {
        let scene = load_scene(&self.request)
            .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        let benchmark = benchmark_scene3d_headless(
            &scene,
            self.request.width,
            self.request.height,
            self.request.measured_frames,
        )
        .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        let receipt = seal_gpu_acceptance(
            &self.request,
            context.command_id.to_string(),
            context.workflow_digest.to_string(),
            benchmark,
        )
        .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        let result_digest = receipt.receipt_digest.clone();
        *self.receipt.lock().map_err(|_| {
            WorkflowExecutionError::Failed("GPU acceptance receipt lock poisoned".into())
        })? = Some(receipt.clone());
        Ok(WorkflowExecution {
            result_digest,
            output: serde_json::json!({
                "verdict": receipt.verdict,
                "first_frame_ns": receipt.benchmark.first_frame_ns,
                "steady_state_fps": receipt.benchmark.steady_state_fps,
            }),
            evidence: serde_json::to_value(&receipt)
                .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?,
            events: vec![WorkflowExecutionEvent {
                kind: "gpu_scene_acceptance_measured".into(),
                source_uri: Some(self.request.copc_path.clone()),
                observed_at: context.command_timestamp,
                details: serde_json::json!({
                    "adapter": receipt.benchmark.adapter,
                    "backend": receipt.benchmark.backend,
                    "fixture_digest": receipt.copc_digest,
                }),
            }],
        })
    }
}

pub fn run_gpu_scene_acceptance_workflow(
    request: GpuSceneAcceptanceRequest,
) -> Result<GpuSceneAcceptanceReceipt, AnalysisError> {
    validate_request(&request)?;
    let crs = Crs::nagoya_projected();
    let copc_source = verified_source(
        &request.copc_path,
        &request.copc_digest,
        "nagoya-scene-copc-v1",
    )?;
    let lod1_source = verified_source(
        &request.lod1_path,
        &request.lod1_digest,
        "nagoya-scene-lod1-v1",
    )?;
    let workflow =
        scene3d_copc_lod1_template(copc_source.clone(), lod1_source.clone(), crs.clone());
    let workflow_digest = workflow
        .stable_digest()
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    let envelope = CommandEnvelope::new(
        CommandOrigin::Cli,
        Command::RunWorkflow {
            workflow_id: workflow.id,
        },
    )
    .with_workflow_digest(workflow_digest)
    .with_source_snapshot(copc_source.clone())
    .with_source_snapshot(lod1_source.clone())
    .with_input_snapshot(InputSnapshot::new("copc", copc_source).with_crs(crs.clone()))
    .with_input_snapshot(InputSnapshot::new("buildings", lod1_source).with_crs(crs));
    let executor = GpuAcceptanceExecutor {
        request,
        receipt: Mutex::new(None),
    };
    let mut project = Project::new("GPU Scene3D acceptance");
    let mut bus = CommandBus::new(project.clone());
    bus.register_workflow(workflow)
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    let execution = bus
        .apply_with_executor(&mut project, envelope, &executor)
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    let receipt = executor
        .receipt
        .into_inner()
        .map_err(|_| AnalysisError::Message("GPU acceptance receipt lock poisoned".into()))?
        .ok_or_else(|| AnalysisError::Message("GPU executor returned no receipt".into()))?;
    if execution.result_digest.as_deref() != Some(receipt.receipt_digest.as_str()) {
        return Err(AnalysisError::Message(
            "CommandBus and GPU acceptance receipt digests differ".into(),
        ));
    }
    Ok(receipt)
}

fn load_scene(request: &GpuSceneAcceptanceRequest) -> Result<Scene3d, AnalysisError> {
    verify_file(&request.copc_path, &request.copc_digest)?;
    verify_file(&request.lod1_path, &request.lod1_digest)?;
    let metadata = genegis_pointcloud::read_copc_path(&request.copc_path)
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    if metadata.crs != "EPSG:6675" || metadata.point_count != 40_949 {
        return Err(AnalysisError::Message(
            "GPU fixture COPC CRS or point count does not match the acceptance manifest".into(),
        ));
    }
    let cloud = genegis_pointcloud::read_point_cloud_path(&request.copc_path)
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    let lod1: Lod1Fixture = serde_json::from_slice(
        &std::fs::read(&request.lod1_path)
            .map_err(|error| AnalysisError::Message(format!("read LOD1 fixture: {error}")))?,
    )
    .map_err(|error| AnalysisError::Message(format!("parse LOD1 fixture: {error}")))?;
    if lod1.schema_version != "0.1.0"
        || lod1.crs != Crs::nagoya_projected()
        || lod1.coordinate_unit != CoordinateUnit::Metres
        || lod1.vertical_unit != "metres"
        || !lod1.source.is_object()
    {
        return Err(AnalysisError::Message(
            "LOD1 schema, CRS, units, or provenance is invalid".into(),
        ));
    }
    let bounds = cloud
        .bounds()
        .ok_or_else(|| AnalysisError::Message("GPU fixture COPC is empty".into()))?;
    let target = [
        (bounds[0] + bounds[3]) * 0.5,
        (bounds[1] + bounds[4]) * 0.5,
        (bounds[2] + bounds[5]) * 0.5,
    ];
    let radius = (bounds[3] - bounds[0])
        .max(bounds[4] - bounds[1])
        .max(bounds[5] - bounds[2])
        * 1.8;
    let copc_source = verified_source(
        &request.copc_path,
        &request.copc_digest,
        "nagoya-scene-copc-v1",
    )?;
    let lod1_source = verified_source(
        &request.lod1_path,
        &request.lod1_digest,
        "nagoya-scene-lod1-v1",
    )?;
    let scene = Scene3d {
        schema_version: "0.1.0".into(),
        crs: Crs::nagoya_projected(),
        coordinate_unit: CoordinateUnit::Metres,
        vertical_unit: "metres".into(),
        sources: vec![
            SceneSource {
                id: "copc".into(),
                role: "point_cloud".into(),
                snapshot: copc_source,
            },
            SceneSource {
                id: "lod1".into(),
                role: "building_height".into(),
                snapshot: lod1_source,
            },
        ],
        point_source_id: "copc".into(),
        points: cloud.points,
        buildings: lod1
            .buildings
            .into_iter()
            .map(|building| BuildingLod1 {
                id: building.id,
                footprint: building.footprint,
                base_z: building.base_z,
                height: building.height,
                height_source_id: "lod1".into(),
            })
            .collect(),
        pois: Vec::new(),
        camera: OrbitCamera {
            target,
            yaw_degrees: 30.0,
            pitch_degrees: 35.0,
            radius,
        },
    };
    scene
        .validate()
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    Ok(scene)
}

fn seal_gpu_acceptance(
    request: &GpuSceneAcceptanceRequest,
    command_id: String,
    workflow_digest: String,
    benchmark: Scene3dBenchmark,
) -> Result<GpuSceneAcceptanceReceipt, AnalysisError> {
    let first_frame_seconds = benchmark.first_frame_ns as f64 / 1_000_000_000.0;
    let mut regressions = Vec::new();
    if first_frame_seconds > request.first_frame_seconds_maximum {
        regressions.push("gpu.first_frame_seconds".into());
    }
    if benchmark.steady_state_fps < request.steady_state_fps_minimum {
        regressions.push("gpu.steady_state_fps".into());
    }
    let verdict = if regressions.is_empty() {
        GpuAcceptanceVerdict::Pass
    } else {
        GpuAcceptanceVerdict::Fail
    };
    let mut receipt = GpuSceneAcceptanceReceipt {
        schema_version: "1.2.0".into(),
        command_id,
        workflow_digest,
        copc_digest: request.copc_digest.clone(),
        lod1_digest: request.lod1_digest.clone(),
        build_digest: request.build_digest.clone(),
        executable_digest: request.executable_digest.clone(),
        build_profile: request.build_profile.clone(),
        observed_at: request.observed_at.clone(),
        os: request.os.clone(),
        cpu: request.cpu.clone(),
        crs: "EPSG:6675".into(),
        coordinate_unit: "metres".into(),
        vertical_unit: "metres".into(),
        benchmark,
        first_frame_seconds_maximum: request.first_frame_seconds_maximum,
        steady_state_fps_minimum: request.steady_state_fps_minimum,
        verdict,
        regressions,
        receipt_digest: String::new(),
    };
    receipt.receipt_digest = gpu_receipt_digest(&receipt)?;
    verify_gpu_scene_acceptance_receipt(&receipt)?;
    Ok(receipt)
}

fn validate_request(request: &GpuSceneAcceptanceRequest) -> Result<(), AnalysisError> {
    if !valid_digest(&request.copc_digest)
        || !valid_digest(&request.lod1_digest)
        || !valid_digest(&request.build_digest)
        || !valid_digest(&request.executable_digest)
        || !matches!(request.build_profile.as_str(), "debug" | "release")
        || request.observed_at.trim().is_empty()
        || request.os.trim().is_empty()
        || request.cpu.trim().is_empty()
        || request.width == 0
        || request.height == 0
        || request.measured_frames == 0
        || !request.first_frame_seconds_maximum.is_finite()
        || request.first_frame_seconds_maximum <= 0.0
        || !request.steady_state_fps_minimum.is_finite()
        || request.steady_state_fps_minimum <= 0.0
    {
        return Err(AnalysisError::Message(
            "GPU acceptance request identity or budgets are invalid".into(),
        ));
    }
    Ok(())
}

fn verified_source(
    path: &str,
    digest: &str,
    version: &str,
) -> Result<SourceSnapshot, AnalysisError> {
    let mut source = SourceSnapshot::from_uri(path, Some(digest), Some(version));
    source.license = Some("Apache-2.0 OR MIT".into());
    if !source.checksum_status.is_verified() {
        return Err(AnalysisError::Message(format!(
            "source digest mismatch for {path}"
        )));
    }
    Ok(source)
}

fn verify_file(path: &str, expected: &str) -> Result<(), AnalysisError> {
    let bytes = std::fs::read(path)
        .map_err(|error| AnalysisError::Message(format!("read fixture {path}: {error}")))?;
    let observed = format!("sha256:{:x}", Sha256::digest(bytes));
    if observed != expected {
        return Err(AnalysisError::Message(format!(
            "fixture digest mismatch for {path}: expected {expected}, observed {observed}"
        )));
    }
    Ok(())
}

fn digest<T: Serialize>(value: &T) -> Result<String, AnalysisError> {
    let bytes =
        serde_json::to_vec(value).map_err(|error| AnalysisError::Message(error.to_string()))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

#[derive(Serialize)]
struct GpuBenchmarkDigestPayload<'a> {
    adapter: &'a str,
    backend: &'a str,
    upload_bytes: u64,
    upload_ns: u64,
    point_count: usize,
    building_count: usize,
    first_frame_ns: u64,
    measured_frames: u32,
    steady_state_fps_bits: u64,
}

#[derive(Serialize)]
struct GpuReceiptDigestPayload<'a> {
    schema_version: &'a str,
    command_id: &'a str,
    workflow_digest: &'a str,
    copc_digest: &'a str,
    lod1_digest: &'a str,
    build_digest: &'a str,
    executable_digest: &'a str,
    build_profile: &'a str,
    observed_at: &'a str,
    os: &'a str,
    cpu: &'a str,
    crs: &'a str,
    coordinate_unit: &'a str,
    vertical_unit: &'a str,
    benchmark: GpuBenchmarkDigestPayload<'a>,
    first_frame_seconds_maximum_bits: u64,
    steady_state_fps_minimum_bits: u64,
    verdict: &'a GpuAcceptanceVerdict,
    regressions: &'a [String],
}

fn gpu_receipt_digest(receipt: &GpuSceneAcceptanceReceipt) -> Result<String, AnalysisError> {
    digest(&GpuReceiptDigestPayload {
        schema_version: &receipt.schema_version,
        command_id: &receipt.command_id,
        workflow_digest: &receipt.workflow_digest,
        copc_digest: &receipt.copc_digest,
        lod1_digest: &receipt.lod1_digest,
        build_digest: &receipt.build_digest,
        executable_digest: &receipt.executable_digest,
        build_profile: &receipt.build_profile,
        observed_at: &receipt.observed_at,
        os: &receipt.os,
        cpu: &receipt.cpu,
        crs: &receipt.crs,
        coordinate_unit: &receipt.coordinate_unit,
        vertical_unit: &receipt.vertical_unit,
        benchmark: GpuBenchmarkDigestPayload {
            adapter: &receipt.benchmark.adapter,
            backend: &receipt.benchmark.backend,
            upload_bytes: receipt.benchmark.upload_bytes,
            upload_ns: receipt.benchmark.upload_ns,
            point_count: receipt.benchmark.point_count,
            building_count: receipt.benchmark.building_count,
            first_frame_ns: receipt.benchmark.first_frame_ns,
            measured_frames: receipt.benchmark.measured_frames,
            steady_state_fps_bits: receipt.benchmark.steady_state_fps.to_bits(),
        },
        first_frame_seconds_maximum_bits: receipt.first_frame_seconds_maximum.to_bits(),
        steady_state_fps_minimum_bits: receipt.steady_state_fps_minimum.to_bits(),
        verdict: &receipt.verdict,
        regressions: &receipt.regressions,
    })
}

fn valid_digest(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> GpuSceneAcceptanceRequest {
        GpuSceneAcceptanceRequest {
            copc_path: "fixture.copc.laz".into(),
            lod1_path: "fixture-lod1.json".into(),
            copc_digest: "sha256:090ee2b1287303ef5994d60f10fbae55a08ebed04a385f3ebf5da1731a6c80e2"
                .into(),
            lod1_digest: "sha256:1683864d6b1af1e1613153b1242b800004ff665636714d15b555f3445ccf7848"
                .into(),
            build_digest: "sha256:70477dfefb18c8189ea00a0d14b5d67d4628d65e8be48d713e255ea6ff116f3c"
                .into(),
            executable_digest: format!("sha256:{}", "a".repeat(64)),
            build_profile: "release".into(),
            observed_at: "2026-08-26T12:00:00Z".into(),
            os: "test-os".into(),
            cpu: "test-cpu".into(),
            width: 1280,
            height: 720,
            measured_frames: 120,
            first_frame_seconds_maximum: 2.0,
            steady_state_fps_minimum: 30.0,
        }
    }

    fn benchmark(first_frame_ns: u64, fps: f64) -> Scene3dBenchmark {
        Scene3dBenchmark {
            adapter: "test-gpu".into(),
            backend: "vulkan".into(),
            upload_bytes: 1024,
            upload_ns: 100,
            point_count: 40_949,
            building_count: 3,
            first_frame_ns,
            measured_frames: 120,
            steady_state_fps: fps,
        }
    }

    #[test]
    fn seals_pass_and_fail_without_accepting_regression() {
        let pass = seal_gpu_acceptance(
            &request(),
            "command".into(),
            "sha256:workflow".into(),
            benchmark(1_500_000_000, 60.0),
        )
        .expect("pass receipt");
        assert_eq!(pass.verdict, GpuAcceptanceVerdict::Pass);
        assert!(pass.receipt_digest.starts_with("sha256:"));
        verify_gpu_scene_acceptance_receipt(&pass).expect("verify pass receipt");
        let round_trip: GpuSceneAcceptanceReceipt =
            serde_json::from_slice(&serde_json::to_vec(&pass).expect("serialize pass receipt"))
                .expect("deserialize pass receipt");
        verify_gpu_scene_acceptance_receipt(&round_trip).expect("verify round-trip receipt");

        let mut tampered = pass.clone();
        tampered.benchmark.steady_state_fps = 1.0;
        assert!(verify_gpu_scene_acceptance_receipt(&tampered).is_err());

        let fail = seal_gpu_acceptance(
            &request(),
            "command".into(),
            "sha256:workflow".into(),
            benchmark(2_500_000_000, 20.0),
        )
        .expect("fail receipt");
        assert_eq!(fail.verdict, GpuAcceptanceVerdict::Fail);
        assert_eq!(
            fail.regressions,
            vec!["gpu.first_frame_seconds", "gpu.steady_state_fps"]
        );
        verify_gpu_scene_acceptance_receipt(&fail).expect("verify fail receipt");
    }

    #[test]
    fn committed_release_hardware_receipt_verifies_against_current_fixtures_and_build() {
        let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let receipt_path = repository.join("docs/reports/phase-14-m1-gpu-hardware-receipt.json");
        let receipt: GpuSceneAcceptanceReceipt = serde_json::from_slice(
            &std::fs::read(&receipt_path).expect("read committed GPU receipt"),
        )
        .expect("parse committed GPU receipt");
        verify_gpu_scene_acceptance_receipt(&receipt).expect("verify committed GPU receipt");
        assert_eq!(receipt.verdict, GpuAcceptanceVerdict::Pass);
        verify_file(
            repository
                .join("examples/nagoya-population-density/data/nagoya-scene.copc.laz")
                .to_str()
                .expect("COPC path"),
            &receipt.copc_digest,
        )
        .expect("verify committed COPC");
        verify_file(
            repository
                .join("examples/nagoya-population-density/data/nagoya-scene-lod1.json")
                .to_str()
                .expect("LOD1 path"),
            &receipt.lod1_digest,
        )
        .expect("verify committed LOD1");
        let lock_bytes = std::fs::read(repository.join("Cargo.lock")).expect("read Cargo.lock");
        assert_eq!(
            receipt.build_digest,
            format!("sha256:{:x}", Sha256::digest(lock_bytes))
        );
    }
}
