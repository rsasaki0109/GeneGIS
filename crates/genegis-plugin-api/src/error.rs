use thiserror::Error;

/// Errors raised while parsing or validating plugin manifests.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PluginApiError {
    /// JSON could not be parsed.
    #[error("invalid plugin manifest JSON: {0}")]
    Json(String),
    /// A required field failed validation.
    #[error("invalid plugin manifest: {0}")]
    InvalidManifest(String),
    /// The manifest targets an unsupported plugin API version.
    #[error("unsupported plugin API version: manifest={manifest}, host={host}")]
    UnsupportedApiVersion {
        /// Version declared by the plugin manifest.
        manifest: String,
        /// Plugin API version supported by the host.
        host: String,
    },
    /// A requested capability is not declared by the plugin.
    #[error("capability not granted: {0}")]
    CapabilityNotGranted(String),
    /// A requested capability is blocked by host policy.
    #[error("capability denied by host policy: {0}")]
    CapabilityDenied(String),
}
