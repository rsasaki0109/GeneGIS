//! Resolve arbitrary place names to boundary or point layers.
//!
//! Providers are explicit and recorded in provenance:
//!
//! * `nominatim` — OpenStreetMap Nominatim search with polygon output
//!   (ODbL, "© OpenStreetMap contributors"), for administrative areas and
//!   named features worldwide;
//! * `gsi` — 国土地理院 地名・住所検索 API, point results for Japanese
//!   addresses and place names.
//!
//! Resolution runs through Command + Workflow Graph; the response bytes are
//! hashed into the source snapshot so the layer digest is bound to exactly
//! what the provider returned. Ambiguous results are reported, not hidden.

use std::collections::BTreeMap;
use std::sync::Mutex;

use genegis_core::{
    Command, CommandBus, CommandEnvelope, CommandOrigin, InputSnapshot, Project, WorkflowDigest,
    WorkflowExecution, WorkflowExecutionContext, WorkflowExecutionError, WorkflowExecutionEvent,
    WorkflowExecutor,
};
use genegis_crs::{ChecksumVerification, SourceSnapshot};
use genegis_workflow::{GeoWorkflow, WorkflowDataRef, WorkflowInputContract, WorkflowStep};
use geo_types::{Geometry, Point};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{Result, ToolkitError};
use crate::geojson_io::geometry_from_json;
use crate::layer::{sha256_bytes, CrsStatus, Feature, Layer, LayerSummary};

/// Place provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaceProvider {
    /// OpenStreetMap Nominatim (boundaries).
    #[default]
    Nominatim,
    /// 国土地理院 地名・住所検索 (points, Japan).
    Gsi,
}

impl PlaceProvider {
    fn endpoint(self) -> &'static str {
        match self {
            Self::Nominatim => "https://nominatim.openstreetmap.org/search",
            Self::Gsi => "https://msearch.gsi.go.jp/address-search/AddressSearch",
        }
    }

    fn license(self) -> &'static str {
        match self {
            Self::Nominatim => "ODbL-1.0",
            Self::Gsi => "国土地理院コンテンツ利用規約",
        }
    }

    fn attribution(self) -> &'static str {
        match self {
            Self::Nominatim => "© OpenStreetMap contributors",
            Self::Gsi => "国土地理院 地名・住所検索API",
        }
    }
}

/// Place resolution request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaceRequest {
    /// Place name, e.g. 札幌市 or 名古屋市中区.
    pub query: String,
    /// Provider.
    #[serde(default)]
    pub provider: PlaceProvider,
    /// Candidate to pick (0 = best); see the receipt for alternatives.
    #[serde(default)]
    pub candidate: usize,
}

/// One candidate returned by a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaceCandidate {
    /// Display name.
    pub name: String,
    /// Feature class/type (e.g. boundary/administrative).
    pub kind: String,
    /// Whether a polygon boundary is available.
    pub has_boundary: bool,
    /// `[lon, lat]`.
    pub center: [f64; 2],
}

/// Receipt of a place resolution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaceReceipt {
    /// Applied command ID.
    pub command_id: String,
    /// Workflow digest.
    pub workflow_digest: String,
    /// Result digest (layer digest).
    pub result_digest: String,
    /// Request URL.
    pub source_uri: String,
    /// `sha256:` of the provider response bytes.
    pub response_sha256: String,
    /// All candidates in provider order.
    pub candidates: Vec<PlaceCandidate>,
    /// Chosen candidate index.
    pub chosen: usize,
    /// Resulting layer.
    pub layer: LayerSummary,
}

/// HTTP fetcher (injectable for tests and air-gapped deployments).
pub trait Fetcher {
    /// GET a URL and return the body bytes.
    fn get(&self, url: &str) -> Result<Vec<u8>>;
}

/// Default fetcher using `ureq` with an identifying User-Agent, as required
/// by the Nominatim usage policy.
pub struct HttpFetcher;

impl Fetcher for HttpFetcher {
    fn get(&self, url: &str) -> Result<Vec<u8>> {
        let mut response = ureq::get(url)
            .header(
                "User-Agent",
                "GeneGIS/0.1 (+https://github.com/rsasaki0109/GeneGIS)",
            )
            .header("Accept-Language", "ja,en")
            .call()
            .map_err(|e| ToolkitError::Provider(format!("{url}: {e}")))?;
        response
            .body_mut()
            .with_config()
            .limit(64 * 1024 * 1024)
            .read_to_vec()
            .map_err(|e| ToolkitError::Provider(format!("{url}: {e}")))
    }
}

fn encode(query: &str) -> String {
    query
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Request URL for a query.
pub fn request_url(request: &PlaceRequest) -> String {
    match request.provider {
        PlaceProvider::Nominatim => format!(
            "{}?q={}&format=jsonv2&polygon_geojson=1&polygon_threshold=0.0001&limit=5&accept-language=ja",
            request.provider.endpoint(),
            encode(request.query.trim())
        ),
        PlaceProvider::Gsi => format!("{}?q={}", request.provider.endpoint(), encode(request.query.trim())),
    }
}

/// Parse a provider response into candidates and features (one per candidate).
pub fn parse_response(
    provider: PlaceProvider,
    query: &str,
    body: &[u8],
) -> Result<Vec<(PlaceCandidate, Feature)>> {
    let value: Value = serde_json::from_slice(body)
        .map_err(|e| ToolkitError::Provider(format!("invalid JSON from provider: {e}")))?;
    let items = value
        .as_array()
        .ok_or_else(|| ToolkitError::Provider("provider response is not a list".into()))?;
    let mut out = Vec::new();
    for (index, item) in items.iter().enumerate() {
        match provider {
            PlaceProvider::Nominatim => {
                let lon = item
                    .get("lon")
                    .and_then(|v| v.as_str()?.parse::<f64>().ok());
                let lat = item
                    .get("lat")
                    .and_then(|v| v.as_str()?.parse::<f64>().ok());
                let (Some(lon), Some(lat)) = (lon, lat) else {
                    continue;
                };
                let name = item
                    .get("display_name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let kind = format!(
                    "{}/{}",
                    item.get("category")
                        .or_else(|| item.get("class"))
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                    item.get("type").and_then(Value::as_str).unwrap_or("")
                );
                let geometry = item
                    .get("geojson")
                    .and_then(|g| geometry_from_json(g).ok())
                    .unwrap_or(Geometry::Point(Point::new(lon, lat)));
                let has_boundary =
                    matches!(geometry, Geometry::Polygon(_) | Geometry::MultiPolygon(_));
                let mut properties = BTreeMap::new();
                properties.insert(
                    "name".to_string(),
                    Value::from(item.get("name").and_then(Value::as_str).unwrap_or(&name)),
                );
                properties.insert("display_name".to_string(), Value::from(name.clone()));
                properties.insert("kind".to_string(), Value::from(kind.clone()));
                if let Some(osm) = item.get("osm_type").and_then(Value::as_str) {
                    let id = item
                        .get("osm_id")
                        .and_then(Value::as_i64)
                        .unwrap_or_default();
                    properties.insert("osm_ref".to_string(), Value::from(format!("{osm}/{id}")));
                }
                if let Some(rank) = item.get("place_rank").and_then(Value::as_i64) {
                    properties.insert("place_rank".to_string(), Value::from(rank));
                }
                out.push((
                    PlaceCandidate {
                        name,
                        kind,
                        has_boundary,
                        center: [lon, lat],
                    },
                    Feature {
                        id: index as u64,
                        geometry: Some(geometry),
                        properties,
                    },
                ));
            }
            PlaceProvider::Gsi => {
                let coordinates = item
                    .pointer("/geometry/coordinates")
                    .and_then(Value::as_array);
                let (Some(lon), Some(lat)) = (
                    coordinates.and_then(|c| c.first()).and_then(Value::as_f64),
                    coordinates.and_then(|c| c.get(1)).and_then(Value::as_f64),
                ) else {
                    continue;
                };
                let name = item
                    .pointer("/properties/title")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let mut properties = BTreeMap::new();
                properties.insert("name".to_string(), Value::from(name.clone()));
                if let Some(code) = item
                    .pointer("/properties/addressCode")
                    .and_then(Value::as_str)
                {
                    properties.insert("address_code".to_string(), Value::from(code));
                }
                out.push((
                    PlaceCandidate {
                        name,
                        kind: "address".into(),
                        has_boundary: false,
                        center: [lon, lat],
                    },
                    Feature {
                        id: index as u64,
                        geometry: Some(Geometry::Point(Point::new(lon, lat))),
                        properties,
                    },
                ));
            }
        }
    }
    match provider {
        // Prefer candidates with a real boundary, keeping provider order otherwise.
        PlaceProvider::Nominatim => out.sort_by_key(|(c, _)| !c.has_boundary),
        // The GSI service orders by address hierarchy, so 「名古屋駅」 first
        // returns 千葉県成田市名古屋. Rank exact titles, then titles that contain
        // the whole query (shortest first), then the provider order.
        PlaceProvider::Gsi => {
            let query = query.trim();
            out.sort_by_key(|(c, _)| {
                if c.name == query {
                    (0, 0)
                } else if c.name.contains(query) {
                    (1, c.name.chars().count())
                } else {
                    (2, 0)
                }
            });
        }
    }
    Ok(out)
}

struct PlaceExecutor<'a> {
    request: &'a PlaceRequest,
    body: &'a [u8],
    url: &'a str,
    result: Mutex<Option<(Layer, Vec<PlaceCandidate>)>>,
}

impl WorkflowExecutor for PlaceExecutor<'_> {
    fn execute(
        &self,
        _workflow: &GeoWorkflow,
        context: &WorkflowExecutionContext,
    ) -> std::result::Result<WorkflowExecution, WorkflowExecutionError> {
        let fail = |m: String| WorkflowExecutionError::Failed(m);
        let observed = sha256_bytes(self.body);
        let authorized = context
            .input_snapshots
            .first()
            .and_then(|s| s.source.checksum.clone());
        if authorized.as_deref() != Some(observed.as_str()) {
            return Err(fail(
                "provider response does not match the authorized digest".into(),
            ));
        }
        let candidates = parse_response(self.request.provider, &self.request.query, self.body)
            .map_err(|e| fail(e.to_string()))?;
        if candidates.is_empty() {
            return Err(fail(format!(
                "no place named {:?} was found",
                self.request.query
            )));
        }
        let (candidate, feature) =
            candidates
                .get(self.request.candidate)
                .cloned()
                .ok_or_else(|| {
                    fail(format!(
                        "candidate {} does not exist ({} found)",
                        self.request.candidate,
                        candidates.len()
                    ))
                })?;
        let mut layer = Layer::new(
            candidate
                .name
                .split([',', '、'])
                .next()
                .unwrap_or(&candidate.name)
                .trim()
                .to_string(),
            "EPSG:4326",
            CrsStatus::Declared,
        );
        layer.features.push(Feature { id: 0, ..feature });
        layer.refresh_schema();
        layer.provenance.source_uri = self.url.to_string();
        layer.provenance.source_sha256 = Some(observed.clone());
        layer.provenance.format = format!(
            "place:{}",
            serde_json::to_value(self.request.provider)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default()
        );
        layer.provenance.license = Some(self.request.provider.license().into());
        layer.provenance.attribution = Some(self.request.provider.attribution().into());
        layer.provenance.retrieved_at = Some(context.command_timestamp.to_rfc3339());
        layer.provenance.workflow_digest = Some(context.workflow_digest.to_string());
        if candidates.len() > 1 {
            layer.provenance.notes.push(format!(
                "{} candidates matched {:?}; chose #{} ({})",
                candidates.len(),
                self.request.query,
                self.request.candidate,
                candidate.kind
            ));
        }
        if !candidate.has_boundary {
            layer
                .provenance
                .notes
                .push("provider returned a point, not a boundary".into());
        }
        let digest = layer.digest();
        let all: Vec<PlaceCandidate> = candidates.into_iter().map(|(c, _)| c).collect();
        let output = json!({"layer": layer.summary(), "candidates": all});
        *self
            .result
            .lock()
            .map_err(|_| fail("place lock poisoned".into()))? = Some((layer, all));
        Ok(WorkflowExecution {
            result_digest: digest,
            output,
            evidence: json!({"response_sha256": observed}),
            events: vec![WorkflowExecutionEvent {
                kind: "source_read".into(),
                source_uri: Some(self.url.to_string()),
                observed_at: context.command_timestamp,
                details: json!({"bytes": self.body.len()}),
            }],
        })
    }
}

/// Resolve a place through Command + Workflow Graph.
pub fn resolve_place(
    request: &PlaceRequest,
    fetcher: &dyn Fetcher,
) -> Result<(Layer, PlaceReceipt)> {
    if request.query.trim().is_empty() {
        return Err(ToolkitError::parameter("resolve_place", "query is empty"));
    }
    let url = request_url(request);
    let body = fetcher.get(&url)?;
    let digest = sha256_bytes(&body);
    let mut source = SourceSnapshot::new(url.clone());
    source.license = Some(request.provider.license().into());
    source.checksum = Some(digest.clone());
    source.expected_checksum = Some(digest.clone());
    source.observed_checksum = Some(digest.clone());
    source.checksum_status = ChecksumVerification::Verified;

    let mut workflow = GeoWorkflow::new(format!("「{}」の場所を解決する", request.query));
    workflow.add_input_contract(
        WorkflowInputContract::new("gazetteer_response")
            .with_value_unit("json")
            .with_source_snapshot(source.clone()),
    );
    let stages: [(&str, &str, Value); 3] = [
        (
            "query-gazetteer",
            "toolkit.QueryGazetteer",
            json!({"provider": request.provider, "query": request.query}),
        ),
        (
            "select-candidate",
            "toolkit.SelectPlaceCandidate",
            json!({"candidate": request.candidate, "prefer": "boundary"}),
        ),
        (
            "normalize-place",
            "toolkit.NormalizePlaceLayer",
            json!({"crs": "EPSG:4326", "license": request.provider.license()}),
        ),
    ];
    let mut previous: Option<&str> = None;
    for (id, operation, parameters) in stages {
        let mut step = WorkflowStep::named(id, operation, parameters)
            .with_outputs([WorkflowDataRef::output(id, "result")]);
        step = match previous {
            None => step.with_inputs([WorkflowDataRef::input("gazetteer_response")]),
            Some(prev) => step
                .with_dependencies([prev])
                .with_inputs([WorkflowDataRef::output(prev, "result")]),
        };
        workflow.push_step(step);
        previous = Some(id);
    }
    workflow.add_output_ref(WorkflowDataRef::output("normalize-place", "result"));
    let workflow_digest = WorkflowDigest::new(
        workflow
            .stable_digest()
            .map_err(|e| ToolkitError::Plan(e.to_string()))?,
    );
    let envelope = CommandEnvelope::new(
        CommandOrigin::Ui,
        Command::RunWorkflow {
            workflow_id: workflow.id,
        },
    )
    .with_workflow_digest(workflow_digest.clone())
    .with_source_snapshot(source.clone())
    .with_input_snapshot(InputSnapshot::new("gazetteer_response", source).with_value_unit("json"));
    let command_id = envelope.id;
    let executor = PlaceExecutor {
        request,
        body: &body,
        url: &url,
        result: Mutex::new(None),
    };
    let mut project = Project::new("GeneGIS place resolution");
    let mut bus = CommandBus::new(project.clone());
    bus.register_workflow(workflow)
        .map_err(|e| ToolkitError::Command(e.to_string()))?;
    let execution = bus
        .apply_with_executor(&mut project, envelope, &executor)
        .map_err(|e| ToolkitError::Provider(e.to_string()))?;
    let (layer, candidates) = executor
        .result
        .into_inner()
        .map_err(|_| ToolkitError::Command("place lock poisoned".into()))?
        .ok_or_else(|| ToolkitError::Command("place resolution produced no layer".into()))?;
    let receipt = PlaceReceipt {
        command_id: command_id.to_string(),
        workflow_digest: workflow_digest.to_string(),
        result_digest: execution.result_digest.unwrap_or_default(),
        source_uri: url,
        response_sha256: digest,
        candidates,
        chosen: request.candidate,
        layer: layer.summary(),
    };
    Ok((layer, receipt))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(&'static str);

    impl Fetcher for Fixed {
        fn get(&self, _url: &str) -> Result<Vec<u8>> {
            Ok(self.0.as_bytes().to_vec())
        }
    }

    const NOMINATIM: &str = r#"[
      {"place_id":1,"osm_type":"node","osm_id":42,"lat":"43.0618","lon":"141.3545","category":"place","type":"city","place_rank":16,"name":"札幌市","display_name":"札幌市, 北海道, 日本"},
      {"place_id":2,"osm_type":"relation","osm_id":3795658,"lat":"43.0621","lon":"141.3544","category":"boundary","type":"administrative","place_rank":12,"name":"札幌市","display_name":"札幌市, 石狩振興局, 北海道, 日本",
       "geojson":{"type":"Polygon","coordinates":[[[141.0,42.8],[141.6,42.8],[141.6,43.2],[141.0,43.2],[141.0,42.8]]]}}
    ]"#;

    #[test]
    fn prefers_boundaries_and_records_provenance() {
        let request = PlaceRequest {
            query: "札幌市".into(),
            provider: PlaceProvider::Nominatim,
            candidate: 0,
        };
        let (layer, receipt) = resolve_place(&request, &Fixed(NOMINATIM)).unwrap();
        assert_eq!(layer.geometry_kind(), crate::GeometryKind::Polygon);
        assert_eq!(layer.name, "札幌市");
        assert_eq!(layer.provenance.license.as_deref(), Some("ODbL-1.0"));
        assert_eq!(
            layer.provenance.attribution.as_deref(),
            Some("© OpenStreetMap contributors")
        );
        assert_eq!(receipt.candidates.len(), 2);
        assert!(receipt.candidates[0].has_boundary);
        assert_eq!(receipt.result_digest, layer.digest());
        assert!(receipt.source_uri.contains("q=%E6%9C%AD%E5%B9%8C%E5%B8%82"));
    }

    #[test]
    fn gsi_points_and_empty_results() {
        let body = r#"[
          {"geometry":{"coordinates":[140.36,35.85],"type":"Point"},"type":"Feature","properties":{"addressCode":"12211","title":"千葉県成田市名古屋"}},
          {"geometry":{"coordinates":[136.8836,35.1707],"type":"Point"},"type":"Feature","properties":{"addressCode":"","title":"名鉄名古屋駅"}},
          {"geometry":{"coordinates":[136.881537,35.170915],"type":"Point"},"type":"Feature","properties":{"addressCode":"","title":"名古屋駅"}}
        ]"#;
        let request = PlaceRequest {
            query: "名古屋駅".into(),
            provider: PlaceProvider::Gsi,
            candidate: 0,
        };
        let (layer, _) = resolve_place(&request, &Fixed(body)).unwrap();
        assert_eq!(layer.geometry_kind(), crate::GeometryKind::Point);
        assert_eq!(
            layer.name, "名古屋駅",
            "exact title must outrank address-order results"
        );
        assert!(layer.provenance.notes.iter().any(|n| n.contains("point")));
        let error = resolve_place(&request, &Fixed("[]")).unwrap_err();
        assert!(error.to_string().contains("no place"), "{error}");
    }
}
