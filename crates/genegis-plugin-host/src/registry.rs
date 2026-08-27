//! Signed SDK v1 plugin registry with explicit revocation.

use std::collections::BTreeMap;

use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::DateTime;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use genegis_plugin_api::PluginManifest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::PluginHostError;

/// Detached Ed25519 signature over canonical manifest or revocation bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginReleaseSignature {
    pub key_id: String,
    pub algorithm: String,
    pub signature_base64: String,
}

/// Registry trust roots. Public keys are base64-encoded Ed25519 keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRegistryPolicy {
    pub schema_version: String,
    pub trusted_signers: BTreeMap<String, String>,
}

/// Signed revocation for one exact release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRevocation {
    pub plugin_id: String,
    pub version: String,
    pub reason: String,
    pub revoked_at: String,
    pub signature: PluginReleaseSignature,
    pub revocation_digest: String,
}

/// One immutable signed release entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRegistryEntry {
    pub manifest: PluginManifest,
    pub manifest_digest: String,
    pub signature: PluginReleaseSignature,
    pub published_at: String,
    pub revocation: Option<PluginRevocation>,
}

/// In-memory portable registry document; persistence is host-owned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRegistry {
    pub schema_version: String,
    pub policy: PluginRegistryPolicy,
    pub entries: BTreeMap<String, PluginRegistryEntry>,
}

impl PluginRegistry {
    pub fn new(policy: PluginRegistryPolicy) -> Result<Self, PluginHostError> {
        if policy.schema_version != "1.0.0"
            || policy.trusted_signers.is_empty()
            || policy
                .trusted_signers
                .iter()
                .any(|(id, key)| id.trim().is_empty() || decode_key(key).is_err())
        {
            return Err(PluginHostError::Bundle(
                "plugin registry policy is invalid".into(),
            ));
        }
        Ok(Self {
            schema_version: "1.0.0".into(),
            policy,
            entries: BTreeMap::new(),
        })
    }

    /// Verify and publish one immutable plugin release.
    pub fn publish(
        &mut self,
        manifest: PluginManifest,
        signature: PluginReleaseSignature,
        published_at: &str,
    ) -> Result<&PluginRegistryEntry, PluginHostError> {
        manifest.validate()?;
        parse_time(published_at)?;
        verify_signed_bytes(&self.policy, &signature, &canonical(&manifest)?)?;
        let key = release_key(&manifest.id, &manifest.version);
        if self.entries.contains_key(&key) {
            return Err(PluginHostError::Bundle(
                "plugin release already exists and is immutable".into(),
            ));
        }
        self.entries.insert(
            key.clone(),
            PluginRegistryEntry {
                manifest_digest: digest(&manifest)?,
                manifest,
                signature,
                published_at: published_at.into(),
                revocation: None,
            },
        );
        Ok(self.entries.get(&key).expect("inserted registry entry"))
    }

    /// Verify and attach a signed revocation. Revoked releases cannot resolve.
    pub fn revoke(&mut self, revocation: PluginRevocation) -> Result<(), PluginHostError> {
        parse_time(&revocation.revoked_at)?;
        if revocation.reason.trim().is_empty()
            || revocation.revocation_digest != revocation_digest(&revocation)?
        {
            return Err(PluginHostError::Bundle(
                "plugin revocation is invalid".into(),
            ));
        }
        let payload = revocation_payload(
            &revocation.plugin_id,
            &revocation.version,
            &revocation.reason,
            &revocation.revoked_at,
        )?;
        verify_signed_bytes(&self.policy, &revocation.signature, &payload)?;
        let key = release_key(&revocation.plugin_id, &revocation.version);
        let entry = self
            .entries
            .get_mut(&key)
            .ok_or_else(|| PluginHostError::NotFound(key.clone()))?;
        if entry.revocation.is_some() {
            return Err(PluginHostError::Bundle(
                "plugin release is already revoked".into(),
            ));
        }
        entry.revocation = Some(revocation);
        Ok(())
    }

    /// Resolve an active release and re-verify signature and identities.
    pub fn resolve(
        &self,
        plugin_id: &str,
        version: &str,
    ) -> Result<&PluginRegistryEntry, PluginHostError> {
        let key = release_key(plugin_id, version);
        let entry = self
            .entries
            .get(&key)
            .ok_or_else(|| PluginHostError::NotFound(key.clone()))?;
        if entry.revocation.is_some() {
            return Err(PluginHostError::Bundle(
                "plugin release has been revoked".into(),
            ));
        }
        if digest(&entry.manifest)? != entry.manifest_digest {
            return Err(PluginHostError::Bundle(
                "plugin manifest digest mismatch".into(),
            ));
        }
        verify_signed_bytes(&self.policy, &entry.signature, &canonical(&entry.manifest)?)?;
        Ok(entry)
    }

    /// Verify downloaded artifact bytes against the signed manifest identity.
    pub fn verify_artifact(
        &self,
        plugin_id: &str,
        version: &str,
        bytes: &[u8],
    ) -> Result<(), PluginHostError> {
        let entry = self.resolve(plugin_id, version)?;
        let observed = format!("sha256:{:x}", Sha256::digest(bytes));
        if observed != entry.manifest.artifact_digest {
            return Err(PluginHostError::Bundle(
                "plugin artifact digest mismatch".into(),
            ));
        }
        Ok(())
    }
}

/// Create an Ed25519 release signature for registry publication.
pub fn sign_plugin_release(
    manifest: &PluginManifest,
    key_id: &str,
    signing_key: &[u8; 32],
) -> Result<PluginReleaseSignature, PluginHostError> {
    manifest.validate()?;
    sign(&canonical(manifest)?, key_id, signing_key)
}

/// Create a signed, digest-bound revocation record.
pub fn sign_plugin_revocation(
    plugin_id: &str,
    version: &str,
    reason: &str,
    revoked_at: &str,
    key_id: &str,
    signing_key: &[u8; 32],
) -> Result<PluginRevocation, PluginHostError> {
    if plugin_id.trim().is_empty() || version.trim().is_empty() || reason.trim().is_empty() {
        return Err(PluginHostError::Bundle(
            "revocation identity and reason are required".into(),
        ));
    }
    parse_time(revoked_at)?;
    let signature = sign(
        &revocation_payload(plugin_id, version, reason, revoked_at)?,
        key_id,
        signing_key,
    )?;
    let mut revocation = PluginRevocation {
        plugin_id: plugin_id.into(),
        version: version.into(),
        reason: reason.into(),
        revoked_at: revoked_at.into(),
        signature,
        revocation_digest: String::new(),
    };
    revocation.revocation_digest = revocation_digest(&revocation)?;
    Ok(revocation)
}

fn sign(
    payload: &[u8],
    key_id: &str,
    signing_key: &[u8; 32],
) -> Result<PluginReleaseSignature, PluginHostError> {
    if key_id.trim().is_empty() {
        return Err(PluginHostError::Bundle("signer key ID is required".into()));
    }
    let signature = SigningKey::from_bytes(signing_key).sign(payload);
    Ok(PluginReleaseSignature {
        key_id: key_id.into(),
        algorithm: "ed25519".into(),
        signature_base64: STANDARD.encode(signature.to_bytes()),
    })
}

fn verify_signed_bytes(
    policy: &PluginRegistryPolicy,
    signature: &PluginReleaseSignature,
    payload: &[u8],
) -> Result<(), PluginHostError> {
    if signature.algorithm != "ed25519" {
        return Err(PluginHostError::Bundle(
            "unsupported plugin signature algorithm".into(),
        ));
    }
    let public = policy
        .trusted_signers
        .get(&signature.key_id)
        .ok_or_else(|| PluginHostError::Bundle("plugin signer is not trusted".into()))?;
    let key = decode_key(public)?;
    let bytes = STANDARD
        .decode(&signature.signature_base64)
        .map_err(|_| PluginHostError::Bundle("invalid plugin signature base64".into()))?;
    let signature = Signature::from_slice(&bytes)
        .map_err(|_| PluginHostError::Bundle("invalid plugin signature bytes".into()))?;
    key.verify(payload, &signature)
        .map_err(|_| PluginHostError::Bundle("plugin signature did not verify".into()))
}

fn decode_key(value: &str) -> Result<VerifyingKey, PluginHostError> {
    let bytes = STANDARD
        .decode(value)
        .map_err(|_| PluginHostError::Bundle("invalid trusted signer base64".into()))?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| PluginHostError::Bundle("trusted signer must be 32 bytes".into()))?;
    VerifyingKey::from_bytes(&bytes)
        .map_err(|_| PluginHostError::Bundle("invalid trusted signer key".into()))
}

fn release_key(plugin_id: &str, version: &str) -> String {
    format!("{plugin_id}@{version}")
}

fn canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, PluginHostError> {
    serde_json::to_vec(value).map_err(|error| PluginHostError::Bundle(error.to_string()))
}

fn digest<T: Serialize>(value: &T) -> Result<String, PluginHostError> {
    Ok(format!("sha256:{:x}", Sha256::digest(canonical(value)?)))
}

fn revocation_payload(
    plugin_id: &str,
    version: &str,
    reason: &str,
    revoked_at: &str,
) -> Result<Vec<u8>, PluginHostError> {
    canonical(&(plugin_id, version, reason, revoked_at))
}

fn revocation_digest(value: &PluginRevocation) -> Result<String, PluginHostError> {
    let mut semantic = value.clone();
    semantic.revocation_digest.clear();
    digest(&semantic)
}

fn parse_time(value: &str) -> Result<(), PluginHostError> {
    DateTime::parse_from_rfc3339(value)
        .map(|_| ())
        .map_err(|_| PluginHostError::Bundle("timestamp must be RFC 3339".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use genegis_plugin_api::demo_manifest;

    fn registry(seed: &[u8; 32]) -> PluginRegistry {
        let public = SigningKey::from_bytes(seed).verifying_key();
        PluginRegistry::new(PluginRegistryPolicy {
            schema_version: "1.0.0".into(),
            trusted_signers: BTreeMap::from([(
                "release".into(),
                STANDARD.encode(public.as_bytes()),
            )]),
        })
        .expect("registry")
    }

    #[test]
    fn publishes_verifies_and_revokes_signed_release() {
        let seed = [7_u8; 32];
        let mut manifest = demo_manifest();
        manifest.artifact_digest = format!("sha256:{:x}", Sha256::digest(b"wasm"));
        let signature = sign_plugin_release(&manifest, "release", &seed).expect("sign");
        let mut registry = registry(&seed);
        registry
            .publish(manifest.clone(), signature, "2026-08-26T10:00:00Z")
            .expect("publish");
        registry
            .verify_artifact(&manifest.id, &manifest.version, b"wasm")
            .expect("artifact");
        assert!(registry
            .verify_artifact(&manifest.id, &manifest.version, b"tampered")
            .is_err());
        let revocation = sign_plugin_revocation(
            &manifest.id,
            &manifest.version,
            "vulnerability",
            "2026-08-26T11:00:00Z",
            "release",
            &seed,
        )
        .expect("revoke signature");
        registry.revoke(revocation).expect("revoke");
        assert!(registry.resolve(&manifest.id, &manifest.version).is_err());
    }

    #[test]
    fn rejects_untrusted_or_tampered_release() {
        let seed = [9_u8; 32];
        let manifest = demo_manifest();
        let signature = sign_plugin_release(&manifest, "release", &[8_u8; 32]).expect("sign other");
        let mut registry = registry(&seed);
        assert!(registry
            .publish(manifest.clone(), signature, "2026-08-26T10:00:00Z")
            .is_err());
        let signature = sign_plugin_release(&manifest, "release", &seed).expect("sign");
        let mut changed = manifest;
        changed.description.push_str(" changed");
        assert!(registry
            .publish(changed, signature, "2026-08-26T10:00:00Z")
            .is_err());
    }
}
