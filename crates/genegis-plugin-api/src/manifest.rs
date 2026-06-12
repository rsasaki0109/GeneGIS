use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::capability::PluginCapability;
use crate::error::PluginApiError;
use crate::version::{is_api_compatible, MANIFEST_FILENAME, PLUGIN_API_VERSION};

/// WASM module metadata embedded in a plugin bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WasmModuleSpec {
    /// Relative path to the compiled `.wasm` module inside the plugin bundle.
    pub entry: String,
}

/// Declarative metadata shipped with a GeneGIS WASM plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Stable plugin identifier (`demo-filter`, kebab-case).
    pub id: String,
    /// Human-readable plugin name.
    #[serde(default)]
    pub name: String,
    /// Semver plugin release (`0.1.0`).
    pub version: String,
    /// Plugin API version targeted by this build.
    pub api_version: String,
    /// Short description for workbench listings.
    #[serde(default)]
    pub description: String,
    /// Author or organization string.
    #[serde(default)]
    pub author: String,
    /// Capabilities requested by the plugin.
    pub capabilities: Vec<PluginCapability>,
    /// Optional WASM entry when the bundle ships a module.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wasm: Option<WasmModuleSpec>,
}

impl Default for PluginManifest {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            version: "0.1.0".into(),
            api_version: PLUGIN_API_VERSION.into(),
            description: String::new(),
            author: String::new(),
            capabilities: Vec::new(),
            wasm: None,
        }
    }
}

impl PluginManifest {
    /// Parse a manifest from JSON text.
    pub fn from_json(json: &str) -> Result<Self, PluginApiError> {
        serde_json::from_str(json).map_err(|err| PluginApiError::Json(err.to_string()))
    }

    /// Parse a manifest from a local JSON file.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, PluginApiError> {
        let bytes = std::fs::read(path.as_ref())
            .map_err(|err| PluginApiError::InvalidManifest(err.to_string()))?;
        Self::from_json(&String::from_utf8(bytes).map_err(|err| {
            PluginApiError::InvalidManifest(format!("manifest is not UTF-8: {err}"))
        })?)
    }

    /// Load `genegis.plugin.json` from a plugin bundle directory.
    pub fn from_bundle_dir(dir: impl AsRef<Path>) -> Result<Self, PluginApiError> {
        Self::from_path(dir.as_ref().join(MANIFEST_FILENAME))
    }

    /// Serialize the manifest to pretty JSON.
    pub fn to_json_pretty(&self) -> Result<String, PluginApiError> {
        serde_json::to_string_pretty(self).map_err(|err| PluginApiError::Json(err.to_string()))
    }

    /// JSON summary for CLI / workbench listings.
    pub fn summary_json(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "name": self.display_name(),
            "version": self.version,
            "api_version": self.api_version,
            "description": self.description,
            "author": self.author,
            "capabilities": self.capabilities,
            "wasm_entry": self.wasm.as_ref().map(|spec| spec.entry.clone()),
        })
    }

    /// Display name — falls back to `id` when `name` is empty.
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            &self.id
        } else {
            &self.name
        }
    }

    /// Returns true when the manifest declares the capability.
    pub fn has_capability(&self, capability: PluginCapability) -> bool {
        self.capabilities.contains(&capability)
    }

    /// Validate manifest fields and API compatibility against the host SDK.
    pub fn validate(&self) -> Result<(), PluginApiError> {
        validate_id(&self.id)?;
        validate_semver(&self.version, "version")?;

        if self.capabilities.is_empty() {
            return Err(PluginApiError::InvalidManifest(
                "capabilities must not be empty".into(),
            ));
        }

        if !is_api_compatible(&self.api_version) {
            return Err(PluginApiError::UnsupportedApiVersion {
                manifest: self.api_version.clone(),
                host: PLUGIN_API_VERSION.into(),
            });
        }

        if let Some(wasm) = &self.wasm {
            if wasm.entry.trim().is_empty() {
                return Err(PluginApiError::InvalidManifest(
                    "wasm.entry must not be empty".into(),
                ));
            }
            if !wasm.entry.ends_with(".wasm") {
                return Err(PluginApiError::InvalidManifest(
                    "wasm.entry must end with .wasm".into(),
                ));
            }
        }

        Ok(())
    }

    /// Parse, validate, and ensure API compatibility in one step.
    pub fn parse_and_validate(json: &str) -> Result<Self, PluginApiError> {
        let manifest = Self::from_json(json)?;
        manifest.validate()?;
        Ok(manifest)
    }
}

/// Example manifest used in docs and host smoke tests.
pub fn demo_manifest() -> PluginManifest {
    PluginManifest {
        id: "demo-filter".into(),
        name: "Demo Filter".into(),
        version: "0.1.0".into(),
        api_version: PLUGIN_API_VERSION.into(),
        description: "Example analysis filter plugin".into(),
        author: "GeneGIS".into(),
        capabilities: vec![PluginCapability::AnalysisStep],
        wasm: Some(WasmModuleSpec {
            entry: "demo_filter.wasm".into(),
        }),
    }
}

fn validate_id(id: &str) -> Result<(), PluginApiError> {
    if id.is_empty() {
        return Err(PluginApiError::InvalidManifest(
            "id must not be empty".into(),
        ));
    }

    let valid = id
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-');
    if !valid {
        return Err(PluginApiError::InvalidManifest(
            "id must use lowercase ASCII letters, digits, and hyphens".into(),
        ));
    }

    Ok(())
}

fn validate_semver(value: &str, field: &str) -> Result<(), PluginApiError> {
    let parts: Vec<_> = value.split('.').collect();
    if parts.len() != 3 {
        return Err(PluginApiError::InvalidManifest(format!(
            "{field} must be semver major.minor.patch, got {value:?}"
        )));
    }

    for part in parts {
        if part.is_empty() || !part.chars().all(|ch| ch.is_ascii_digit()) {
            return Err(PluginApiError::InvalidManifest(format!(
                "{field} must be semver major.minor.patch, got {value:?}"
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::CapabilityPolicy;

    #[test]
    fn demo_manifest_validates() {
        let manifest = demo_manifest();
        manifest.validate().expect("valid demo manifest");
        assert!(manifest.has_capability(PluginCapability::AnalysisStep));
    }

    #[test]
    fn parses_manifest_json() {
        let json = r#"{
            "id": "demo-filter",
            "version": "0.1.0",
            "api_version": "0.1.0",
            "capabilities": ["analysis_step"],
            "wasm": { "entry": "demo_filter.wasm" }
        }"#;

        let manifest = PluginManifest::parse_and_validate(json).expect("parse");
        assert_eq!(manifest.id, "demo-filter");
        assert_eq!(manifest.capabilities, vec![PluginCapability::AnalysisStep]);
    }

    #[test]
    fn rejects_unknown_capability_in_json() {
        let json = r#"{
            "id": "bad-plugin",
            "version": "0.1.0",
            "api_version": "0.1.0",
            "capabilities": ["fly_to_moon"]
        }"#;

        assert!(PluginManifest::from_json(json).is_err());
    }

    #[test]
    fn policy_blocks_extra_capabilities() {
        let manifest = demo_manifest();
        let policy = CapabilityPolicy::read_only();
        assert!(policy.validate_manifest(&manifest).is_err());
    }

    #[test]
    fn round_trips_through_json_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join(MANIFEST_FILENAME);
        let manifest = demo_manifest();
        std::fs::write(&path, manifest.to_json_pretty().expect("json")).expect("write");

        let loaded = PluginManifest::from_path(&path).expect("load");
        loaded.validate().expect("validate loaded manifest");
        assert_eq!(loaded, manifest);
    }
}
