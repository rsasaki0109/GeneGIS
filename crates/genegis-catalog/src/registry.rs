use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use genegis_core::{Command, CommandEnvelope, ProvenanceStore};
use genegis_workflow::GeoWorkflow;
use serde::{Deserialize, Serialize};

use crate::{
    CatalogError, FederatedCatalog, FederatedSearchResult, StacAuthentication, StacEndpoint,
};

pub const ENDPOINT_REGISTRY_PATH: &str = ".genegis/catalog/endpoints.json";
pub const ENDPOINT_REGISTRY_ENV: &str = "GENEGIS_STAC_ENDPOINT_REGISTRY";

pub fn endpoint_registry_path() -> PathBuf {
    std::env::var(ENDPOINT_REGISTRY_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(ENDPOINT_REGISTRY_PATH))
}

/// Persistent, auditable STAC endpoint registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointRegistry {
    pub schema_version: u32,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub endpoints: Vec<StacEndpoint>,
    #[serde(default)]
    pub command_history: Vec<CommandEnvelope>,
    #[serde(default)]
    pub workflows: Vec<GeoWorkflow>,
    #[serde(default)]
    pub provenance: ProvenanceStore,
}

impl Default for EndpointRegistry {
    fn default() -> Self {
        Self {
            schema_version: 1,
            updated_at: Utc::now(),
            endpoints: Vec::new(),
            command_history: Vec::new(),
            workflows: Vec::new(),
            provenance: ProvenanceStore::default(),
        }
    }
}

impl EndpointRegistry {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, CatalogError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }
        let json = std::fs::read_to_string(path)
            .map_err(|error| CatalogError::InvalidRegistry(format!("read {}: {error}", path.display())))?;
        let registry: Self = serde_json::from_str(&json)
            .map_err(|error| CatalogError::InvalidRegistry(format!("parse {}: {error}", path.display())))?;
        if registry.schema_version != 1 {
            return Err(CatalogError::InvalidRegistry(format!(
                "unsupported schema version {}",
                registry.schema_version
            )));
        }
        Ok(registry)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), CatalogError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                CatalogError::InvalidRegistry(format!("create {}: {error}", parent.display()))
            })?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|error| CatalogError::InvalidRegistry(format!("serialize: {error}")))?;
        std::fs::write(path, json)
            .map_err(|error| CatalogError::InvalidRegistry(format!("write {}: {error}", path.display())))
    }

    /// Apply a catalog mutation only when represented by both a Command and GeoWorkflow.
    pub fn apply(
        &mut self,
        envelope: CommandEnvelope,
        workflow: GeoWorkflow,
    ) -> Result<(), CatalogError> {
        let (action, target, details) = match &envelope.command {
            Command::RegisterStacEndpoint {
                endpoint_id,
                title,
                url,
                auth_kind,
                auth_env,
                auth_header,
            } => {
                validate_endpoint_id(endpoint_id)?;
                let authentication =
                    authentication_from_fields(auth_kind, auth_env.as_deref(), auth_header.as_deref())?;
                self.endpoints.retain(|endpoint| endpoint.id != *endpoint_id);
                self.endpoints.push(
                    StacEndpoint::new(endpoint_id, url)
                        .with_authentication(authentication)
                        .with_title(title),
                );
                (
                    "register_stac_endpoint",
                    endpoint_id.clone(),
                    serde_json::json!({
                        "url": url,
                        "authentication": {
                            "kind": auth_kind,
                            "env_var": auth_env,
                            "header": auth_header,
                        },
                        "command_id": envelope.id,
                    }),
                )
            }
            Command::RemoveStacEndpoint { endpoint_id } => {
                let before = self.endpoints.len();
                self.endpoints.retain(|endpoint| endpoint.id != *endpoint_id);
                if self.endpoints.len() == before {
                    return Err(CatalogError::NotFound(endpoint_id.clone()));
                }
                (
                    "remove_stac_endpoint",
                    endpoint_id.clone(),
                    serde_json::json!({ "command_id": envelope.id }),
                )
            }
            other => {
                return Err(CatalogError::InvalidRegistry(format!(
                    "unsupported registry command: {other:?}"
                )))
            }
        };

        self.updated_at = envelope.timestamp;
        self.provenance.record_workflow(
            workflow.id.to_string(),
            format!("{:?}", envelope.origin).to_lowercase(),
            action,
            target,
            details,
        );
        self.command_history.push(envelope);
        self.workflows.push(workflow);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&StacEndpoint> {
        self.endpoints.iter().find(|endpoint| endpoint.id == id)
    }

    pub fn record_search(
        &mut self,
        envelope: CommandEnvelope,
        workflow: GeoWorkflow,
        result: &FederatedSearchResult,
    ) -> Result<(), CatalogError> {
        let endpoint_ids = match &envelope.command {
            Command::SearchFederatedStac { endpoint_ids, .. } => endpoint_ids,
            other => {
                return Err(CatalogError::InvalidRegistry(format!(
                    "expected federated search command, got {other:?}"
                )))
            }
        };
        self.updated_at = envelope.timestamp;
        self.provenance.record_workflow(
            workflow.id.to_string(),
            format!("{:?}", envelope.origin).to_lowercase(),
            "search_federated_stac",
            endpoint_ids.join(","),
            serde_json::json!({
                "command_id": envelope.id,
                "endpoint_ids": endpoint_ids,
                "successful_endpoints": result.successful_endpoints(),
                "failed_endpoints": result.failed_endpoints(),
                "item_count": result.items.len(),
                "crs": "EPSG:4326",
                "units": "degrees",
            }),
        );
        self.command_history.push(envelope);
        self.workflows.push(workflow);
        Ok(())
    }

    pub fn federated_catalog(&self, ids: &[String]) -> Result<FederatedCatalog, CatalogError> {
        let selected = if ids.is_empty() {
            self.endpoints.clone()
        } else {
            ids.iter()
                .map(|id| {
                    self.get(id)
                        .cloned()
                        .ok_or_else(|| CatalogError::NotFound(id.clone()))
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut catalog = FederatedCatalog::new();
        for endpoint in selected {
            catalog.register(endpoint);
        }
        Ok(catalog)
    }
}

impl StacEndpoint {
    fn with_title(mut self, title: &str) -> Self {
        self.title = title.to_string();
        self
    }
}

fn validate_endpoint_id(id: &str) -> Result<(), CatalogError> {
    if id.is_empty()
        || !id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(CatalogError::InvalidRegistry(format!(
            "endpoint id {id:?} must contain only ASCII letters, digits, '-' or '_'"
        )));
    }
    Ok(())
}

fn authentication_from_fields(
    kind: &str,
    env_var: Option<&str>,
    header: Option<&str>,
) -> Result<StacAuthentication, CatalogError> {
    match kind {
        "anonymous" => Ok(StacAuthentication::Anonymous),
        "bearer_env" => Ok(StacAuthentication::BearerEnv {
            env_var: required_auth_field(env_var, "auth environment variable")?.into(),
        }),
        "header_env" => Ok(StacAuthentication::HeaderEnv {
            header: required_auth_field(header, "auth header")?.into(),
            env_var: required_auth_field(env_var, "auth environment variable")?.into(),
        }),
        _ => Err(CatalogError::InvalidRegistry(format!(
            "unsupported authentication kind {kind:?}"
        ))),
    }
}

fn required_auth_field<'a>(
    value: Option<&'a str>,
    label: &str,
) -> Result<&'a str, CatalogError> {
    value
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| CatalogError::InvalidRegistry(format!("{label} is required")))
}

#[cfg(test)]
mod tests {
    use genegis_core::{CommandOrigin, CommandEnvelope};
    use genegis_workflow::stac_endpoint_registry_template;

    use super::*;

    #[test]
    fn persists_endpoint_command_workflow_and_provenance() {
        let path = std::env::temp_dir().join(format!(
            "genegis-endpoints-{}.json",
            Utc::now().timestamp_nanos_opt().expect("timestamp")
        ));
        let command = Command::RegisterStacEndpoint {
            endpoint_id: "local".into(),
            title: "Local fixture".into(),
            url: "examples/stac/sample-search.json".into(),
            auth_kind: "anonymous".into(),
            auth_env: None,
            auth_header: None,
        };
        let workflow = stac_endpoint_registry_template("register", "local");
        let mut registry = EndpointRegistry::default();
        registry
            .apply(CommandEnvelope::new(CommandOrigin::Cli, command), workflow)
            .expect("apply");
        registry.save(&path).expect("save");

        let restored = EndpointRegistry::load(&path).expect("load");
        assert_eq!(restored.endpoints.len(), 1);
        assert_eq!(restored.command_history.len(), 1);
        assert_eq!(restored.workflows.len(), 1);
        assert_eq!(restored.provenance.entries.len(), 1);
        assert_eq!(restored.get("local").expect("endpoint").title, "Local fixture");

        std::fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn stores_only_auth_environment_reference() {
        let command = Command::RegisterStacEndpoint {
            endpoint_id: "secure".into(),
            title: "Secure".into(),
            url: "https://example.com/stac".into(),
            auth_kind: "bearer_env".into(),
            auth_env: Some("GENEGIS_TEST_TOKEN".into()),
            auth_header: None,
        };
        let mut registry = EndpointRegistry::default();
        registry
            .apply(
                CommandEnvelope::new(CommandOrigin::Cli, command),
                stac_endpoint_registry_template("register", "secure"),
            )
            .expect("apply");
        let json = serde_json::to_string(&registry).expect("json");
        assert!(json.contains("GENEGIS_TEST_TOKEN"));
        assert!(!json.contains("secret-value"));
    }
}
