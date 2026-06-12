use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::capability::PluginCapability;
use crate::error::PluginApiError;
use crate::manifest::PluginManifest;

/// Host-side capability allow-list applied before loading a WASM plugin.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CapabilityPolicy {
    /// Capabilities the host is willing to grant plugins.
    pub allow: Vec<PluginCapability>,
}

impl CapabilityPolicy {
    /// Create a policy that grants every Phase 4 alpha capability.
    pub fn permissive() -> Self {
        Self {
            allow: PluginCapability::all().to_vec(),
        }
    }

    /// Create a read-only policy suitable for catalog inspection plugins.
    pub fn read_only() -> Self {
        Self {
            allow: vec![PluginCapability::ReadCatalog, PluginCapability::ReadStorage],
        }
    }

    /// Returns true when the policy grants the capability.
    pub fn allows(&self, capability: PluginCapability) -> bool {
        self.allow.contains(&capability)
    }

    /// Effective capabilities after intersecting manifest declarations with host policy.
    pub fn effective_capabilities<'a>(
        &'a self,
        manifest: &'a PluginManifest,
    ) -> impl Iterator<Item = PluginCapability> + 'a {
        manifest
            .capabilities
            .iter()
            .copied()
            .filter(|capability| self.allows(*capability))
    }

    /// Verify that the manifest only requests capabilities allowed by this policy.
    pub fn validate_manifest(&self, manifest: &PluginManifest) -> Result<(), PluginApiError> {
        for capability in &manifest.capabilities {
            if !self.allows(*capability) {
                return Err(PluginApiError::CapabilityDenied(capability.to_string()));
            }
        }
        Ok(())
    }

    /// Verify that the plugin declared a capability before the host invokes it.
    pub fn require_capability(
        &self,
        manifest: &PluginManifest,
        capability: PluginCapability,
    ) -> Result<(), PluginApiError> {
        if !manifest.has_capability(capability) {
            return Err(PluginApiError::CapabilityNotGranted(
                capability.to_string(),
            ));
        }
        if !self.allows(capability) {
            return Err(PluginApiError::CapabilityDenied(capability.to_string()));
        }
        Ok(())
    }

    /// Collect effective capabilities into a set for quick membership checks.
    pub fn effective_set(&self, manifest: &PluginManifest) -> HashSet<PluginCapability> {
        self.effective_capabilities(manifest).collect()
    }
}
