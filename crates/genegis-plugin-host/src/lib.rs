//! GeneGIS WASM plugin host — discover manifests, enforce capability policy, load modules.

pub mod discover;
pub mod error;
pub mod loader;

pub use discover::{discover_bundle, discover_plugins, find_plugin, PluginEntry};
pub use error::PluginHostError;
pub use loader::{LoadedPlugin, PluginHost};
