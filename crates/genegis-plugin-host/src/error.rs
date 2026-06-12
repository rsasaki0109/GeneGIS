use thiserror::Error;

use genegis_plugin_api::PluginApiError;

/// Errors raised while discovering or loading WASM plugins.
#[derive(Debug, Error)]
pub enum PluginHostError {
    /// Manifest parsing or validation failed.
    #[error(transparent)]
    Manifest(#[from] PluginApiError),
    /// The plugin bundle directory is missing or unreadable.
    #[error("plugin bundle error: {0}")]
    Bundle(String),
    /// WASM module loading failed.
    #[error("wasm load error: {0}")]
    Wasm(String),
    /// The requested plugin id was not found in the scanned directory.
    #[error("plugin not found: {0}")]
    NotFound(String),
}
