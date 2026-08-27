//! Portable local-first, managed-cloud, and air-gapped deployment profiles.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentClass {
    LocalFirst,
    ManagedCloud,
    AirGapped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "mode")]
pub enum DeploymentNetwork {
    Disabled,
    AllowList { origins: BTreeSet<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentPersistence {
    LocalFilesystem,
    ObjectStorage,
    OfflineBundle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentIdentity {
    Local,
    Oidc,
    OfflineDirectory,
}

/// Runtime declaration whose verification semantics do not vary by host class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentProfile {
    pub schema_version: String,
    pub id: String,
    pub class: DeploymentClass,
    pub network: DeploymentNetwork,
    pub persistence: DeploymentPersistence,
    pub identity: DeploymentIdentity,
    pub maximum_concurrent_workflows: u32,
    pub command_workflow_required: bool,
    pub offline_capsule_verification: bool,
    pub allowed_open_formats: BTreeSet<String>,
    pub build_digest: String,
    pub registry_snapshot_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DeploymentProfileError {
    #[error("invalid deployment profile: {0}")]
    Invalid(String),
    #[error("deployment profile serialization failed: {0}")]
    Serialization(String),
}

impl DeploymentProfile {
    pub fn validate(&self) -> Result<(), DeploymentProfileError> {
        let required_formats = BTreeSet::from([
            "cog".to_string(),
            "copc".to_string(),
            "geoparquet".to_string(),
            "pmtiles".to_string(),
            "stac".to_string(),
        ]);
        if self.schema_version != "1.0.0"
            || self.id.trim().is_empty()
            || self.maximum_concurrent_workflows == 0
            || !self.command_workflow_required
            || !self.offline_capsule_verification
            || !required_formats.is_subset(&self.allowed_open_formats)
            || !valid_digest(&self.build_digest)
            || self
                .registry_snapshot_digest
                .as_deref()
                .is_some_and(|value| !valid_digest(value))
        {
            return Err(DeploymentProfileError::Invalid(
                "required identity, verification, format, concurrency, or digest is absent".into(),
            ));
        }
        match self.class {
            DeploymentClass::LocalFirst
                if self.persistence != DeploymentPersistence::LocalFilesystem
                    || self.identity != DeploymentIdentity::Local =>
            {
                Err(DeploymentProfileError::Invalid(
                    "local-first requires local filesystem and local identity".into(),
                ))
            }
            DeploymentClass::ManagedCloud
                if self.persistence != DeploymentPersistence::ObjectStorage
                    || self.identity != DeploymentIdentity::Oidc
                    || !matches!(self.network, DeploymentNetwork::AllowList { ref origins } if !origins.is_empty()) =>
            {
                Err(DeploymentProfileError::Invalid(
                    "managed cloud requires OIDC, object storage, and a network allowlist".into(),
                ))
            }
            DeploymentClass::AirGapped
                if self.network != DeploymentNetwork::Disabled
                    || self.persistence != DeploymentPersistence::OfflineBundle
                    || self.identity != DeploymentIdentity::OfflineDirectory
                    || self.registry_snapshot_digest.is_none() =>
            {
                Err(DeploymentProfileError::Invalid(
                    "air-gapped requires disabled network and pinned offline bundles".into(),
                ))
            }
            _ => Ok(()),
        }
    }

    pub fn stable_digest(&self) -> Result<String, DeploymentProfileError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)
            .map_err(|error| DeploymentProfileError::Serialization(error.to_string()))?;
        Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
    }
}

fn valid_digest(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_profiles_validate_and_air_gap_fails_closed() {
        for json in [
            include_str!("../../../deploy/profiles/local-first.json"),
            include_str!("../../../deploy/profiles/managed-cloud.json"),
            include_str!("../../../deploy/profiles/air-gapped.json"),
        ] {
            let profile: DeploymentProfile = serde_json::from_str(json).expect("profile JSON");
            profile.validate().expect("valid profile");
            assert!(profile
                .stable_digest()
                .expect("digest")
                .starts_with("sha256:"));
        }
        let mut air_gap: DeploymentProfile =
            serde_json::from_str(include_str!("../../../deploy/profiles/air-gapped.json"))
                .expect("air gap");
        air_gap.network = DeploymentNetwork::AllowList {
            origins: BTreeSet::from(["https://example.test".into()]),
        };
        assert!(air_gap.validate().is_err());
    }
}
