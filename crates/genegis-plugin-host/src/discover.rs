use std::path::{Path, PathBuf};

use genegis_plugin_api::{CapabilityPolicy, PluginCapability, PluginManifest, MANIFEST_FILENAME};
use serde::{Deserialize, Serialize};

use crate::error::PluginHostError;

/// A discovered plugin bundle that passed manifest and policy checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginEntry {
    /// Absolute or relative path to the plugin bundle directory.
    pub bundle_dir: PathBuf,
    /// Parsed manifest from `genegis.plugin.json`.
    pub manifest: PluginManifest,
    /// Capabilities granted after intersecting manifest requests with host policy.
    pub effective_capabilities: Vec<PluginCapability>,
}

impl PluginEntry {
    /// JSON summary for CLI listings and workbench panels.
    pub fn summary_json(&self) -> serde_json::Value {
        let mut summary = self.manifest.summary_json();
        if let Some(obj) = summary.as_object_mut() {
            obj.insert(
                "bundle_dir".into(),
                serde_json::Value::String(self.bundle_dir.display().to_string()),
            );
            obj.insert(
                "effective_capabilities".into(),
                serde_json::json!(self.effective_capabilities),
            );
        }
        summary
    }

    /// Resolve the WASM module path declared by the manifest, if any.
    pub fn wasm_path(&self) -> Option<PathBuf> {
        self.manifest
            .wasm
            .as_ref()
            .map(|spec| self.bundle_dir.join(&spec.entry))
    }
}

/// Scan a plugin root directory and return manifest-only entries.
pub fn discover_plugins(
    root: impl AsRef<Path>,
    policy: &CapabilityPolicy,
) -> Result<Vec<PluginEntry>, PluginHostError> {
    let root = root.as_ref();
    if !root.is_dir() {
        return Err(PluginHostError::Bundle(format!(
            "plugin root is not a directory: {}",
            root.display()
        )));
    }

    let mut entries = Vec::new();
    for child in std::fs::read_dir(root)
        .map_err(|err| PluginHostError::Bundle(err.to_string()))?
    {
        let child = child.map_err(|err| PluginHostError::Bundle(err.to_string()))?;
        if !child.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }

        let bundle_dir = child.path();
        if bundle_dir.join(MANIFEST_FILENAME).exists() {
            entries.push(discover_bundle(&bundle_dir, policy)?);
        }
    }

    entries.sort_by(|left, right| left.manifest.id.cmp(&right.manifest.id));
    Ok(entries)
}

/// Discover a single plugin bundle directory.
pub fn discover_bundle(
    bundle_dir: impl AsRef<Path>,
    policy: &CapabilityPolicy,
) -> Result<PluginEntry, PluginHostError> {
    let bundle_dir = bundle_dir.as_ref();
    let manifest = PluginManifest::from_bundle_dir(bundle_dir)?;
    manifest.validate()?;
    policy.validate_manifest(&manifest)?;

    Ok(PluginEntry {
        bundle_dir: bundle_dir.to_path_buf(),
        effective_capabilities: policy.effective_capabilities(&manifest).collect(),
        manifest,
    })
}

/// Find a plugin entry by id within a plugin root directory.
pub fn find_plugin<'a>(
    entries: &'a [PluginEntry],
    plugin_id: &str,
) -> Result<&'a PluginEntry, PluginHostError> {
    entries
        .iter()
        .find(|entry| entry.manifest.id == plugin_id)
        .ok_or_else(|| PluginHostError::NotFound(plugin_id.into()))
}
