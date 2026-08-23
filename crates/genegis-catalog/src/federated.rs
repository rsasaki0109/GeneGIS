use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::CatalogError;
use crate::external_stac::{fetch_json_bytes, resolve_catalog_url};
use crate::stac::{StacItem, StacLink};

/// Authentication material is resolved from the environment at request time.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StacAuthentication {
    #[default]
    Anonymous,
    BearerEnv {
        env_var: String,
    },
    HeaderEnv {
        header: String,
        env_var: String,
    },
}

/// A named STAC API search endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StacEndpoint {
    /// Stable identifier used in provenance and result summaries.
    pub id: String,
    /// Human-readable endpoint name.
    pub title: String,
    /// STAC API root, explicit `/search` URL, or local ItemCollection path.
    pub url: String,
    /// Secret-free authentication configuration.
    #[serde(default)]
    pub authentication: StacAuthentication,
}

impl StacEndpoint {
    /// Create an endpoint whose title defaults to its identifier.
    pub fn new(id: impl Into<String>, url: impl Into<String>) -> Self {
        let id = id.into();
        Self {
            title: id.clone(),
            id,
            url: url.into(),
            authentication: StacAuthentication::Anonymous,
        }
    }

    pub fn with_authentication(mut self, authentication: StacAuthentication) -> Self {
        self.authentication = authentication;
        self
    }
}

/// Portable subset of a STAC API Item Search request.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StacSearchRequest {
    /// Spatial filter in WGS84 longitude/latitude order.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<[f64; 4]>,
    /// RFC 3339 instant or interval.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub datetime: Option<String>,
    /// Optional collection identifiers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collections: Vec<String>,
    /// Maximum number of deduplicated items returned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

/// Minimal STAC ItemCollection returned by Item Search.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StacItemCollection {
    /// GeoJSON/STAC object type, normally `FeatureCollection`.
    #[serde(rename = "type")]
    pub collection_type: String,
    /// Search result items.
    #[serde(default)]
    pub features: Vec<StacItem>,
    /// Pagination and relation links.
    #[serde(default)]
    pub links: Vec<StacLink>,
}

/// One normalized item with all endpoints that returned it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FederatedStacItem {
    /// Stable collection-qualified deduplication key.
    pub key: String,
    /// Endpoint identifiers that returned this item.
    pub source_endpoints: Vec<String>,
    /// Original STAC Item payload.
    pub item: StacItem,
}

/// Per-endpoint outcome retained even when another endpoint fails.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointSearchOutcome {
    /// Endpoint identifier.
    pub endpoint_id: String,
    /// Exact configured endpoint URL or fixture path.
    pub endpoint_url: String,
    /// Number of items accepted before federated deduplication.
    pub matched_items: usize,
    /// Error text for a failed endpoint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Complete federated search response with source attribution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FederatedSearchResult {
    /// Original search request.
    pub request: StacSearchRequest,
    /// Per-endpoint outcomes in registration order.
    pub endpoints: Vec<EndpointSearchOutcome>,
    /// Normalized and deduplicated items.
    pub items: Vec<FederatedStacItem>,
}

/// Requirements used by the planner to rank assets without hiding trade-offs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetRequirements {
    /// Preferred media types in descending priority order.
    pub media_types: Vec<String>,
    /// Search area whose coverage must intersect the candidate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<[f64; 4]>,
    /// Require explicit CRS, units, and license metadata.
    #[serde(default = "default_true")]
    pub require_metadata: bool,
}

impl Default for AssetRequirements {
    fn default() -> Self {
        Self {
            media_types: vec!["application/vnd.apache.parquet".into()],
            bbox: None,
            require_metadata: true,
        }
    }
}

/// One auditable verification performed before an asset is eligible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetVerification {
    pub check: String,
    pub passed: bool,
    pub evidence: String,
}

/// A flattened item/asset candidate with deterministic score and explanation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetCandidate {
    pub stac_item_key: String,
    pub asset_key: String,
    pub href: String,
    pub media_type: String,
    pub source_endpoints: Vec<String>,
    pub score: i32,
    pub compatible: bool,
    pub verifications: Vec<AssetVerification>,
}

/// Selected STAC asset and the evidence needed to reproduce the decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetBindingReceipt {
    pub selected: AssetCandidate,
    pub candidates: Vec<AssetCandidate>,
    pub selection_reason: String,
    pub source_urls: Vec<String>,
    pub stac_item_id: String,
    pub stac_collection: Option<String>,
    pub crs: String,
    pub units: String,
    pub license: String,
    pub retrieved_at: DateTime<Utc>,
}

impl FederatedSearchResult {
    /// Number of endpoints that returned a valid ItemCollection.
    pub fn successful_endpoints(&self) -> usize {
        self.endpoints
            .iter()
            .filter(|outcome| outcome.error.is_none())
            .count()
    }

    /// Number of endpoints isolated as failures.
    pub fn failed_endpoints(&self) -> usize {
        self.endpoints.len() - self.successful_endpoints()
    }

    /// Compare every data asset, reject unverifiable candidates, and bind the best one.
    pub fn compare_and_bind(
        &self,
        requirements: &AssetRequirements,
    ) -> Result<AssetBindingReceipt, CatalogError> {
        let mut candidates = Vec::new();
        for federated in &self.items {
            for (asset_key, asset) in &federated.item.assets {
                let metadata = item_metadata(&federated.item);
                let mut verifications = vec![
                    verification(
                        "media_type",
                        requirements.media_types.contains(&asset.media_type),
                        &asset.media_type,
                    ),
                    verification(
                        "data_role",
                        asset.roles.iter().any(|role| role == "data"),
                        &format!("{:?}", asset.roles),
                    ),
                    verification(
                        "source_coverage",
                        requirements
                            .bbox
                            .map_or(true, |bbox| bbox_intersects(federated.item.bbox, bbox)),
                        &format!("{:?}", federated.item.bbox),
                    ),
                ];
                if requirements.require_metadata {
                    verifications.extend([
                        verification("crs", metadata.crs != "unknown", &metadata.crs),
                        verification("units", metadata.units != "unknown", &metadata.units),
                        verification("license", metadata.license != "unknown", &metadata.license),
                    ]);
                }
                let compatible = verifications.iter().all(|check| check.passed);
                let media_rank = requirements
                    .media_types
                    .iter()
                    .position(|media_type| media_type == &asset.media_type)
                    .map_or(0, |index| 100 - index as i32 * 10);
                let score = media_rank
                    + if asset.roles.iter().any(|role| role == "data") {
                        20
                    } else {
                        0
                    }
                    + federated.source_endpoints.len() as i32 * 5
                    + if compatible { 1_000 } else { 0 };
                candidates.push(AssetCandidate {
                    stac_item_key: federated.key.clone(),
                    asset_key: asset_key.clone(),
                    href: asset.href.clone(),
                    media_type: asset.media_type.clone(),
                    source_endpoints: federated.source_endpoints.clone(),
                    score,
                    compatible,
                    verifications,
                });
            }
        }
        candidates.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.stac_item_key.cmp(&right.stac_item_key))
                .then_with(|| left.asset_key.cmp(&right.asset_key))
        });
        let selected = candidates
            .iter()
            .find(|candidate| candidate.compatible)
            .cloned()
            .ok_or_else(|| {
                CatalogError::InvalidStac("no compatible, verified asset candidate".into())
            })?;
        let item = self
            .items
            .iter()
            .find(|item| item.key == selected.stac_item_key)
            .expect("selected candidate retains its item");
        let metadata = item_metadata(&item.item);
        let source_urls = self
            .endpoints
            .iter()
            .filter(|endpoint| selected.source_endpoints.contains(&endpoint.endpoint_id))
            .map(|endpoint| endpoint.endpoint_url.clone())
            .collect();
        let selection_reason = format!(
            "Selected {}/{}: verified all {} checks; score {} (preferred media type, data role, {} source endpoint(s)).",
            selected.stac_item_key,
            selected.asset_key,
            selected.verifications.len(),
            selected.score,
            selected.source_endpoints.len()
        );
        Ok(AssetBindingReceipt {
            selected,
            candidates,
            selection_reason,
            source_urls,
            stac_item_id: item.item.id.clone(),
            stac_collection: item.item.collection.clone(),
            crs: metadata.crs,
            units: metadata.units,
            license: metadata.license,
            retrieved_at: Utc::now(),
        })
    }
}

fn default_true() -> bool {
    true
}

fn verification(check: &str, passed: bool, evidence: &str) -> AssetVerification {
    AssetVerification {
        check: check.into(),
        passed,
        evidence: evidence.into(),
    }
}

struct ItemMetadata {
    crs: String,
    units: String,
    license: String,
}

fn item_metadata(item: &StacItem) -> ItemMetadata {
    let text = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| item.properties.get(key).and_then(serde_json::Value::as_str))
            .unwrap_or("unknown")
            .to_string()
    };
    ItemMetadata {
        crs: text(&["proj:code", "genegis:crs"]),
        units: text(&["genegis:units", "units"]),
        license: text(&["license", "genegis:license"]),
    }
}

/// In-memory registry and federated STAC search coordinator.
#[derive(Debug, Clone, Default)]
pub struct FederatedCatalog {
    endpoints: Vec<StacEndpoint>,
}

impl FederatedCatalog {
    /// Create an empty endpoint registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register or replace an endpoint by stable identifier.
    pub fn register(&mut self, endpoint: StacEndpoint) {
        self.endpoints.retain(|existing| existing.id != endpoint.id);
        self.endpoints.push(endpoint);
    }

    /// List endpoints in registration order.
    pub fn endpoints(&self) -> &[StacEndpoint] {
        &self.endpoints
    }

    /// Search every endpoint, isolate failures, and merge duplicate items.
    pub fn search(&self, request: &StacSearchRequest) -> FederatedSearchResult {
        self.search_with_policy(request, &genegis_storage::RemoteAccessPolicy::default())
    }

    /// Search every endpoint under one explicit remote access policy.
    pub fn search_with_policy(
        &self,
        request: &StacSearchRequest,
        policy: &genegis_storage::RemoteAccessPolicy,
    ) -> FederatedSearchResult {
        let mut endpoints = Vec::with_capacity(self.endpoints.len());
        let mut items_by_key: BTreeMap<String, FederatedStacItem> = BTreeMap::new();

        for endpoint in &self.endpoints {
            match search_endpoint(endpoint, request, policy) {
                Ok(items) => {
                    let matched_items = items.len();
                    for item in items {
                        let key = item_key(&item);
                        if let Some(existing) = items_by_key.get_mut(&key) {
                            existing.source_endpoints.push(endpoint.id.clone());
                        } else {
                            items_by_key.insert(
                                key.clone(),
                                FederatedStacItem {
                                    key,
                                    source_endpoints: vec![endpoint.id.clone()],
                                    item,
                                },
                            );
                        }
                    }
                    endpoints.push(EndpointSearchOutcome {
                        endpoint_id: endpoint.id.clone(),
                        endpoint_url: endpoint.url.clone(),
                        matched_items,
                        error: None,
                    });
                }
                Err(error) => endpoints.push(EndpointSearchOutcome {
                    endpoint_id: endpoint.id.clone(),
                    endpoint_url: endpoint.url.clone(),
                    matched_items: 0,
                    error: Some(error.to_string()),
                }),
            }
        }

        let mut items: Vec<_> = items_by_key.into_values().collect();
        if let Some(limit) = request.limit {
            items.truncate(limit);
        }

        FederatedSearchResult {
            request: request.clone(),
            endpoints,
            items,
        }
    }
}

fn search_endpoint(
    endpoint: &StacEndpoint,
    request: &StacSearchRequest,
    policy: &genegis_storage::RemoteAccessPolicy,
) -> Result<Vec<StacItem>, CatalogError> {
    let bytes = if endpoint.url.starts_with("http://") || endpoint.url.starts_with("https://") {
        let url = stac_search_url(&endpoint.url);
        let body = serde_json::to_vec(request)
            .map_err(|error| CatalogError::InvalidStac(format!("search request: {error}")))?;
        let headers = authentication_headers(&endpoint.authentication)?;
        genegis_storage::post_http_json_bytes_with_policy(&url, &body, &headers, policy)
            .map_err(|error| CatalogError::Remote(error.to_string()))?
            .bytes
    } else {
        let path = resolve_catalog_url(&endpoint.url);
        fetch_json_bytes(&path)?
    };

    let collection: StacItemCollection = serde_json::from_slice(&bytes)
        .map_err(|error| CatalogError::InvalidStac(format!("item collection: {error}")))?;

    if collection.collection_type != "FeatureCollection" {
        return Err(CatalogError::InvalidStac(format!(
            "expected FeatureCollection, got {}",
            collection.collection_type
        )));
    }

    Ok(collection
        .features
        .into_iter()
        .filter(|item| matches_request(item, request))
        .collect())
}

fn authentication_headers(
    authentication: &StacAuthentication,
) -> Result<Vec<(String, String)>, CatalogError> {
    match authentication {
        StacAuthentication::Anonymous => Ok(Vec::new()),
        StacAuthentication::BearerEnv { env_var } => {
            let value = std::env::var(env_var).map_err(|_| {
                CatalogError::Remote(format!(
                    "authentication environment variable {env_var:?} is not set"
                ))
            })?;
            Ok(vec![("Authorization".into(), format!("Bearer {value}"))])
        }
        StacAuthentication::HeaderEnv { header, env_var } => {
            let value = std::env::var(env_var).map_err(|_| {
                CatalogError::Remote(format!(
                    "authentication environment variable {env_var:?} is not set"
                ))
            })?;
            Ok(vec![(header.clone(), value)])
        }
    }
}

fn stac_search_url(url: &str) -> String {
    let url = url.trim_end_matches('/');
    if url.ends_with("/search") {
        url.to_string()
    } else {
        format!("{url}/search")
    }
}

fn matches_request(item: &StacItem, request: &StacSearchRequest) -> bool {
    if let Some(query_bbox) = request.bbox {
        if !bbox_intersects(item.bbox, query_bbox) {
            return false;
        }
    }

    if !request.collections.is_empty()
        && !item
            .collection
            .as_ref()
            .is_some_and(|collection| request.collections.contains(collection))
    {
        return false;
    }

    true
}

fn bbox_intersects(left: [f64; 4], right: [f64; 4]) -> bool {
    left[0] <= right[2] && left[2] >= right[0] && left[1] <= right[3] && left[3] >= right[1]
}

fn item_key(item: &StacItem) -> String {
    match &item.collection {
        Some(collection) => format!("{collection}/{}", item.id),
        None => item.id.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpListener};
    use std::thread;

    use super::*;

    fn fixture_endpoint(id: &str) -> StacEndpoint {
        StacEndpoint::new(id, "examples/stac/sample-search.json")
    }

    fn spawn_stac_search_fixture() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let response_body = include_bytes!("../../../examples/stac/sample-search.json").to_vec();

        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let read = stream.read(&mut buffer).expect("read");
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                let Some(header_end) = request
                    .windows(4)
                    .position(|window| window == b"\r\n\r\n")
                    .map(|position| position + 4)
                else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| line.split_once(':'))
                    .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if request.len() >= header_end + content_length {
                    break;
                }
            }

            let request_text = String::from_utf8_lossy(&request);
            let status = if request_text.starts_with("POST /search ") {
                "200 OK"
            } else {
                "405 Method Not Allowed"
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/geo+json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response_body.len()
            );
            stream.write_all(response.as_bytes()).expect("headers");
            stream.write_all(&response_body).expect("body");
            stream.flush().expect("flush");
            let _ = stream.shutdown(Shutdown::Write);
        });

        format!("http://{address}")
    }

    #[test]
    fn searches_local_item_collection_and_deduplicates_sources() {
        let mut catalog = FederatedCatalog::new();
        catalog.register(fixture_endpoint("primary"));
        catalog.register(fixture_endpoint("mirror"));

        let result = catalog.search(&StacSearchRequest {
            bbox: Some([136.79, 35.03, 137.07, 35.27]),
            limit: Some(10),
            ..StacSearchRequest::default()
        });

        assert_eq!(result.successful_endpoints(), 2);
        assert_eq!(result.failed_endpoints(), 0);
        assert_eq!(result.items.len(), 1);
        assert_eq!(
            result.items[0].source_endpoints,
            vec!["primary".to_string(), "mirror".to_string()]
        );
    }

    #[test]
    fn applies_bbox_filter_to_local_fixture() {
        let mut catalog = FederatedCatalog::new();
        catalog.register(fixture_endpoint("local"));

        let result = catalog.search(&StacSearchRequest {
            bbox: Some([0.0, 0.0, 1.0, 1.0]),
            ..StacSearchRequest::default()
        });

        assert!(result.items.is_empty());
        assert_eq!(result.endpoints[0].matched_items, 0);
    }

    #[test]
    fn isolates_endpoint_failure() {
        let mut catalog = FederatedCatalog::new();
        catalog.register(fixture_endpoint("available"));
        catalog.register(StacEndpoint::new(
            "missing",
            "examples/stac/does-not-exist.json",
        ));

        let result = catalog.search(&StacSearchRequest::default());

        assert_eq!(result.successful_endpoints(), 1);
        assert_eq!(result.failed_endpoints(), 1);
        assert_eq!(result.items.len(), 1);
        assert!(result.endpoints[1].error.is_some());
    }

    #[test]
    fn searches_http_stac_api_with_post() {
        let mut catalog = FederatedCatalog::new();
        catalog.register(StacEndpoint::new("http", spawn_stac_search_fixture()));

        let result = catalog.search(&StacSearchRequest {
            bbox: Some([136.79, 35.03, 137.07, 35.27]),
            limit: Some(10),
            ..StacSearchRequest::default()
        });

        assert_eq!(result.successful_endpoints(), 1);
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].source_endpoints, vec!["http".to_string()]);
    }

    #[test]
    fn compares_candidates_and_explains_verified_binding() {
        let mut catalog = FederatedCatalog::new();
        catalog.register(fixture_endpoint("primary"));
        catalog.register(fixture_endpoint("mirror"));
        let result = catalog.search(&StacSearchRequest {
            bbox: Some([136.79, 35.03, 137.07, 35.27]),
            ..StacSearchRequest::default()
        });

        let receipt = result
            .compare_and_bind(&AssetRequirements {
                bbox: result.request.bbox,
                ..AssetRequirements::default()
            })
            .expect("verified GeoParquet binding");

        assert_eq!(receipt.selected.asset_key, "geoparquet");
        assert_eq!(receipt.crs, "EPSG:4326");
        assert_eq!(receipt.units, "degrees");
        assert_eq!(receipt.license, "CC-BY-4.0");
        assert_eq!(receipt.source_urls.len(), 2);
        assert!(receipt
            .selected
            .verifications
            .iter()
            .all(|check| check.passed));
        assert!(receipt.selection_reason.contains("2 source endpoint(s)"));
        assert!(receipt
            .candidates
            .iter()
            .any(|candidate| !candidate.compatible));
    }

    #[test]
    fn rejects_remote_endpoint_outside_explicit_allowlist() {
        let mut catalog = FederatedCatalog::new();
        catalog.register(StacEndpoint::new("blocked", "https://blocked.invalid/stac"));
        let policy = genegis_storage::RemoteAccessPolicy {
            allowed_hosts: vec![],
            allow_loopback: false,
            ..genegis_storage::RemoteAccessPolicy::from_env()
        };

        let result = catalog.search_with_policy(&StacSearchRequest::default(), &policy);

        assert_eq!(result.failed_endpoints(), 1);
        assert!(result.endpoints[0]
            .error
            .as_deref()
            .is_some_and(|error| error.contains("not allowlisted")));
    }
}
