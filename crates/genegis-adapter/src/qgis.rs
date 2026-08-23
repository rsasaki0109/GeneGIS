//! Typed QGIS Processing execution with constrained file mounts and no plugins.

use crate::{
    admit, AdapterInvocation, AdapterManifest, AdapterOperation, AdmissionFailure, BackendFamily,
    BackendIdentity, Capability, CapabilityPolicy, Determinism, EvidenceHook, SandboxEvidence,
    ADAPTER_MANIFEST_SCHEMA_VERSION,
};
use genegis_contract::{
    AggregationBasis, AxisOrder, GeoContract, GeometryKind, MeasureContract, MeasureKind,
    SpatialContract,
};
use genegis_crs::Crs;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;
use thiserror::Error;

/// Official QGIS image tag used to resolve the conformance runtime.
pub const QGIS_IMAGE_REFERENCE: &str = "docker.io/qgis/qgis:3.44-noble";

/// Immutable QGIS registry digest executed by the Phase 12 harness.
pub const QGIS_IMAGE_DIGEST: &str =
    "sha256:4389d5ed64d7fc89647ac5cea45f97274a7e85b35d8bb7869810be297667ed79";

const ADAPTER_ID: &str = "org.genegis.qgis-processing.sandbox";
const ADAPTER_VERSION: &str = "0.1.0";
const QGIS_VERSION: &str = "3.44.13-Solothurn";
const QGIS_CODE_REVISION: &str = "2c8a7782a96";
const GDAL_VERSION: &str = "3.8.4";
const PROJ_VERSION: &str = "9.4.0";
const GEOS_VERSION: &str = "3.12.1-CAPI-1.18.1";
const PROJECTED_SRID: u32 = 6675;
const WGS84_SRID: u32 = 4326;
const MAX_INPUT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_OUTPUT_BYTES: u64 = 128 * 1024 * 1024;

/// Repair algorithm selected for `native:fixgeometries`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QgisRepairMethod {
    /// Preserve linework and construct valid polygons from it.
    Linework,
    /// Use the GEOS structure method.
    Structure,
}

impl QgisRepairMethod {
    fn processing_value(self) -> &'static str {
        match self {
            Self::Linework => "0",
            Self::Structure => "1",
        }
    }
}

/// Typed QGIS Processing algorithms exposed by the initial adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum QgisOperation {
    /// Create a rectangular polygon grid in EPSG:6675.
    CreateGrid {
        /// Minimum X in metres.
        min_x: f64,
        /// Maximum X in metres.
        max_x: f64,
        /// Minimum Y in metres.
        min_y: f64,
        /// Maximum Y in metres.
        max_y: f64,
        /// Horizontal spacing in metres.
        horizontal_spacing: f64,
        /// Vertical spacing in metres.
        vertical_spacing: f64,
        /// CRS; v0 is fixed to EPSG:6675.
        srid: u32,
    },
    /// Buffer a reviewed GeoJSON input in EPSG:6675.
    Buffer {
        /// Host input file mounted read-only.
        input: PathBuf,
        /// Buffer distance in metres.
        distance: f64,
        /// Quarter-circle segment count.
        segments: u32,
    },
    /// Reproject reviewed EPSG:6675 GeoJSON to EPSG:4326.
    Reproject {
        /// Host input file mounted read-only.
        input: PathBuf,
        /// Target CRS; v0 is fixed to EPSG:4326.
        target_srid: u32,
    },
    /// Calculate polygon centroids from reviewed GeoJSON.
    Centroids {
        /// Host input file mounted read-only.
        input: PathBuf,
        /// Emit one centroid for every multipart member.
        all_parts: bool,
    },
    /// Repair invalid polygonal GeoJSON without an arbitrary expression.
    FixGeometries {
        /// Host input file mounted read-only.
        input: PathBuf,
        /// Reviewed GEOS repair strategy.
        method: QgisRepairMethod,
    },
}

impl QgisOperation {
    fn operation_id(&self) -> &'static str {
        match self {
            Self::CreateGrid { .. } => "qgis.native.create-grid",
            Self::Buffer { .. } => "qgis.native.buffer",
            Self::Reproject { .. } => "qgis.native.reproject-layer",
            Self::Centroids { .. } => "qgis.native.centroids",
            Self::FixGeometries { .. } => "qgis.native.fix-geometries",
        }
    }

    fn algorithm_id(&self) -> &'static str {
        match self {
            Self::CreateGrid { .. } => "native:creategrid",
            Self::Buffer { .. } => "native:buffer",
            Self::Reproject { .. } => "native:reprojectlayer",
            Self::Centroids { .. } => "native:centroids",
            Self::FixGeometries { .. } => "native:fixgeometries",
        }
    }

    fn input(&self) -> Option<&Path> {
        match self {
            Self::CreateGrid { .. } => None,
            Self::Buffer { input, .. }
            | Self::Reproject { input, .. }
            | Self::Centroids { input, .. }
            | Self::FixGeometries { input, .. } => Some(input),
        }
    }

    fn output_name(&self) -> &'static str {
        match self {
            Self::CreateGrid { .. } => "grid.geojson",
            Self::Buffer { .. } => "buffer.geojson",
            Self::Reproject { .. } => "reproject.geojson",
            Self::Centroids { .. } => "centroids.geojson",
            Self::FixGeometries { .. } => "fixed.geojson",
        }
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        let mut capabilities = BTreeSet::from([Capability::ProcessSpawn, Capability::FileWrite]);
        if self.input().is_some() {
            capabilities.insert(Capability::FileRead);
        }
        capabilities
    }

    fn sanitized_parameters(&self, input_digest: Option<&str>) -> Value {
        match self {
            Self::CreateGrid {
                min_x,
                max_x,
                min_y,
                max_y,
                horizontal_spacing,
                vertical_spacing,
                srid,
            } => json!({
                "min_x": min_x,
                "max_x": max_x,
                "min_y": min_y,
                "max_y": max_y,
                "horizontal_spacing": horizontal_spacing,
                "vertical_spacing": vertical_spacing,
                "srid": srid
            }),
            Self::Buffer {
                distance, segments, ..
            } => json!({
                "input_digest": input_digest,
                "distance": distance,
                "segments": segments
            }),
            Self::Reproject { target_srid, .. } => json!({
                "input_digest": input_digest,
                "target_srid": target_srid
            }),
            Self::Centroids { all_parts, .. } => json!({
                "input_digest": input_digest,
                "all_parts": all_parts
            }),
            Self::FixGeometries { method, .. } => json!({
                "input_digest": input_digest,
                "method": method
            }),
        }
    }

    fn process_args(&self, input_name: Option<&str>) -> Vec<String> {
        let output = format!("--OUTPUT=/output/{}", self.output_name());
        match self {
            Self::CreateGrid {
                min_x,
                max_x,
                min_y,
                max_y,
                horizontal_spacing,
                vertical_spacing,
                ..
            } => vec![
                "--TYPE=2".into(),
                format!("--EXTENT={min_x},{max_x},{min_y},{max_y} [EPSG:{PROJECTED_SRID}]"),
                format!("--HSPACING={horizontal_spacing}"),
                format!("--VSPACING={vertical_spacing}"),
                "--HOVERLAY=0".into(),
                "--VOVERLAY=0".into(),
                format!("--CRS=EPSG:{PROJECTED_SRID}"),
                output,
            ],
            Self::Buffer {
                distance, segments, ..
            } => vec![
                format!("--INPUT=/input/{}", input_name.expect("validated input")),
                format!("--DISTANCE={distance}"),
                format!("--SEGMENTS={segments}"),
                "--END_CAP_STYLE=0".into(),
                "--JOIN_STYLE=0".into(),
                "--MITER_LIMIT=2".into(),
                "--DISSOLVE=0".into(),
                "--SEPARATE_DISJOINT=0".into(),
                output,
            ],
            Self::Reproject { .. } => vec![
                format!("--INPUT=/input/{}", input_name.expect("validated input")),
                format!("--TARGET_CRS=EPSG:{WGS84_SRID}"),
                "--CONVERT_CURVED_GEOMETRIES=1".into(),
                output,
            ],
            Self::Centroids { all_parts, .. } => vec![
                format!("--INPUT=/input/{}", input_name.expect("validated input")),
                format!("--ALL_PARTS={}", u8::from(*all_parts)),
                output,
            ],
            Self::FixGeometries { method, .. } => vec![
                format!("--INPUT=/input/{}", input_name.expect("validated input")),
                format!("--METHOD={}", method.processing_value()),
                output,
            ],
        }
    }
}

/// Canonical summary derived independently from a QGIS GeoJSON artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QgisVectorSummary {
    /// Number of GeoJSON features.
    pub feature_count: u64,
    /// Sorted unique GeoJSON geometry types.
    pub geometry_types: Vec<String>,
    /// CRS name recorded by the GeoJSON driver.
    pub crs: String,
    /// Exact artifact byte count.
    pub byte_size: u64,
}

/// Evidence emitted by one successful QGIS Processing operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QgisReceipt {
    /// Adapter manifest digest admitted before process spawn.
    pub manifest_digest: String,
    /// Typed semantic operation identity.
    pub operation_id: String,
    /// Exact QGIS Processing algorithm identifier.
    pub algorithm_id: String,
    /// Exact pinned backend identity.
    pub backend: BackendIdentity,
    /// Input artifact digest, absent for generated grids.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_digest: Option<String>,
    /// Sanitized parameters that never include a host absolute path.
    pub parameters: Value,
    /// Digest of the complete machine-readable `qgis_process` report.
    pub process_report_digest: String,
    /// Digest of the actual output GeoJSON bytes.
    pub output_digest: String,
    /// Independently parsed output summary.
    pub output: QgisVectorSummary,
    /// Output filename relative to the approved output directory.
    pub output_name: String,
    /// Captured runtime warnings.
    pub warnings: Vec<String>,
    /// Enforced container controls.
    pub sandbox: SandboxEvidence,
    /// Measured wall time including admission and artifact validation.
    pub elapsed_ns: u64,
}

/// Failure at admission, path validation, runtime identity, or QGIS execution.
#[derive(Debug, Error)]
pub enum QgisError {
    /// Capability policy rejected execution before Docker was called.
    #[error("QGIS adapter admission failed: {0:?}")]
    Admission(Vec<AdmissionFailure>),
    /// Typed values disagree with the reviewed operation contract.
    #[error("QGIS operation contract failed: {0}")]
    Contract(String),
    /// Input or output path is unsafe, unsupported, or outside its budget.
    #[error("QGIS file boundary failed: {0}")]
    FileBoundary(String),
    /// Docker or local artifact I/O failed.
    #[error("QGIS sandbox I/O failed: {0}")]
    Io(#[from] std::io::Error),
    /// Image or runtime component identity differs from the manifest.
    #[error("QGIS runtime identity mismatch: {0}")]
    RuntimeIdentity(String),
    /// QGIS Processing returned non-zero.
    #[error("QGIS Processing failed with status {status}: {stderr}")]
    Processing {
        /// Container exit status, or -1 if unavailable.
        status: i32,
        /// Captured diagnostics.
        stderr: String,
    },
    /// Process report or GeoJSON output was malformed.
    #[error("QGIS output validation failed: {0}")]
    Output(String),
    /// JSON serialization failed.
    #[error("QGIS receipt serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Immutable QGIS Processing sandbox executor.
#[derive(Debug, Clone)]
pub struct QgisAdapter {
    manifest: AdapterManifest,
    policy: CapabilityPolicy,
}

impl Default for QgisAdapter {
    fn default() -> Self {
        let manifest = qgis_manifest();
        let policy = CapabilityPolicy::sandboxed_file_process(&manifest.adapter_id);
        Self { manifest, policy }
    }
}

impl QgisAdapter {
    /// Construct an adapter with an explicitly reviewed manifest and policy.
    pub fn new(manifest: AdapterManifest, policy: CapabilityPolicy) -> Self {
        Self { manifest, policy }
    }

    /// Return the reviewed QGIS Processing manifest.
    pub fn manifest(&self) -> &AdapterManifest {
        &self.manifest
    }

    /// Execute one typed native algorithm into an explicitly approved directory.
    pub fn execute(
        &self,
        operation: &QgisOperation,
        output_directory: &Path,
    ) -> Result<QgisReceipt, QgisError> {
        let started = Instant::now();
        validate_operation_contract(operation)?;
        let invocation = AdapterInvocation {
            adapter_id: self.manifest.adapter_id.clone(),
            adapter_version: self.manifest.adapter_version.clone(),
            operation_id: operation.operation_id().into(),
            operation_version: "1.0.0".into(),
            backend: self.manifest.backend.clone(),
            requested_capabilities: operation.capabilities(),
        };
        let admission = admit(&self.manifest, &invocation, &self.policy);
        if !admission.admitted || !admission.verification_eligible {
            return Err(QgisError::Admission(admission.failures));
        }
        validate_local_image(&self.manifest.backend)?;

        let output_directory = validate_output_directory(output_directory)?;
        let output_path = output_directory.join(operation.output_name());
        if output_path.exists() {
            return Err(QgisError::FileBoundary(format!(
                "refusing to overwrite existing output {}",
                output_path.display()
            )));
        }
        let input = operation.input().map(validate_input).transpose()?;
        let input_digest = input.as_ref().map(|input| input.digest.clone());
        let parameters = operation.sanitized_parameters(input_digest.as_deref());
        let input_name = input.as_ref().map(|input| input.file_name.as_str());

        let image = format!("qgis/qgis@{QGIS_IMAGE_DIGEST}");
        let output_mount = format!("{}:/output:rw", output_directory.display());
        let mut command = Command::new("docker");
        command.args([
            "run",
            "--rm",
            "--pull",
            "never",
            "--network",
            "none",
            "--read-only",
            "--user",
            "65534:65534",
            "--cap-drop",
            "ALL",
            "--security-opt",
            "no-new-privileges",
            "--pids-limit",
            "256",
            "--memory",
            "1g",
            "--cpus",
            "1",
            "--tmpfs",
            "/tmp:rw,nosuid,size=256m,mode=1777",
            "-e",
            "HOME=/tmp",
            "-e",
            "XDG_RUNTIME_DIR=/tmp/runtime",
            "-e",
            "QT_QPA_PLATFORM=offscreen",
            "-v",
            &output_mount,
        ]);
        let input_mount;
        if let Some(input) = input.as_ref() {
            input_mount = format!("{}:/input:ro", input.parent.display());
            command.args(["-v", &input_mount]);
        }
        command.args([
            "--entrypoint",
            "qgis_process",
            &image,
            "--json",
            "--no-python",
            "--skip-loading-plugins",
            "run",
            operation.algorithm_id(),
        ]);
        command.args(operation.process_args(input_name));
        let result = command.output()?;
        let stdout = String::from_utf8_lossy(&result.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&result.stderr).into_owned();
        if !result.status.success() {
            return Err(QgisError::Processing {
                status: result.status.code().unwrap_or(-1),
                stderr,
            });
        }
        let process_report: Value = serde_json::from_str(&stdout)
            .map_err(|error| QgisError::Output(format!("process report: {error}")))?;
        validate_runtime_report(&process_report)?;
        let bytes = fs::read(&output_path)?;
        if bytes.len() as u64 > MAX_OUTPUT_BYTES {
            return Err(QgisError::FileBoundary(format!(
                "output is {} bytes; limit is {MAX_OUTPUT_BYTES}",
                bytes.len()
            )));
        }
        let output = summarize_geojson(&bytes)?;
        let is_wgs84_output = matches!(operation, QgisOperation::Reproject { .. });
        let expected_srid = if is_wgs84_output {
            WGS84_SRID
        } else {
            PROJECTED_SRID
        };
        let crs_matches = output.crs.ends_with(&format!("::{expected_srid}"))
            || output.crs.ends_with(&format!(":{expected_srid}"))
            || (is_wgs84_output && output.crs == "urn:ogc:def:crs:OGC:1.3:CRS84");
        if !crs_matches {
            return Err(QgisError::Output(format!(
                "output CRS {:?} does not match EPSG:{expected_srid}",
                output.crs
            )));
        }
        let warnings = stderr
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect();
        let mut sandbox = SandboxEvidence::default();
        sandbox.pids_limit = 256;
        sandbox.memory_mib = 1024;
        sandbox.tmpfs_paths = vec!["/tmp".into()];

        Ok(QgisReceipt {
            manifest_digest: self.manifest.digest()?,
            operation_id: operation.operation_id().into(),
            algorithm_id: operation.algorithm_id().into(),
            backend: self.manifest.backend.clone(),
            input_digest,
            parameters,
            process_report_digest: digest_json(&process_report)?,
            output_digest: format!("sha256:{:x}", Sha256::digest(&bytes)),
            output,
            output_name: operation.output_name().into(),
            warnings,
            sandbox,
            elapsed_ns: started.elapsed().as_nanos().min(u64::MAX as u128) as u64,
        })
    }
}

struct ValidatedInput {
    parent: PathBuf,
    file_name: String,
    digest: String,
}

fn validate_operation_contract(operation: &QgisOperation) -> Result<(), QgisError> {
    let finite = |values: &[f64]| values.iter().all(|value| value.is_finite());
    let valid = match operation {
        QgisOperation::CreateGrid {
            min_x,
            max_x,
            min_y,
            max_y,
            horizontal_spacing,
            vertical_spacing,
            srid,
        } => {
            *srid == PROJECTED_SRID
                && finite(&[
                    *min_x,
                    *max_x,
                    *min_y,
                    *max_y,
                    *horizontal_spacing,
                    *vertical_spacing,
                ])
                && max_x > min_x
                && max_y > min_y
                && *horizontal_spacing > 0.0
                && *vertical_spacing > 0.0
                && ((*max_x - *min_x) / *horizontal_spacing)
                    * ((*max_y - *min_y) / *vertical_spacing)
                    <= 1_000_000.0
        }
        QgisOperation::Buffer {
            distance, segments, ..
        } => {
            distance.is_finite()
                && *distance > 0.0
                && *distance <= 10_000.0
                && (1..=100).contains(segments)
        }
        QgisOperation::Reproject { target_srid, .. } => *target_srid == WGS84_SRID,
        QgisOperation::Centroids { .. } | QgisOperation::FixGeometries { .. } => true,
    };
    if valid {
        Ok(())
    } else {
        Err(QgisError::Contract(format!(
            "{} parameters exceed its CRS or resource contract",
            operation.operation_id()
        )))
    }
}

fn validate_output_directory(path: &Path) -> Result<PathBuf, QgisError> {
    let canonical = path.canonicalize().map_err(|error| {
        QgisError::FileBoundary(format!("output directory {}: {error}", path.display()))
    })?;
    let metadata = fs::metadata(&canonical)?;
    if !metadata.is_dir() {
        return Err(QgisError::FileBoundary(format!(
            "output {} is not a directory",
            canonical.display()
        )));
    }
    #[cfg(unix)]
    if metadata.mode() & 0o002 == 0 {
        return Err(QgisError::FileBoundary(format!(
            "output {} is not writable by sandbox UID 65534",
            canonical.display()
        )));
    }
    if canonical.to_string_lossy().contains(':') {
        return Err(QgisError::FileBoundary(
            "output path contains a Docker mount delimiter".into(),
        ));
    }
    Ok(canonical)
}

fn validate_input(path: &Path) -> Result<ValidatedInput, QgisError> {
    let canonical = path
        .canonicalize()
        .map_err(|error| QgisError::FileBoundary(format!("input {}: {error}", path.display())))?;
    let metadata = fs::metadata(&canonical)?;
    if !metadata.is_file() || metadata.len() > MAX_INPUT_BYTES {
        return Err(QgisError::FileBoundary(format!(
            "input must be a regular GeoJSON no larger than {MAX_INPUT_BYTES} bytes"
        )));
    }
    let extension = canonical.extension().and_then(|value| value.to_str());
    if !matches!(extension, Some("geojson") | Some("json")) {
        return Err(QgisError::FileBoundary(
            "input extension must be .geojson or .json".into(),
        ));
    }
    let bytes = fs::read(&canonical)?;
    let summary = summarize_geojson(&bytes)?;
    if summary.feature_count == 0
        || summary
            .geometry_types
            .iter()
            .any(|kind| !matches!(kind.as_str(), "Polygon" | "MultiPolygon"))
    {
        return Err(QgisError::FileBoundary(
            "input must be a non-empty polygonal GeoJSON FeatureCollection".into(),
        ));
    }
    if !summary.crs.ends_with(&format!("::{PROJECTED_SRID}"))
        && !summary.crs.ends_with(&format!(":{PROJECTED_SRID}"))
    {
        return Err(QgisError::FileBoundary(format!(
            "input CRS {:?} is outside the EPSG:{PROJECTED_SRID} contract",
            summary.crs
        )));
    }
    let parent = canonical
        .parent()
        .ok_or_else(|| QgisError::FileBoundary("input has no canonical parent directory".into()))?;
    if parent.to_string_lossy().contains(':') {
        return Err(QgisError::FileBoundary(
            "input path contains a Docker mount delimiter".into(),
        ));
    }
    let file_name = canonical
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| QgisError::FileBoundary("input filename is not UTF-8".into()))?;
    Ok(ValidatedInput {
        parent: parent.into(),
        file_name: file_name.into(),
        digest: format!("sha256:{:x}", Sha256::digest(bytes)),
    })
}

fn summarize_geojson(bytes: &[u8]) -> Result<QgisVectorSummary, QgisError> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| QgisError::Output(format!("GeoJSON: {error}")))?;
    if value.get("type").and_then(Value::as_str) != Some("FeatureCollection") {
        return Err(QgisError::Output(
            "output is not a GeoJSON FeatureCollection".into(),
        ));
    }
    let features = value
        .get("features")
        .and_then(Value::as_array)
        .ok_or_else(|| QgisError::Output("GeoJSON features are missing".into()))?;
    let mut geometry_types = features
        .iter()
        .filter_map(|feature| feature.pointer("/geometry/type").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<Vec<_>>();
    geometry_types.sort();
    geometry_types.dedup();
    let crs = value
        .pointer("/crs/properties/name")
        .and_then(Value::as_str)
        .ok_or_else(|| QgisError::Output("GeoJSON CRS identity is missing".into()))?;
    Ok(QgisVectorSummary {
        feature_count: features.len() as u64,
        geometry_types,
        crs: crs.into(),
        byte_size: bytes.len() as u64,
    })
}

fn validate_local_image(backend: &BackendIdentity) -> Result<(), QgisError> {
    let image = format!("qgis/qgis@{}", backend.build_digest);
    let output = Command::new("docker")
        .args([
            "image",
            "inspect",
            &image,
            "--format",
            "{{json .RepoDigests}}",
        ])
        .output()?;
    let observed = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() || !observed.contains(&backend.build_digest) {
        return Err(QgisError::RuntimeIdentity(format!(
            "pinned image {} is unavailable or has drifted",
            backend.build_digest
        )));
    }
    Ok(())
}

fn validate_runtime_report(report: &Value) -> Result<(), QgisError> {
    for (pointer, expected) in [
        ("/qgis_version", QGIS_VERSION),
        ("/qgis_code_revision", QGIS_CODE_REVISION),
        ("/gdal_version", GDAL_VERSION),
        ("/geos_version", GEOS_VERSION),
    ] {
        if report.pointer(pointer).and_then(Value::as_str) != Some(expected) {
            return Err(QgisError::RuntimeIdentity(format!(
                "expected {pointer}={expected:?}, observed {:?}",
                report.pointer(pointer)
            )));
        }
    }
    let proj = report
        .get("proj_version")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !proj.contains(PROJ_VERSION) {
        return Err(QgisError::RuntimeIdentity(format!(
            "expected PROJ {PROJ_VERSION}, observed {proj:?}"
        )));
    }
    if report
        .pointer("/provider_details/name")
        .and_then(Value::as_str)
        != Some("QGIS (native c++)")
    {
        return Err(QgisError::RuntimeIdentity(
            "native C++ processing provider identity is missing".into(),
        ));
    }
    Ok(())
}

fn digest_json(value: &Value) -> Result<String, serde_json::Error> {
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(value)?)
    ))
}

/// Return the reviewed manifest for QGIS 3.44.13 native Processing.
pub fn qgis_manifest() -> AdapterManifest {
    let hooks = BTreeSet::from([
        EvidenceHook::InputDigests,
        EvidenceHook::OutputDigests,
        EvidenceHook::Parameters,
        EvidenceHook::Warnings,
        EvidenceHook::ComponentIdentity,
        EvidenceHook::EnvironmentDigest,
    ]);
    let operation = |operation_id: &str,
                     inputs: Vec<GeoContract>,
                     outputs: Vec<GeoContract>,
                     capabilities: BTreeSet<Capability>| AdapterOperation {
        operation_id: operation_id.into(),
        operation_version: "1.0.0".into(),
        inputs,
        outputs,
        capabilities,
        determinism: Determinism::ToleranceBounded,
        evidence_hooks: hooks.clone(),
        opaque: false,
    };
    let generated = BTreeSet::from([Capability::ProcessSpawn, Capability::FileWrite]);
    let transformed = BTreeSet::from([
        Capability::FileRead,
        Capability::FileWrite,
        Capability::ProcessSpawn,
    ]);
    AdapterManifest {
        schema_version: ADAPTER_MANIFEST_SCHEMA_VERSION.into(),
        adapter_id: ADAPTER_ID.into(),
        adapter_version: ADAPTER_VERSION.into(),
        backend: BackendIdentity {
            family: BackendFamily::QgisProcessing,
            engine_version: QGIS_VERSION.into(),
            build_digest: QGIS_IMAGE_DIGEST.into(),
            components: BTreeMap::from([
                ("qgis-code-revision".into(), QGIS_CODE_REVISION.into()),
                ("gdal".into(), GDAL_VERSION.into()),
                ("proj".into(), PROJ_VERSION.into()),
                ("geos".into(), GEOS_VERSION.into()),
            ]),
        },
        license: "GPL-2.0-or-later".into(),
        operations: vec![
            operation(
                "qgis.native.create-grid",
                vec![polygon_contract(
                    "qgis.grid.extent",
                    Crs::nagoya_projected(),
                )],
                vec![polygon_contract(
                    "qgis.grid.output",
                    Crs::nagoya_projected(),
                )],
                generated,
            ),
            operation(
                "qgis.native.buffer",
                vec![polygon_contract(
                    "qgis.buffer.input",
                    Crs::nagoya_projected(),
                )],
                vec![polygon_contract(
                    "qgis.buffer.output",
                    Crs::nagoya_projected(),
                )],
                transformed.clone(),
            ),
            operation(
                "qgis.native.reproject-layer",
                vec![polygon_contract(
                    "qgis.reproject.input",
                    Crs::nagoya_projected(),
                )],
                vec![polygon_contract("qgis.reproject.output", Crs::wgs84())],
                transformed.clone(),
            ),
            operation(
                "qgis.native.centroids",
                vec![polygon_contract(
                    "qgis.centroids.input",
                    Crs::nagoya_projected(),
                )],
                vec![point_contract(
                    "qgis.centroids.output",
                    Crs::nagoya_projected(),
                )],
                transformed.clone(),
            ),
            operation(
                "qgis.native.fix-geometries",
                vec![polygon_contract("qgis.fix.input", Crs::nagoya_projected())],
                vec![polygon_contract("qgis.fix.output", Crs::nagoya_projected())],
                transformed,
            ),
        ],
    }
}

fn polygon_contract(id: &str, crs: Crs) -> GeoContract {
    let axis = if crs == Crs::wgs84() {
        AxisOrder::LongitudeLatitude
    } else {
        AxisOrder::Xy
    };
    GeoContract::new(id)
        .with_spatial(SpatialContract::known(GeometryKind::Polygon, crs, axis))
        .with_measure(MeasureContract::simple(
            MeasureKind::Geometry,
            "geometry",
            AggregationBasis::None,
        ))
}

fn point_contract(id: &str, crs: Crs) -> GeoContract {
    GeoContract::new(id)
        .with_spatial(SpatialContract::known(
            GeometryKind::Point,
            crs,
            AxisOrder::Xy,
        ))
        .with_measure(MeasureContract::simple(
            MeasureKind::Geometry,
            "geometry",
            AggregationBasis::None,
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qgis_manifest_exposes_only_five_native_typed_algorithms() {
        let manifest = qgis_manifest();
        assert!(manifest.validate().is_empty());
        assert_eq!(manifest.operations.len(), 5);
        assert!(manifest.operations.iter().all(|operation| {
            operation.capabilities.contains(&Capability::ProcessSpawn)
                && operation.capabilities.contains(&Capability::FileWrite)
                && !operation.opaque
        }));
    }

    #[test]
    fn qgis_operation_rejects_arbitrary_algorithm_expression_and_output() {
        let value = json!({
            "operation": "create_grid",
            "min_x": 0,
            "max_x": 1000,
            "min_y": 0,
            "max_y": 1000,
            "horizontal_spacing": 100,
            "vertical_spacing": 100,
            "srid": 6675,
            "algorithm": "native:runpython",
            "expression": "system('danger')",
            "output": "/etc/important"
        });
        assert!(serde_json::from_value::<QgisOperation>(value).is_err());
    }

    #[test]
    fn grid_budget_and_target_crs_fail_closed() {
        assert!(matches!(
            validate_operation_contract(&QgisOperation::CreateGrid {
                min_x: 0.0,
                max_x: 1_000_000.0,
                min_y: 0.0,
                max_y: 1_000_000.0,
                horizontal_spacing: 1.0,
                vertical_spacing: 1.0,
                srid: 6675,
            }),
            Err(QgisError::Contract(_))
        ));
        assert!(matches!(
            validate_operation_contract(&QgisOperation::Reproject {
                input: "fixture.geojson".into(),
                target_srid: 3857,
            }),
            Err(QgisError::Contract(_))
        ));
    }
}
