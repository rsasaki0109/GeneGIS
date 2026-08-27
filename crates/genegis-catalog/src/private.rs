//! Access-bound federation across local, organization-private, and public catalogs.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{CatalogError, FederatedCatalog, StacEndpoint};

/// Catalog trust/access boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogVisibility {
    Local,
    OrganizationPrivate,
    Public,
}

/// Access rule for one stable endpoint identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogAccessRule {
    pub endpoint_id: String,
    pub visibility: CatalogVisibility,
    /// Required only for local/private endpoints.
    pub organization_id: Option<String>,
    /// Private endpoint is admitted if either subject or role is allowlisted.
    pub allowed_subjects: BTreeSet<String>,
    pub allowed_roles: BTreeSet<String>,
}

/// Versioned policy covering every endpoint in the requested federation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FederatedCatalogPolicy {
    pub schema_version: String,
    pub policy_id: String,
    pub policy_version: String,
    pub rules: BTreeMap<String, CatalogAccessRule>,
}

/// Caller identity supplied by the host's authentication boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogAccessContext {
    pub subject_id: String,
    pub organization_id: Option<String>,
    pub roles: BTreeSet<String>,
}

/// Visible endpoint with immutable source identity and secret-free auth declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmittedCatalogEndpoint {
    pub endpoint: StacEndpoint,
    pub visibility: CatalogVisibility,
    pub endpoint_identity_digest: String,
}

/// Fail-closed admission result. Denied endpoint identities and URLs are not disclosed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateFederationAdmission {
    pub schema_version: String,
    pub policy_digest: String,
    pub subject_id: String,
    pub admitted: Vec<AdmittedCatalogEndpoint>,
    pub denied_count: usize,
    pub admission_digest: String,
}

impl PrivateFederationAdmission {
    /// Construct a search coordinator containing only endpoints admitted by policy.
    pub fn federated_catalog(&self) -> Result<FederatedCatalog, CatalogError> {
        verify_private_federation_admission(self)?;
        let mut catalog = FederatedCatalog::new();
        for admitted in &self.admitted {
            catalog.register(admitted.endpoint.clone());
        }
        Ok(catalog)
    }
}

/// Admit an exact endpoint set without leaking inaccessible endpoint details.
pub fn admit_private_federation(
    endpoints: &[StacEndpoint],
    policy: &FederatedCatalogPolicy,
    context: &CatalogAccessContext,
) -> Result<PrivateFederationAdmission, CatalogError> {
    validate(endpoints, policy, context)?;
    let policy_digest = digest(policy)?;
    let mut admitted = Vec::new();
    let mut denied_count = 0;
    for endpoint in endpoints {
        let rule = policy
            .rules
            .get(&endpoint.id)
            .expect("validated rule coverage");
        if visible(rule, context) {
            admitted.push(AdmittedCatalogEndpoint {
                endpoint: endpoint.clone(),
                visibility: rule.visibility,
                endpoint_identity_digest: digest(endpoint)?,
            });
        } else {
            denied_count += 1;
        }
    }
    admitted.sort_by(|left, right| left.endpoint.id.cmp(&right.endpoint.id));
    let mut result = PrivateFederationAdmission {
        schema_version: "0.1.0".into(),
        policy_digest,
        subject_id: context.subject_id.clone(),
        admitted,
        denied_count,
        admission_digest: String::new(),
    };
    result.admission_digest = admission_digest(&result)?;
    verify_private_federation_admission(&result)?;
    Ok(result)
}

/// Recompute endpoint and admission identities before a search is allowed.
pub fn verify_private_federation_admission(
    admission: &PrivateFederationAdmission,
) -> Result<(), CatalogError> {
    if admission.schema_version != "0.1.0"
        || admission.subject_id.trim().is_empty()
        || !valid_digest(&admission.policy_digest)
        || admission.admitted.iter().any(|item| {
            digest(&item.endpoint).ok().as_deref() != Some(&item.endpoint_identity_digest)
        })
        || admission_digest(admission)? != admission.admission_digest
    {
        return Err(CatalogError::InvalidRegistry(
            "private federation admission identity mismatch".into(),
        ));
    }
    Ok(())
}

fn visible(rule: &CatalogAccessRule, context: &CatalogAccessContext) -> bool {
    match rule.visibility {
        CatalogVisibility::Public => true,
        CatalogVisibility::Local => rule.organization_id == context.organization_id,
        CatalogVisibility::OrganizationPrivate => {
            rule.organization_id == context.organization_id
                && (rule.allowed_subjects.contains(&context.subject_id)
                    || !rule.allowed_roles.is_disjoint(&context.roles))
        }
    }
}

fn validate(
    endpoints: &[StacEndpoint],
    policy: &FederatedCatalogPolicy,
    context: &CatalogAccessContext,
) -> Result<(), CatalogError> {
    let endpoint_ids = endpoints
        .iter()
        .map(|endpoint| endpoint.id.as_str())
        .collect::<BTreeSet<_>>();
    if policy.schema_version != "0.1.0"
        || policy.policy_id.trim().is_empty()
        || policy.policy_version.trim().is_empty()
        || context.subject_id.trim().is_empty()
        || endpoint_ids.len() != endpoints.len()
        || endpoint_ids.len() != policy.rules.len()
        || endpoints
            .iter()
            .any(|endpoint| !policy.rules.contains_key(&endpoint.id))
        || policy.rules.iter().any(|(id, rule)| {
            id != &rule.endpoint_id
                || id.trim().is_empty()
                || (rule.visibility != CatalogVisibility::Public
                    && rule.organization_id.as_deref().is_none_or(str::is_empty))
                || (rule.visibility == CatalogVisibility::Public && rule.organization_id.is_some())
        })
    {
        return Err(CatalogError::InvalidRegistry(
            "private federation policy or endpoint coverage is invalid".into(),
        ));
    }
    Ok(())
}

fn admission_digest(value: &PrivateFederationAdmission) -> Result<String, CatalogError> {
    let mut semantic = value.clone();
    semantic.admission_digest.clear();
    digest(&semantic)
}

fn digest<T: Serialize>(value: &T) -> Result<String, CatalogError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| CatalogError::InvalidRegistry(error.to_string()))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn valid_digest(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Vec<StacEndpoint>, FederatedCatalogPolicy) {
        let endpoints = vec![
            StacEndpoint::new("local", "fixtures/local.json"),
            StacEndpoint::new("private", "https://private.example.test/stac"),
            StacEndpoint::new("public", "https://public.example.test/stac"),
        ];
        let rules = BTreeMap::from([
            (
                "local".into(),
                CatalogAccessRule {
                    endpoint_id: "local".into(),
                    visibility: CatalogVisibility::Local,
                    organization_id: Some("org-a".into()),
                    allowed_subjects: BTreeSet::new(),
                    allowed_roles: BTreeSet::new(),
                },
            ),
            (
                "private".into(),
                CatalogAccessRule {
                    endpoint_id: "private".into(),
                    visibility: CatalogVisibility::OrganizationPrivate,
                    organization_id: Some("org-a".into()),
                    allowed_subjects: BTreeSet::new(),
                    allowed_roles: BTreeSet::from(["analyst".into()]),
                },
            ),
            (
                "public".into(),
                CatalogAccessRule {
                    endpoint_id: "public".into(),
                    visibility: CatalogVisibility::Public,
                    organization_id: None,
                    allowed_subjects: BTreeSet::new(),
                    allowed_roles: BTreeSet::new(),
                },
            ),
        ]);
        (
            endpoints,
            FederatedCatalogPolicy {
                schema_version: "0.1.0".into(),
                policy_id: "org-a-catalogs".into(),
                policy_version: "1".into(),
                rules,
            },
        )
    }

    #[test]
    fn combines_visible_local_private_and_public_sources() {
        let (endpoints, policy) = fixture();
        let admission = admit_private_federation(
            &endpoints,
            &policy,
            &CatalogAccessContext {
                subject_id: "alice".into(),
                organization_id: Some("org-a".into()),
                roles: BTreeSet::from(["analyst".into()]),
            },
        )
        .expect("admit");
        assert_eq!(admission.admitted.len(), 3);
        assert_eq!(admission.denied_count, 0);
        assert_eq!(
            admission
                .federated_catalog()
                .expect("catalog")
                .endpoints()
                .len(),
            3
        );
    }

    #[test]
    fn hides_private_identity_and_rejects_tampering_or_incomplete_policy() {
        let (endpoints, mut policy) = fixture();
        let admission = admit_private_federation(
            &endpoints,
            &policy,
            &CatalogAccessContext {
                subject_id: "outside".into(),
                organization_id: Some("org-b".into()),
                roles: BTreeSet::new(),
            },
        )
        .expect("public only");
        assert_eq!(
            admission
                .admitted
                .iter()
                .map(|item| item.endpoint.id.as_str())
                .collect::<Vec<_>>(),
            vec!["public"]
        );
        assert_eq!(admission.denied_count, 2);
        let mut tampered = admission;
        tampered.admitted[0].endpoint.url.push_str("/changed");
        assert!(verify_private_federation_admission(&tampered).is_err());
        policy.rules.remove("private");
        assert!(admit_private_federation(
            &endpoints,
            &policy,
            &CatalogAccessContext {
                subject_id: "alice".into(),
                organization_id: Some("org-a".into()),
                roles: BTreeSet::new()
            }
        )
        .is_err());
    }
}
