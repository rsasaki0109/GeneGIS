//! GeneGIS WASM plugin host — discover manifests, enforce capability policy, load modules.

pub mod discover;
pub mod error;
pub mod loader;
pub mod registry;
pub mod solution_pack;
pub mod workflow;

pub use discover::{discover_bundle, discover_plugins, find_plugin, PluginEntry};
pub use error::PluginHostError;
pub use loader::{LoadedPlugin, PluginHost};
pub use registry::{
    sign_plugin_release, sign_plugin_revocation, PluginRegistry, PluginRegistryEntry,
    PluginRegistryPolicy, PluginReleaseSignature, PluginRevocation,
};
pub use solution_pack::{
    admit_solution_pack_workflow, seal_solution_pack, verify_solution_pack, SolutionDomain,
    SolutionPackAdmissionReceipt, SolutionPackDraft, SolutionPackManifest,
    SolutionPluginRequirement,
};
pub use workflow::{
    execute_plugin_registry_operation, PluginRegistryOperation, PluginRegistryOperationReceipt,
};
