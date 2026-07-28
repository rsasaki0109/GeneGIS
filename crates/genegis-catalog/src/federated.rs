use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::CatalogError;
use crate::external_stac::{fetch_json_bytes, resolve_catalog_url};
use crate::stac::{StacItem, StacLink};

/// A named STAC API search endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StacEndpoint {
    /// Stable identifier used in provenance and result summaries.
    pub id: String,
    /// Human-readable endpoint name.
    pub title: String,
    /// STAC API root, explicit `/search` URL, or local ItemCollection path.
    pub url: String,
}

impl StacEndpoint {
    /// Create an endpoint whose title defaults to its identifier.
    pub fn new(id: impl Into<String>, url: impl Into<String>) -> Self {
        let id = id.into();
        Self {
            title: id.clone(),
            id,
            url: url.into(),
        }
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
        let mut endpoints = Vec::with_capacity(self.endpoints.len());
        let mut items_by_key: BTreeMap<String, FederatedStacItem> = BTreeMap::new();

        for endpoint in &self.endpoints {
            match search_endpoint(endpoint, request) {
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
) -> Result<Vec<StacItem>, CatalogError> {
    let bytes = if endpoint.url.starts_with("http://") || endpoint.url.starts_with("https://") {
        let url = stac_search_url(&endpoint.url);
        let body = serde_json::to_vec(request)
            .map_err(|error| CatalogError::InvalidStac(format!("search request: {error}")))?;
        genegis_storage::post_http_json_bytes(&url, &body)
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
    use std::net::TcpListener;
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
}
