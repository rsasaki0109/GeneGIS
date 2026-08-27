//! Provider-neutral desktop layer transfer into a Command/Workflow-bound capsule.

use std::{
    fs,
    path::{Path, PathBuf},
};

use genegis_core::{
    Command, CommandEnvelope, CommandOrigin, InputSnapshot, LayerKind, WorkflowDigest,
};
use genegis_crs::{CoordinateUnit, Crs, SourceMetadata};
use genegis_workflow::{desktop_layer_bridge_template, DesktopLayerBridgeInput, GeoWorkflow};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{CapsuleEntry, CapsuleError};

/// Bridge-capsule schema version.
pub const BRIDGE_CAPSULE_SCHEMA_VERSION: &str = "0.1.0";

const REQUEST_PATH: &str = "metadata/bridge-request.json";
const COMMAND_PATH: &str = "metadata/command.json";
const WORKFLOW_PATH: &str = "metadata/workflow.json";

/// Open formats accepted by the first desktop bridge slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopLayerFormat {
    /// GeoJSON vector data.
    GeoJson,
    /// GeoParquet vector or tabular data.
    GeoParquet,
    /// Cloud Optimized GeoTIFF raster.
    Cog,
    /// Cloud Optimized Point Cloud.
    Copc,
    /// PMTiles archive.
    PmTiles,
}

impl DesktopLayerFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::GeoJson => "geojson",
            Self::GeoParquet => "parquet",
            Self::Cog => "tif",
            Self::Copc => "copc.laz",
            Self::PmTiles => "pmtiles",
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::GeoJson => "geojson",
            Self::GeoParquet => "geoparquet",
            Self::Cog => "cog",
            Self::Copc => "copc",
            Self::PmTiles => "pmtiles",
        }
    }
}

/// One explicitly selected desktop layer and its transfer contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesktopBridgeLayer {
    /// Stable lowercase identifier used in portable paths.
    pub id: String,
    /// Display name retained in the proposed project.
    pub name: String,
    /// Semantic layer kind.
    pub kind: LayerKind,
    /// Open transfer encoding.
    pub format: DesktopLayerFormat,
    /// Local exported asset read by the bridge.
    pub source_path: String,
    /// Known CRS of the exact exported bytes.
    pub crs: Crs,
    /// Coordinate unit asserted by the desktop host and checked against CRS.
    pub coordinate_unit: CoordinateUnit,
    /// Required license or attribution notice.
    pub license: String,
    /// Optional provider-declared SHA-256 compared with observed bytes.
    pub expected_checksum: Option<String>,
    /// Optional layer extent in the declared CRS.
    pub extent: Option<[String; 4]>,
    /// Optional closed temporal interval.
    pub temporal_interval: Option<[String; 2]>,
}

/// Provider-neutral request emitted by a thin desktop plugin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesktopBridgeRequest {
    /// Proposed GeneGIS project name.
    pub project_name: String,
    /// Desktop host identity and version, without user identity.
    pub desktop_host: String,
    /// Explicitly selected layers only.
    pub layers: Vec<DesktopBridgeLayer>,
}

/// Content-addressed inventory for one desktop bridge capsule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BridgeCapsuleManifest {
    /// Bridge schema version.
    pub schema_version: String,
    /// Stable Workflow Graph identity.
    pub workflow_digest: String,
    /// Canonical identity of project declaration and asset inventory.
    pub project_digest: String,
    /// Sorted content-addressed files.
    pub entries: Vec<CapsuleEntry>,
}

/// Successful offline bridge verification summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BridgeCapsuleVerification {
    /// Verified schema version.
    pub schema_version: String,
    /// Verified workflow identity.
    pub workflow_digest: String,
    /// Verified project identity.
    pub project_digest: String,
    /// Number of byte-exact layer assets.
    pub verified_layers: usize,
    /// Number of inventory files checked.
    pub verified_entries: usize,
}

/// Validate and seal selected desktop exports into a new bridge capsule directory.
pub fn seal_desktop_bridge_capsule(
    request: &DesktopBridgeRequest,
    root: impl AsRef<Path>,
) -> Result<BridgeCapsuleManifest, CapsuleError> {
    validate_request(request)?;
    let root = root.as_ref();
    ensure_empty_destination(root)?;
    fs::create_dir_all(root.join("metadata")).map_err(|source| io_error(root, source))?;
    fs::create_dir_all(root.join("assets")).map_err(|source| io_error(root, source))?;

    let mut normalized = request.clone();
    let mut bridge_inputs = Vec::with_capacity(request.layers.len());
    let mut entries = Vec::new();
    for (layer, normalized_layer) in request.layers.iter().zip(normalized.layers.iter_mut()) {
        let source_path = Path::new(&layer.source_path);
        let bytes = fs::read(source_path).map_err(|source| io_error(source_path, source))?;
        if bytes.is_empty() {
            return Err(verify_error(format!(
                "layer {} has empty asset bytes",
                layer.id
            )));
        }
        validate_asset(layer.format, source_path, &bytes)?;
        let observed = sha256(&bytes);
        if let Some(expected) = layer.expected_checksum.as_deref() {
            if normalize_digest(expected) != Some(observed.as_str()) {
                return Err(verify_error(format!(
                    "layer {} checksum mismatch",
                    layer.id
                )));
            }
        }
        normalized_layer.expected_checksum = Some(observed.clone());
        let mut source = SourceMetadata::from_uri(
            source_path.to_string_lossy().to_string(),
            Some(&observed),
            Some("desktop-bridge-v0"),
        );
        source.license = Some(layer.license.clone());
        if !source.checksum_verified() {
            return Err(verify_error(format!(
                "layer {} source snapshot is not verified",
                layer.id
            )));
        }
        bridge_inputs.push(DesktopLayerBridgeInput {
            layer_id: layer.id.clone(),
            format: layer.format.as_str().into(),
            crs: layer.crs.clone(),
            source,
        });
        let asset_path = format!("assets/{}.{}", layer.id, layer.format.extension());
        write_file(&root.join(&asset_path), &bytes)?;
        entries.push(entry(
            &asset_path,
            "desktop-layer-asset",
            media_type(layer.format),
            &bytes,
        ));
    }

    let workflow = desktop_layer_bridge_template(&bridge_inputs);
    workflow
        .validate()
        .map_err(|error| verify_error(error.to_string()))?;
    let workflow_digest = workflow
        .stable_digest()
        .map_err(|error| verify_error(error.to_string()))?;
    let mut command = CommandEnvelope::new(
        CommandOrigin::Plugin,
        Command::RunWorkflow {
            workflow_id: workflow.id,
        },
    )
    .with_workflow_digest(WorkflowDigest::new(workflow_digest.clone()));
    for input in &bridge_inputs {
        command = command
            .with_source_snapshot(input.source.clone())
            .with_input_snapshot(
                InputSnapshot::new(input.layer_id.clone(), input.source.clone())
                    .with_crs(input.crs.clone())
                    .with_value_unit("source_bytes"),
            );
    }

    let request_bytes = serde_json::to_vec_pretty(&normalized)?;
    let workflow_bytes = serde_json::to_vec_pretty(&workflow)?;
    let command_bytes = serde_json::to_vec_pretty(&command)?;
    for (path, role, bytes) in [
        (REQUEST_PATH, "desktop-bridge-request", request_bytes),
        (WORKFLOW_PATH, "workflow", workflow_bytes),
        (COMMAND_PATH, "command", command_bytes),
    ] {
        write_file(&root.join(path), &bytes)?;
        entries.push(entry(path, role, "application/json", &bytes));
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    let project_digest = project_digest(&normalized, &entries)?;
    let manifest = BridgeCapsuleManifest {
        schema_version: BRIDGE_CAPSULE_SCHEMA_VERSION.into(),
        workflow_digest,
        project_digest,
        entries,
    };
    write_file(
        &root.join("bridge.json"),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    verify_desktop_bridge_capsule(root)?;
    Ok(manifest)
}

/// Verify bridge inventory, assets, CRS/unit/license contracts, and graph binding offline.
pub fn verify_desktop_bridge_capsule(
    root: impl AsRef<Path>,
) -> Result<BridgeCapsuleVerification, CapsuleError> {
    let root = root.as_ref();
    let manifest: BridgeCapsuleManifest = read_json(&root.join("bridge.json"))?;
    if manifest.schema_version != BRIDGE_CAPSULE_SCHEMA_VERSION {
        return Err(verify_error("unsupported bridge capsule schema"));
    }
    for item in &manifest.entries {
        validate_portable_path(&item.path)?;
        let bytes = fs::read(root.join(&item.path))
            .map_err(|source| io_error(&root.join(&item.path), source))?;
        if item.bytes != bytes.len() as u64 || item.sha256 != sha256(&bytes) {
            return Err(verify_error(format!("inventory mismatch: {}", item.path)));
        }
    }
    let request: DesktopBridgeRequest = read_json(&root.join(REQUEST_PATH))?;
    validate_request(&request)?;
    let workflow: GeoWorkflow = read_json(&root.join(WORKFLOW_PATH))?;
    workflow
        .validate()
        .map_err(|error| verify_error(error.to_string()))?;
    let actual_workflow_digest = workflow
        .stable_digest()
        .map_err(|error| verify_error(error.to_string()))?;
    if actual_workflow_digest != manifest.workflow_digest {
        return Err(verify_error("bridge workflow digest mismatch"));
    }
    let command: CommandEnvelope = read_json(&root.join(COMMAND_PATH))?;
    if command.workflow_digest.as_ref().map(WorkflowDigest::as_str)
        != Some(manifest.workflow_digest.as_str())
        || !matches!(command.command, Command::RunWorkflow { workflow_id } if workflow_id == workflow.id)
    {
        return Err(verify_error("bridge command is not bound to workflow"));
    }
    for layer in &request.layers {
        let path = root.join(format!("assets/{}.{}", layer.id, layer.format.extension()));
        let bytes = fs::read(&path).map_err(|source| io_error(&path, source))?;
        validate_asset(layer.format, &path, &bytes)?;
        if layer.expected_checksum.as_deref() != Some(sha256(&bytes).as_str()) {
            return Err(verify_error(format!("bridge asset changed: {}", layer.id)));
        }
    }
    let project_digest = project_digest(&request, &manifest.entries)?;
    if project_digest != manifest.project_digest {
        return Err(verify_error("bridge project digest mismatch"));
    }
    Ok(BridgeCapsuleVerification {
        schema_version: manifest.schema_version,
        workflow_digest: manifest.workflow_digest,
        project_digest: manifest.project_digest,
        verified_layers: request.layers.len(),
        verified_entries: manifest.entries.len(),
    })
}

fn validate_request(request: &DesktopBridgeRequest) -> Result<(), CapsuleError> {
    if request.project_name.trim().is_empty() || request.desktop_host.trim().is_empty() {
        return Err(verify_error("bridge project and desktop host are required"));
    }
    if request.layers.is_empty() {
        return Err(verify_error("bridge requires at least one selected layer"));
    }
    let mut ids = std::collections::BTreeSet::new();
    for layer in &request.layers {
        if layer.id.is_empty()
            || !layer.id.chars().all(|character| {
                character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
            })
            || !ids.insert(layer.id.as_str())
        {
            return Err(verify_error(format!(
                "invalid or duplicate layer id: {}",
                layer.id
            )));
        }
        if layer.name.trim().is_empty() || layer.license.trim().is_empty() {
            return Err(verify_error(format!(
                "layer {} lacks name or license",
                layer.id
            )));
        }
        let definition = layer
            .crs
            .require_known()
            .map_err(|error| verify_error(error.to_string()))?;
        if definition.unit != layer.coordinate_unit {
            return Err(verify_error(format!(
                "layer {} CRS/unit mismatch",
                layer.id
            )));
        }
        if let Some(extent) = &layer.extent {
            let values = extent
                .iter()
                .map(|value| value.parse::<f64>())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| verify_error(format!("layer {} invalid extent", layer.id)))?;
            if values.iter().any(|value| !value.is_finite())
                || values[0] >= values[2]
                || values[1] >= values[3]
            {
                return Err(verify_error(format!("layer {} invalid extent", layer.id)));
            }
        }
        if let Some(interval) = &layer.temporal_interval {
            if interval[0].trim().is_empty()
                || interval[1].trim().is_empty()
                || interval[0] >= interval[1]
            {
                return Err(verify_error(format!(
                    "layer {} invalid temporal interval",
                    layer.id
                )));
            }
        }
    }
    Ok(())
}

fn validate_asset(
    format: DesktopLayerFormat,
    path: &Path,
    bytes: &[u8],
) -> Result<(), CapsuleError> {
    let lower_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let valid = match format {
        DesktopLayerFormat::GeoJson => {
            lower_name.ends_with(".geojson")
                && serde_json::from_slice::<serde_json::Value>(bytes)
                    .ok()
                    .and_then(|value| {
                        value
                            .get("type")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned)
                    })
                    .as_deref()
                    == Some("FeatureCollection")
        }
        DesktopLayerFormat::GeoParquet => {
            lower_name.ends_with(".parquet")
                && bytes.len() >= 8
                && bytes.starts_with(b"PAR1")
                && bytes.ends_with(b"PAR1")
        }
        DesktopLayerFormat::Cog => {
            (lower_name.ends_with(".tif") || lower_name.ends_with(".tiff"))
                && (bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*"))
        }
        DesktopLayerFormat::Copc => lower_name.ends_with(".copc.laz") && bytes.starts_with(b"LASF"),
        DesktopLayerFormat::PmTiles => {
            lower_name.ends_with(".pmtiles")
                && bytes.len() >= 8
                && &bytes[..7] == b"PMTiles"
                && bytes[7] == 3
        }
    };
    if !valid {
        return Err(verify_error(format!(
            "asset {} does not match declared {} format",
            path.display(),
            format.as_str()
        )));
    }
    Ok(())
}

fn ensure_empty_destination(root: &Path) -> Result<(), CapsuleError> {
    if root.exists() {
        let mut items = fs::read_dir(root).map_err(|source| io_error(root, source))?;
        if items
            .next()
            .transpose()
            .map_err(|source| io_error(root, source))?
            .is_some()
        {
            return Err(verify_error(format!(
                "destination is not empty: {}",
                root.display()
            )));
        }
    }
    Ok(())
}

fn project_digest(
    request: &DesktopBridgeRequest,
    entries: &[CapsuleEntry],
) -> Result<String, CapsuleError> {
    let assets = entries
        .iter()
        .filter(|item| item.role == "desktop-layer-asset")
        .collect::<Vec<_>>();
    Ok(sha256(&serde_json::to_vec(&serde_json::json!({
        "request": request,
        "assets": assets,
    }))?))
}

fn media_type(format: DesktopLayerFormat) -> &'static str {
    match format {
        DesktopLayerFormat::GeoJson => "application/geo+json",
        DesktopLayerFormat::GeoParquet => "application/vnd.apache.parquet",
        DesktopLayerFormat::Cog => "image/tiff; application=geotiff; profile=cloud-optimized",
        DesktopLayerFormat::Copc => "application/vnd.laszip+copc",
        DesktopLayerFormat::PmTiles => "application/vnd.pmtiles",
    }
}

fn entry(path: &str, role: &str, media_type: &str, bytes: &[u8]) -> CapsuleEntry {
    CapsuleEntry {
        path: path.into(),
        role: role.into(),
        media_type: media_type.into(),
        sha256: sha256(bytes),
        bytes: bytes.len() as u64,
    }
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), CapsuleError> {
    fs::write(path, bytes).map_err(|source| io_error(path, source))
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, CapsuleError> {
    let bytes = fs::read(path).map_err(|source| io_error(path, source))?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn validate_portable_path(value: &str) -> Result<(), CapsuleError> {
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(verify_error(format!("non-portable bridge path: {value}")));
    }
    Ok(())
}

fn normalize_digest(value: &str) -> Option<&str> {
    let value = value.trim();
    (value.len() == 71
        && value.starts_with("sha256:")
        && value[7..].bytes().all(|byte| byte.is_ascii_hexdigit()))
    .then_some(value)
}

fn sha256(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn io_error(path: &Path, source: std::io::Error) -> CapsuleError {
    CapsuleError::Io {
        path: PathBuf::from(path),
        source,
    }
}

fn verify_error(message: impl Into<String>) -> CapsuleError {
    CapsuleError::Verification(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(path: &Path) -> DesktopBridgeRequest {
        DesktopBridgeRequest {
            project_name: "Desktop transfer".into(),
            desktop_host: "desktop-gis-test/1.0".into(),
            layers: vec![DesktopBridgeLayer {
                id: "wards".into(),
                name: "Wards".into(),
                kind: LayerKind::Vector,
                format: DesktopLayerFormat::GeoJson,
                source_path: path.to_string_lossy().into_owned(),
                crs: Crs::wgs84(),
                coordinate_unit: CoordinateUnit::Degrees,
                license: "CC BY 4.0".into(),
                expected_checksum: None,
                extent: Some(["136".into(), "35".into(), "137".into(), "36".into()]),
                temporal_interval: None,
            }],
        }
    }

    #[test]
    fn seals_and_verifies_byte_exact_command_workflow_bridge_capsule() {
        let temp = tempfile::tempdir().expect("temp");
        let source = temp.path().join("wards.geojson");
        fs::write(&source, br#"{"type":"FeatureCollection","features":[]}"#).expect("fixture");
        let root = temp.path().join("capsule");
        let manifest = seal_desktop_bridge_capsule(&request(&source), &root).expect("seal");
        assert!(manifest.workflow_digest.starts_with("sha256:"));
        let verified = verify_desktop_bridge_capsule(&root).expect("verify");
        assert_eq!(verified.verified_layers, 1);

        fs::write(root.join("assets/wards.geojson"), b"tampered").expect("tamper");
        assert!(verify_desktop_bridge_capsule(&root).is_err());
    }

    #[test]
    fn rejects_unknown_crs_unit_missing_license_and_checksum_mismatch() {
        let temp = tempfile::tempdir().expect("temp");
        let source = temp.path().join("wards.geojson");
        fs::write(&source, br#"{"type":"FeatureCollection","features":[]}"#).expect("fixture");

        let mut invalid = request(&source);
        invalid.layers[0].coordinate_unit = CoordinateUnit::Metres;
        assert!(seal_desktop_bridge_capsule(&invalid, temp.path().join("unit")).is_err());

        let mut invalid = request(&source);
        invalid.layers[0].license.clear();
        assert!(seal_desktop_bridge_capsule(&invalid, temp.path().join("license")).is_err());

        let mut invalid = request(&source);
        invalid.layers[0].expected_checksum = Some(format!("sha256:{}", "0".repeat(64)));
        assert!(seal_desktop_bridge_capsule(&invalid, temp.path().join("checksum")).is_err());

        let disguised = temp.path().join("disguised.geojson");
        fs::write(&disguised, b"not geojson").expect("disguised fixture");
        assert!(
            seal_desktop_bridge_capsule(&request(&disguised), temp.path().join("format")).is_err()
        );
    }
}
