use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::PluginApiError;

/// Capability granted to a plugin by the host sandbox (RFC D7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginCapability {
    /// Read dataset metadata from the GeneGIS catalog.
    ReadCatalog,
    /// Perform cloud-native asset IO via `genegis-storage` (HTTP range reads).
    ReadStorage,
    /// Register or execute an analysis workflow step.
    AnalysisStep,
    /// Hook into the render pipeline (choropleth, tiles, overlays).
    RenderHook,
    /// Export maps or tabular artifacts from analysis runs.
    ExportArtifact,
    /// Publish STAC items derived from catalog assets.
    PublishStac,
}

impl PluginCapability {
    /// Stable string identifier used in manifest JSON and host policy files.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadCatalog => "read_catalog",
            Self::ReadStorage => "read_storage",
            Self::AnalysisStep => "analysis_step",
            Self::RenderHook => "render_hook",
            Self::ExportArtifact => "export_artifact",
            Self::PublishStac => "publish_stac",
        }
    }

    /// All capabilities defined by the Phase 4 alpha contract.
    pub fn all() -> &'static [Self] {
        &[
            Self::ReadCatalog,
            Self::ReadStorage,
            Self::AnalysisStep,
            Self::RenderHook,
            Self::ExportArtifact,
            Self::PublishStac,
        ]
    }
}

impl FromStr for PluginCapability {
    type Err = PluginApiError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "read_catalog" => Ok(Self::ReadCatalog),
            "read_storage" => Ok(Self::ReadStorage),
            "analysis_step" => Ok(Self::AnalysisStep),
            "render_hook" => Ok(Self::RenderHook),
            "export_artifact" => Ok(Self::ExportArtifact),
            "publish_stac" => Ok(Self::PublishStac),
            other => Err(PluginApiError::InvalidManifest(format!(
                "unknown capability {other:?}"
            ))),
        }
    }
}

impl std::fmt::Display for PluginCapability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
