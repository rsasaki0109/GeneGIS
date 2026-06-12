use std::path::Path;

use genegis_plugin_api::CapabilityPolicy;
use wasmtime::{Engine, Module};

use crate::discover::PluginEntry;
use crate::error::PluginHostError;

/// A WASM module loaded after capability gating.
#[derive(Debug)]
pub struct LoadedPlugin {
    /// Manifest metadata and bundle location.
    pub entry: PluginEntry,
    /// Raw WASM bytes read from the bundle.
    pub wasm_bytes: Vec<u8>,
    /// Parsed module ready for instantiation by the host runtime.
    pub module: Module,
}

/// WASM plugin host with an explicit capability allow-list (RFC D7).
#[derive(Debug)]
pub struct PluginHost {
    policy: CapabilityPolicy,
    engine: Engine,
}

impl Default for PluginHost {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginHost {
    /// Create a host with a permissive capability policy.
    pub fn new() -> Self {
        Self::with_policy(CapabilityPolicy::permissive())
    }

    /// Create a host with an explicit capability policy.
    pub fn with_policy(policy: CapabilityPolicy) -> Self {
        let engine = Engine::default();
        Self { policy, engine }
    }

    /// Access the host capability policy.
    pub fn policy(&self) -> &CapabilityPolicy {
        &self.policy
    }

    /// Access the shared Wasmtime engine.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Discover plugin bundles under a root directory.
    pub fn discover_plugins(
        &self,
        root: impl AsRef<Path>,
    ) -> Result<Vec<PluginEntry>, PluginHostError> {
        crate::discover::discover_plugins(root, &self.policy)
    }

    /// Discover a single plugin bundle directory.
    pub fn discover_bundle(
        &self,
        bundle_dir: impl AsRef<Path>,
    ) -> Result<PluginEntry, PluginHostError> {
        crate::discover::discover_bundle(bundle_dir, &self.policy)
    }

    /// Validate, gate, and compile the WASM module for a plugin bundle.
    pub fn load_bundle(&self, bundle_dir: impl AsRef<Path>) -> Result<LoadedPlugin, PluginHostError> {
        let entry = self.discover_bundle(&bundle_dir)?;
        let wasm_path = entry.wasm_path().ok_or_else(|| {
            PluginHostError::Bundle(format!(
                "plugin {} does not declare a wasm entry",
                entry.manifest.id
            ))
        })?;

        let wasm_bytes = std::fs::read(&wasm_path).map_err(|err| {
            PluginHostError::Bundle(format!(
                "failed to read wasm module {}: {err}",
                wasm_path.display()
            ))
        })?;

        let module = Module::from_binary(&self.engine, &wasm_bytes)
            .map_err(|err| PluginHostError::Wasm(err.to_string()))?;

        Ok(LoadedPlugin {
            entry,
            wasm_bytes,
            module,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use genegis_plugin_api::{PluginCapability, PluginManifest, WasmModuleSpec, MANIFEST_FILENAME};
    use std::path::Path;

    fn write_demo_bundle(dir: &Path, include_wasm: bool) {
        let manifest = PluginManifest {
            id: "demo-filter".into(),
            name: "Demo Filter".into(),
            version: "0.1.0".into(),
            api_version: genegis_plugin_api::PLUGIN_API_VERSION.into(),
            description: "Host smoke plugin".into(),
            author: "GeneGIS".into(),
            capabilities: vec![PluginCapability::AnalysisStep],
            wasm: Some(WasmModuleSpec {
                entry: "demo_filter.wasm".into(),
            }),
        };

        std::fs::write(
            dir.join(MANIFEST_FILENAME),
            manifest.to_json_pretty().expect("json"),
        )
        .expect("write manifest");

        if include_wasm {
            let wasm = wat::parse_str(
                r#"(module (func (export "plugin_info") (result i32) i32.const 42))"#,
            )
            .expect("wat");
            std::fs::write(dir.join("demo_filter.wasm"), wasm).expect("write wasm");
        }
    }

    #[test]
    fn discovers_manifest_only_plugins() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bundle = temp.path().join("demo-filter");
        std::fs::create_dir(&bundle).expect("bundle dir");
        write_demo_bundle(&bundle, false);

        let host = PluginHost::new();
        let entries = host.discover_plugins(temp.path()).expect("discover");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].manifest.id, "demo-filter");
    }

    #[test]
    fn loads_wasm_after_capability_gate() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_demo_bundle(temp.path(), true);

        let host = PluginHost::new();
        let loaded = host.load_bundle(temp.path()).expect("load");
        assert_eq!(loaded.entry.manifest.id, "demo-filter");
        assert!(!loaded.wasm_bytes.is_empty());
    }

    #[test]
    fn restrictive_policy_blocks_load() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_demo_bundle(temp.path(), true);

        let host = PluginHost::with_policy(CapabilityPolicy::read_only());
        assert!(host.load_bundle(temp.path()).is_err());
    }
}
