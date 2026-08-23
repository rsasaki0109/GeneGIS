//! Typed GRASS GIS execution inside an immutable, network-isolated container.

use crate::{
    admit, AdapterInvocation, AdapterManifest, AdapterOperation, AdmissionFailure, BackendFamily,
    BackendIdentity, Capability, CapabilityPolicy, Determinism, EvidenceHook,
    ADAPTER_MANIFEST_SCHEMA_VERSION,
};
use genegis_contract::{
    AggregationBasis, AxisOrder, GeoContract, GeometryKind, MeasureContract, MeasureKind,
    SpatialContract,
};
use genegis_crs::Crs;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Number, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;
use std::time::Instant;
use thiserror::Error;

/// Official GRASS image tag used to resolve the conformance runtime.
pub const GRASS_IMAGE_REFERENCE: &str = "docker.io/osgeo/grass-gis:releasebranch_8_5-alpine";

/// Immutable registry digest executed by the Phase 12 conformance harness.
pub const GRASS_IMAGE_DIGEST: &str =
    "sha256:dbefc741e2cb03dcb0def2f2fbb077d2bf06656801e37285fd29624c5745940e";

const ADAPTER_ID: &str = "org.genegis.grass.sandbox";
const ADAPTER_VERSION: &str = "0.1.0";
const GRASS_VERSION: &str = "8.5.1dev";
const GDAL_VERSION: &str = "3.13.2";
const PROJ_VERSION: &str = "9.8.1";
const PROJECTED_SRID: i32 = 6675;
const GEOGRAPHIC_SRID: i32 = 4326;

/// Typed modules and fixed module pipelines exposed by the GRASS adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum GrassOperation {
    /// Derive a grid region from projected bounds and resolution.
    RegionGrid {
        /// Northern bound in metres.
        north: f64,
        /// Southern bound in metres.
        south: f64,
        /// Eastern bound in metres.
        east: f64,
        /// Western bound in metres.
        west: f64,
        /// Cell resolution in metres.
        resolution: f64,
        /// Projected CRS; v0 is fixed to EPSG:6675.
        srid: i32,
    },
    /// Report the exact GRASS project CRS definition.
    ProjectionInfo {
        /// CRS to inspect; v0 accepts EPSG:6675.
        srid: i32,
    },
    /// Transform one longitude/latitude point to Nagoya's projected CRS.
    TransformPoint {
        /// Longitude in decimal degrees.
        longitude: f64,
        /// Latitude in decimal degrees.
        latitude: f64,
        /// Source CRS; v0 accepts EPSG:4326.
        source_srid: i32,
        /// Target CRS; v0 accepts EPSG:6675.
        target_srid: i32,
    },
    /// Measure a geodesic segment in metres in EPSG:4326.
    GeodesicDistance {
        /// First longitude.
        start_longitude: f64,
        /// First latitude.
        start_latitude: f64,
        /// Second longitude.
        end_longitude: f64,
        /// Second latitude.
        end_latitude: f64,
    },
    /// Build a constant raster in a disposable mapset and report statistics.
    ConstantRasterStats {
        /// Raster row count, bounded by policy validation.
        rows: u32,
        /// Raster column count, bounded by policy validation.
        cols: u32,
        /// Integer cell value used by the fixed `r.mapcalc` expression.
        value: i32,
    },
    /// Generate seeded points in a disposable mapset and report topology.
    SeededPointCount {
        /// Number of points, bounded by policy validation.
        points: u32,
        /// Explicit deterministic random seed.
        seed: u32,
    },
}

impl GrassOperation {
    fn operation_id(&self) -> &'static str {
        match self {
            Self::RegionGrid { .. } => "grass.region.grid",
            Self::ProjectionInfo { .. } => "grass.projection.info",
            Self::TransformPoint { .. } => "grass.transform.point",
            Self::GeodesicDistance { .. } => "grass.measure.geodesic",
            Self::ConstantRasterStats { .. } => "grass.raster.constant-stats",
            Self::SeededPointCount { .. } => "grass.vector.seeded-points",
        }
    }

    fn parameters(&self) -> Value {
        serde_json::to_value(self).expect("GRASS operation serialization is infallible")
    }

    fn modules(&self) -> Vec<String> {
        match self {
            Self::RegionGrid { .. } => vec!["g.region".into()],
            Self::ProjectionInfo { .. } => vec!["g.proj".into()],
            Self::TransformPoint { .. } => vec!["m.proj".into()],
            Self::GeodesicDistance { .. } => vec!["m.measure".into()],
            Self::ConstantRasterStats { .. } => {
                vec!["g.region".into(), "r.mapcalc".into(), "r.univar".into()]
            }
            Self::SeededPointCount { .. } => {
                vec!["g.region".into(), "v.random".into(), "v.info".into()]
            }
        }
    }

    fn grass_args(&self) -> Vec<String> {
        match self {
            Self::RegionGrid {
                north,
                south,
                east,
                west,
                resolution,
                ..
            } => vec![
                "--tmp-project".into(),
                format!("EPSG:{PROJECTED_SRID}"),
                "--exec".into(),
                "g.region".into(),
                "-g".into(),
                format!("n={north}"),
                format!("s={south}"),
                format!("e={east}"),
                format!("w={west}"),
                format!("res={resolution}"),
            ],
            Self::ProjectionInfo { .. } => vec![
                "--tmp-project".into(),
                format!("EPSG:{PROJECTED_SRID}"),
                "--exec".into(),
                "g.proj".into(),
                "-g".into(),
            ],
            Self::TransformPoint {
                longitude,
                latitude,
                ..
            } => vec![
                "--tmp-project".into(),
                format!("EPSG:{PROJECTED_SRID}"),
                "--exec".into(),
                "m.proj".into(),
                "-id".into(),
                format!("coordinates={longitude},{latitude}"),
                "separator=comma".into(),
            ],
            Self::GeodesicDistance {
                start_longitude,
                start_latitude,
                end_longitude,
                end_latitude,
            } => vec![
                "--tmp-project".into(),
                format!("EPSG:{GEOGRAPHIC_SRID}"),
                "--exec".into(),
                "m.measure".into(),
                "-g".into(),
                format!(
                    "coordinates={start_longitude},{start_latitude},{end_longitude},{end_latitude}"
                ),
                "units=meters".into(),
            ],
            Self::ConstantRasterStats { rows, cols, value } => vec![
                "--tmp-project".into(),
                format!("EPSG:{PROJECTED_SRID}"),
                "--exec".into(),
                "sh".into(),
                "-eu".into(),
                "-c".into(),
                "g.region n=1000 s=0 e=1000 w=0 rows=\"$1\" cols=\"$2\"; r.mapcalc expression=\"fixture=$3\" --quiet; r.univar -g map=fixture".into(),
                "genegis-grass-raster".into(),
                rows.to_string(),
                cols.to_string(),
                value.to_string(),
            ],
            Self::SeededPointCount { points, seed } => vec![
                "--tmp-project".into(),
                format!("EPSG:{PROJECTED_SRID}"),
                "--exec".into(),
                "sh".into(),
                "-eu".into(),
                "-c".into(),
                "g.region n=1000 s=0 e=1000 w=0 res=10; v.random output=fixture npoints=\"$1\" seed=\"$2\" --quiet; v.info -t map=fixture".into(),
                "genegis-grass-vector".into(),
                points.to_string(),
                seed.to_string(),
            ],
        }
    }
}

/// Container controls proven for each GRASS receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxEvidence {
    /// Container network mode.
    pub network: String,
    /// Whether the container root filesystem was read-only.
    pub read_only_root: bool,
    /// Numeric non-root UID:GID.
    pub user: String,
    /// Whether every Linux capability was dropped.
    pub all_capabilities_dropped: bool,
    /// Whether privilege escalation was disabled.
    pub no_new_privileges: bool,
    /// Process-count limit.
    pub pids_limit: u32,
    /// Memory limit in MiB.
    pub memory_mib: u32,
    /// CPU quota.
    pub cpus: String,
    /// Writable ephemeral paths; no host path is mounted.
    pub tmpfs_paths: Vec<String>,
}

impl Default for SandboxEvidence {
    fn default() -> Self {
        Self {
            network: "none".into(),
            read_only_root: true,
            user: "65534:65534".into(),
            all_capabilities_dropped: true,
            no_new_privileges: true,
            pids_limit: 128,
            memory_mib: 512,
            cpus: "1".into(),
            tmpfs_paths: vec!["/tmp".into(), "/grassdb".into()],
        }
    }
}

/// Evidence emitted by one successful sandboxed GRASS operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrassReceipt {
    /// Adapter manifest digest admitted before process spawn.
    pub manifest_digest: String,
    /// Semantic operation identifier.
    pub operation_id: String,
    /// Exact pinned backend identity.
    pub backend: BackendIdentity,
    /// GRASS modules invoked by the fixed operation.
    pub modules: Vec<String>,
    /// Normalized typed parameters.
    pub parameters: Value,
    /// Parsed operation result.
    pub output: Value,
    /// Digest of the canonical result.
    pub output_digest: String,
    /// Captured warnings excluding expected version probe lines.
    pub warnings: Vec<String>,
    /// Enforced container controls.
    pub sandbox: SandboxEvidence,
    /// Measured wall time including admission and container execution.
    pub elapsed_ns: u64,
}

/// Failure at admission, contract validation, runtime identity, or module execution.
#[derive(Debug, Error)]
pub enum GrassError {
    /// Capability policy rejected execution before Docker was called.
    #[error("GRASS adapter admission failed: {0:?}")]
    Admission(Vec<AdmissionFailure>),
    /// Typed parameters disagree with the reviewed operation contract.
    #[error("GRASS operation contract failed: {0}")]
    Contract(String),
    /// Docker is unavailable or a local process could not be launched.
    #[error("GRASS sandbox process failed: {0}")]
    Process(#[from] std::io::Error),
    /// Local image or runtime component identity differs from the manifest.
    #[error("GRASS runtime identity mismatch: {0}")]
    RuntimeIdentity(String),
    /// GRASS or a fixed module pipeline returned non-zero.
    #[error("GRASS module failed with status {status}: {stderr}")]
    Module {
        /// Container exit status, or -1 if unavailable.
        status: i32,
        /// Captured module diagnostics.
        stderr: String,
    },
    /// Expected machine-readable module output was malformed.
    #[error("GRASS output parsing failed: {0}")]
    Output(String),
    /// Receipt serialization failed.
    #[error("GRASS receipt serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Immutable GRASS sandbox executor.
#[derive(Debug, Clone)]
pub struct GrassAdapter {
    manifest: AdapterManifest,
    policy: CapabilityPolicy,
}

impl Default for GrassAdapter {
    fn default() -> Self {
        let manifest = grass_manifest();
        let policy = CapabilityPolicy::sandboxed_process(&manifest.adapter_id);
        Self { manifest, policy }
    }
}

impl GrassAdapter {
    /// Construct an adapter with an explicitly reviewed manifest and policy.
    pub fn new(manifest: AdapterManifest, policy: CapabilityPolicy) -> Self {
        Self { manifest, policy }
    }

    /// Return the reviewed GRASS manifest.
    pub fn manifest(&self) -> &AdapterManifest {
        &self.manifest
    }

    /// Execute a typed operation in a disposable, networkless, non-root container.
    pub fn execute(&self, operation: &GrassOperation) -> Result<GrassReceipt, GrassError> {
        let started = Instant::now();
        validate_operation_contract(operation)?;
        let invocation = AdapterInvocation {
            adapter_id: self.manifest.adapter_id.clone(),
            adapter_version: self.manifest.adapter_version.clone(),
            operation_id: operation.operation_id().into(),
            operation_version: "1.0.0".into(),
            backend: self.manifest.backend.clone(),
            requested_capabilities: BTreeSet::from([Capability::ProcessSpawn]),
        };
        let admission = admit(&self.manifest, &invocation, &self.policy);
        if !admission.admitted || !admission.verification_eligible {
            return Err(GrassError::Admission(admission.failures));
        }
        validate_local_image(&self.manifest.backend)?;

        let sandbox = SandboxEvidence::default();
        let image = format!("osgeo/grass-gis@{GRASS_IMAGE_DIGEST}");
        let wrapper = "grass --version >&2; gdalinfo --version >&2; proj 2>&1 | head -1 >&2; exec grass \"$@\"";
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
            "128",
            "--memory",
            "512m",
            "--cpus",
            "1",
            "--tmpfs",
            "/tmp:rw,noexec,nosuid,size=64m,mode=1777",
            "--tmpfs",
            "/grassdb:rw,noexec,nosuid,size=128m,mode=1777",
            "-e",
            "HOME=/tmp",
            &image,
            "sh",
            "-eu",
            "-c",
            wrapper,
            "genegis-grass-wrapper",
        ]);
        command.args(operation.grass_args());
        let result = command.output()?;
        let stdout = String::from_utf8_lossy(&result.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&result.stderr).into_owned();
        if !result.status.success() {
            return Err(GrassError::Module {
                status: result.status.code().unwrap_or(-1),
                stderr,
            });
        }
        validate_runtime_versions(&stderr)?;
        let output = parse_output(operation, &stdout)?;
        let warnings = runtime_warnings(&stderr);

        Ok(GrassReceipt {
            manifest_digest: self.manifest.digest()?,
            operation_id: operation.operation_id().into(),
            backend: self.manifest.backend.clone(),
            modules: operation.modules(),
            parameters: operation.parameters(),
            output_digest: digest_json(&output)?,
            output,
            warnings,
            sandbox,
            elapsed_ns: started.elapsed().as_nanos().min(u64::MAX as u128) as u64,
        })
    }
}

fn validate_operation_contract(operation: &GrassOperation) -> Result<(), GrassError> {
    let finite = |values: &[f64]| values.iter().all(|value| value.is_finite());
    let valid = match operation {
        GrassOperation::RegionGrid {
            north,
            south,
            east,
            west,
            resolution,
            srid,
        } => {
            *srid == PROJECTED_SRID
                && finite(&[*north, *south, *east, *west, *resolution])
                && north > south
                && east > west
                && *resolution > 0.0
                && ((*north - *south) / *resolution) <= 10_000.0
                && ((*east - *west) / *resolution) <= 10_000.0
        }
        GrassOperation::ProjectionInfo { srid } => *srid == PROJECTED_SRID,
        GrassOperation::TransformPoint {
            longitude,
            latitude,
            source_srid,
            target_srid,
        } => {
            *source_srid == GEOGRAPHIC_SRID
                && *target_srid == PROJECTED_SRID
                && finite(&[*longitude, *latitude])
                && (-180.0..=180.0).contains(longitude)
                && (-90.0..=90.0).contains(latitude)
        }
        GrassOperation::GeodesicDistance {
            start_longitude,
            start_latitude,
            end_longitude,
            end_latitude,
        } => {
            finite(&[
                *start_longitude,
                *start_latitude,
                *end_longitude,
                *end_latitude,
            ]) && (-180.0..=180.0).contains(start_longitude)
                && (-180.0..=180.0).contains(end_longitude)
                && (-90.0..=90.0).contains(start_latitude)
                && (-90.0..=90.0).contains(end_latitude)
        }
        GrassOperation::ConstantRasterStats { rows, cols, .. } => {
            (1..=10_000).contains(rows)
                && (1..=10_000).contains(cols)
                && u64::from(*rows) * u64::from(*cols) <= 10_000_000
        }
        GrassOperation::SeededPointCount { points, seed } => {
            (1..=100_000).contains(points) && *seed > 0
        }
    };
    if valid {
        Ok(())
    } else {
        Err(GrassError::Contract(format!(
            "{} parameters exceed its CRS or resource contract",
            operation.operation_id()
        )))
    }
}

fn validate_local_image(backend: &BackendIdentity) -> Result<(), GrassError> {
    let image = format!("osgeo/grass-gis@{}", backend.build_digest);
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
        return Err(GrassError::RuntimeIdentity(format!(
            "pinned image {} is unavailable or does not resolve to its manifest digest",
            backend.build_digest
        )));
    }
    Ok(())
}

fn validate_runtime_versions(stderr: &str) -> Result<(), GrassError> {
    for (component, expected) in [
        ("GRASS", format!("GRASS {GRASS_VERSION}")),
        ("GDAL", format!("GDAL {GDAL_VERSION}")),
        ("PROJ", format!("Rel. {PROJ_VERSION}")),
    ] {
        if !stderr.contains(&expected) {
            return Err(GrassError::RuntimeIdentity(format!(
                "expected {component} marker {expected:?}; observed {stderr:?}"
            )));
        }
    }
    Ok(())
}

fn runtime_warnings(stderr: &str) -> Vec<String> {
    stderr
        .lines()
        .filter(|line| {
            !line.trim().is_empty()
                && !line.starts_with("GRASS ")
                && !line.starts_with("Geographic Resources")
                && !line.starts_with("1999-")
                && !line.starts_with("This GRASS")
                && !line.starts_with("the GRASS")
                && !line.starts_with("GNU General")
                && !line.starts_with("This program")
                && !line.starts_with("WITHOUT ANY")
                && !line.starts_with("MERCHANTABILITY")
                && !line.starts_with("General Public")
                && !line.starts_with("See the GNU")
                && !line.starts_with("GDAL ")
                && !line.starts_with("Rel. ")
        })
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn parse_output(operation: &GrassOperation, stdout: &str) -> Result<Value, GrassError> {
    if matches!(operation, GrassOperation::TransformPoint { .. }) {
        let values = stdout
            .trim()
            .split(',')
            .map(|value| value.parse::<f64>())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| GrassError::Output(error.to_string()))?;
        if values.len() != 3 {
            return Err(GrassError::Output(format!(
                "expected x,y,z from m.proj, observed {stdout:?}"
            )));
        }
        return Ok(json!({ "x": values[0], "y": values[1], "z": values[2] }));
    }
    let mut map = Map::new();
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let (key, value) = line.split_once('=').ok_or_else(|| {
            GrassError::Output(format!("expected key=value output, observed {line:?}"))
        })?;
        map.insert(key.into(), parse_scalar(value));
    }
    if map.is_empty() {
        return Err(GrassError::Output("module returned no values".into()));
    }
    Ok(Value::Object(map))
}

fn parse_scalar(value: &str) -> Value {
    if let Ok(integer) = value.parse::<i64>() {
        return Value::Number(integer.into());
    }
    if let Ok(float) = value.parse::<f64>() {
        if let Some(number) = Number::from_f64(float) {
            return Value::Number(number);
        }
    }
    Value::String(value.into())
}

fn digest_json(value: &Value) -> Result<String, serde_json::Error> {
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(value)?)
    ))
}

/// Return the reviewed manifest for the pinned GRASS 8.5 stable-branch image.
pub fn grass_manifest() -> AdapterManifest {
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
                     determinism: Determinism| AdapterOperation {
        operation_id: operation_id.into(),
        operation_version: "1.0.0".into(),
        inputs,
        outputs,
        capabilities: BTreeSet::from([Capability::ProcessSpawn]),
        determinism,
        evidence_hooks: hooks.clone(),
        opaque: false,
    };
    AdapterManifest {
        schema_version: ADAPTER_MANIFEST_SCHEMA_VERSION.into(),
        adapter_id: ADAPTER_ID.into(),
        adapter_version: ADAPTER_VERSION.into(),
        backend: BackendIdentity {
            family: BackendFamily::Grass,
            engine_version: GRASS_VERSION.into(),
            build_digest: GRASS_IMAGE_DIGEST.into(),
            components: BTreeMap::from([
                ("gdal".into(), GDAL_VERSION.into()),
                ("proj".into(), PROJ_VERSION.into()),
            ]),
        },
        license: "GPL-2.0-or-later".into(),
        operations: vec![
            operation(
                "grass.region.grid",
                vec![polygon_contract(
                    "grass.region.extent",
                    Crs::nagoya_projected(),
                )],
                vec![raster_contract("grass.region.grid.output")],
                Determinism::Deterministic,
            ),
            operation(
                "grass.projection.info",
                vec![polygon_contract(
                    "grass.projection.input",
                    Crs::nagoya_projected(),
                )],
                vec![measure_contract(
                    "grass.projection.output",
                    MeasureKind::Identifier,
                    "crs-definition",
                )],
                Determinism::Deterministic,
            ),
            operation(
                "grass.transform.point",
                vec![point_contract("grass.transform.input", Crs::wgs84())],
                vec![point_contract(
                    "grass.transform.output",
                    Crs::nagoya_projected(),
                )],
                Determinism::ToleranceBounded,
            ),
            operation(
                "grass.measure.geodesic",
                vec![point_contract("grass.measure.input", Crs::wgs84())],
                vec![measure_contract(
                    "grass.measure.output",
                    MeasureKind::Length,
                    "m",
                )],
                Determinism::ToleranceBounded,
            ),
            operation(
                "grass.raster.constant-stats",
                vec![measure_contract(
                    "grass.raster.constant",
                    MeasureKind::Count,
                    "integer",
                )],
                vec![raster_contract("grass.raster.stats")],
                Determinism::Deterministic,
            ),
            operation(
                "grass.vector.seeded-points",
                vec![measure_contract(
                    "grass.vector.seed",
                    MeasureKind::Count,
                    "points",
                )],
                vec![point_contract(
                    "grass.vector.points",
                    Crs::nagoya_projected(),
                )],
                Determinism::Seeded,
            ),
        ],
    }
}

fn polygon_contract(id: &str, crs: Crs) -> GeoContract {
    GeoContract::new(id)
        .with_spatial(SpatialContract::known(
            GeometryKind::Polygon,
            crs,
            AxisOrder::Xy,
        ))
        .with_measure(MeasureContract::simple(
            MeasureKind::Geometry,
            "geometry",
            AggregationBasis::None,
        ))
}

fn point_contract(id: &str, crs: Crs) -> GeoContract {
    let axis = if crs == Crs::wgs84() {
        AxisOrder::LongitudeLatitude
    } else {
        AxisOrder::Xy
    };
    GeoContract::new(id)
        .with_spatial(SpatialContract::known(GeometryKind::Point, crs, axis))
        .with_measure(MeasureContract::simple(
            MeasureKind::Geometry,
            "geometry",
            AggregationBasis::None,
        ))
}

fn raster_contract(id: &str) -> GeoContract {
    GeoContract::new(id)
        .with_spatial(SpatialContract::known(
            GeometryKind::Raster,
            Crs::nagoya_projected(),
            AxisOrder::Xy,
        ))
        .with_measure(MeasureContract::simple(
            MeasureKind::Count,
            "integer",
            AggregationBasis::None,
        ))
}

fn measure_contract(id: &str, kind: MeasureKind, unit: &str) -> GeoContract {
    GeoContract::new(id).with_measure(MeasureContract::simple(kind, unit, AggregationBasis::None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grass_manifest_has_only_typed_sandboxed_operations() {
        let manifest = grass_manifest();
        assert!(manifest.validate().is_empty());
        assert_eq!(manifest.operations.len(), 6);
        assert!(manifest.operations.iter().all(|operation| {
            operation.capabilities == BTreeSet::from([Capability::ProcessSpawn])
                && !operation.opaque
                && operation
                    .evidence_hooks
                    .contains(&EvidenceHook::EnvironmentDigest)
        }));
    }

    #[test]
    fn grass_operation_rejects_arbitrary_module_or_shell() {
        let value = json!({
            "operation": "region_grid",
            "north": 1000.0,
            "south": 0.0,
            "east": 1000.0,
            "west": 0.0,
            "resolution": 10.0,
            "srid": 6675,
            "module": "g.remove",
            "shell": "rm -rf /"
        });
        assert!(serde_json::from_value::<GrassOperation>(value).is_err());
    }

    #[test]
    fn invalid_crs_extent_and_resource_budget_fail_before_spawn() {
        for operation in [
            GrassOperation::ProjectionInfo { srid: 4326 },
            GrassOperation::RegionGrid {
                north: 0.0,
                south: 1000.0,
                east: 1000.0,
                west: 0.0,
                resolution: 10.0,
                srid: 6675,
            },
            GrassOperation::ConstantRasterStats {
                rows: 10_000,
                cols: 10_000,
                value: 1,
            },
        ] {
            assert!(matches!(
                validate_operation_contract(&operation),
                Err(GrassError::Contract(_))
            ));
        }
    }
}
