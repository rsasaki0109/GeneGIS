//! GeneGIS plugin SDK — manifest schema, capability model, and version contract.
//!
//! WASM hosts load [`PluginManifest`] files, intersect declared
//! [`PluginCapability`] values with a [`CapabilityPolicy`], and only then
//! instantiate plugin modules (RFC D7).

pub mod capability;
pub mod error;
pub mod manifest;
pub mod policy;
pub mod version;

pub use capability::PluginCapability;
pub use error::PluginApiError;
pub use manifest::{demo_manifest, PluginManifest, WasmModuleSpec};
pub use policy::CapabilityPolicy;
pub use version::{is_api_compatible, MANIFEST_FILENAME, PLUGIN_API_VERSION};
