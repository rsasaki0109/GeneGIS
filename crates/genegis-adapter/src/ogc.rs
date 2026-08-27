//! OGC WMS/WFS read adapter with exact capability admission and I/O evidence.

use std::{collections::BTreeSet, time::Instant};

use genegis_contract::GeoContract;
use genegis_crs::{ChecksumVerification, CoordinateUnit, Crs, SourceSnapshot};
use genegis_storage::{
    fetch_http_bytes_with_policy, CloudFormat, IoReceipt, IoRequestEvidence, IoSelection,
    RemoteAccessPolicy,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    admit, AdapterInvocation, AdapterManifest, AdapterOperation, AdmissionReport, BackendFamily,
    BackendIdentity, Capability, CapabilityPolicy, Determinism, EvidenceHook,
    ADAPTER_MANIFEST_SCHEMA_VERSION,
};

/// Digest of the reviewed native OGC HTTP adapter implementation contract.
pub const OGC_ADAPTER_BUILD_DIGEST: &str =
    "sha256:9eb2904df21b2a82a501019de18597af27687638b6deafbbc63c399d28a24343";

const ADAPTER_ID: &str = "org.genegis.ogc-web-service";
const ADAPTER_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Admitted OGC operation represented by a receipt.
pub enum OgcOperation {
    /// Discover WMS or WFS service metadata.
    GetCapabilities,
    /// Request one WMS map portrayal.
    WmsGetMap,
    /// Select WFS vector features.
    WfsGetFeature,
}

impl OgcOperation {
    fn operation_id(self) -> &'static str {
        match self {
            Self::GetCapabilities => "ogc.get_capabilities",
            Self::WmsGetMap => "ogc.wms.get_map",
            Self::WfsGetFeature => "ogc.wfs.get_feature",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Typed WMS 1.3.0 GetMap parameters.
pub struct WmsGetMapRequest {
    /// Allowlisted service endpoint without required query parameters.
    pub endpoint: String,
    /// Protocol version; currently `1.3.0`.
    pub version: String,
    /// Layer names in portrayal order.
    pub layers: Vec<String>,
    /// Optional style names aligned with layers.
    pub styles: Vec<String>,
    /// Requested output CRS.
    pub crs: Crs,
    /// Requested extent in the declared CRS.
    pub bbox: [f64; 4],
    /// Output width in pixels.
    pub width: u32,
    /// Output height in pixels.
    pub height: u32,
    /// Output image media type.
    pub format: String,
    /// Whether the server should render a transparent background.
    pub transparent: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Typed WFS 2.0.0 GetFeature parameters.
pub struct WfsGetFeatureRequest {
    /// Allowlisted service endpoint without required query parameters.
    pub endpoint: String,
    /// Protocol version; currently `2.0.0`.
    pub version: String,
    /// Feature type names selected from capabilities.
    pub type_names: Vec<String>,
    /// Requested output CRS.
    pub crs: Crs,
    /// Optional spatial extent in the declared CRS.
    pub bbox: Option<[f64; 4]>,
    /// Optional positive feature limit.
    pub count: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
/// Closed set of requests understood by the OGC adapter.
pub enum OgcRequest {
    /// Discover capabilities for one service family.
    GetCapabilities {
        /// Allowlisted service endpoint.
        endpoint: String,
        /// `WMS` or `WFS`.
        service: String,
        /// Requested protocol version.
        version: String,
    },
    /// WMS map request.
    WmsGetMap(WmsGetMapRequest),
    /// WFS feature request.
    WfsGetFeature(WfsGetFeatureRequest),
}

impl OgcRequest {
    fn operation(&self) -> OgcOperation {
        match self {
            Self::GetCapabilities { .. } => OgcOperation::GetCapabilities,
            Self::WmsGetMap(_) => OgcOperation::WmsGetMap,
            Self::WfsGetFeature(_) => OgcOperation::WfsGetFeature,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Evidence envelope emitted for one admitted OGC request.
pub struct OgcServiceReceipt {
    /// Receipt schema version.
    pub schema_version: String,
    /// Digest of the exact adapter manifest admitted before transport.
    pub manifest_digest: String,
    /// Fail-closed capability admission result.
    pub admission: AdmissionReport,
    /// Semantic operation executed.
    pub operation: OgcOperation,
    /// Fully encoded URL after typed parameter construction.
    pub request_url: String,
    /// Response source snapshot with observed content digest.
    pub source: SourceSnapshot,
    /// Requested output CRS for spatial responses.
    pub crs: Option<Crs>,
    /// Coordinate unit derived from `crs`.
    pub coordinate_unit: Option<CoordinateUnit>,
    /// Validated response media type.
    pub content_type: String,
    /// SHA-256 identity of returned bytes.
    pub output_digest: String,
    /// Shared request, byte, selection, and timing evidence.
    pub io: IoReceipt,
}

#[derive(Debug, Clone, PartialEq)]
/// OGC response bytes paired with their verified receipt.
pub struct OgcResponse {
    /// Validated response bytes.
    pub bytes: Vec<u8>,
    /// Admission and I/O evidence for these exact bytes.
    pub receipt: OgcServiceReceipt,
}

#[derive(Debug, Error)]
/// Fail-closed OGC adapter error taxonomy.
pub enum OgcAdapterError {
    /// Manifest, capability, backend, or evidence-hook admission failed.
    #[error("OGC adapter admission failed: {0:?}")]
    Admission(Vec<String>),
    /// Typed request parameters are invalid or unsupported.
    #[error("invalid OGC request: {0}")]
    Request(String),
    /// Allowlist, timeout, HTTP status, or response-size transport failure.
    #[error("OGC transport failed: {0}")]
    Transport(String),
    /// Response media type or payload does not satisfy the operation contract.
    #[error("OGC response rejected: {0}")]
    Response(String),
    /// Receipt or manifest digest generation failed.
    #[error("OGC evidence serialization failed: {0}")]
    Evidence(String),
}

#[derive(Debug, Clone)]
/// Read-only WMS/WFS adapter admitted by manifest and host policies.
pub struct OgcServiceAdapter {
    manifest: AdapterManifest,
    capability_policy: CapabilityPolicy,
    remote_policy: RemoteAccessPolicy,
}

impl OgcServiceAdapter {
    /// Build the reviewed adapter with a caller-selected remote allowlist policy.
    pub fn new(remote_policy: RemoteAccessPolicy) -> Self {
        let manifest = ogc_web_service_manifest();
        Self {
            capability_policy: CapabilityPolicy::read_only_network(&manifest.adapter_id),
            manifest,
            remote_policy,
        }
    }

    /// Build with explicit contracts for policy and negative-fixture tests.
    pub fn with_contracts(
        manifest: AdapterManifest,
        capability_policy: CapabilityPolicy,
        remote_policy: RemoteAccessPolicy,
    ) -> Self {
        Self {
            manifest,
            capability_policy,
            remote_policy,
        }
    }

    /// Admit, execute, validate, hash, and receipt one OGC request.
    pub fn execute(&self, request: &OgcRequest) -> Result<OgcResponse, OgcAdapterError> {
        let operation = request.operation();
        let invocation = AdapterInvocation {
            adapter_id: self.manifest.adapter_id.clone(),
            adapter_version: self.manifest.adapter_version.clone(),
            operation_id: operation.operation_id().into(),
            operation_version: "1.0.0".into(),
            backend: self.manifest.backend.clone(),
            requested_capabilities: BTreeSet::from([Capability::NetworkRead]),
        };
        let admission = admit(&self.manifest, &invocation, &self.capability_policy);
        if !admission.admitted {
            return Err(OgcAdapterError::Admission(
                admission
                    .failures
                    .iter()
                    .map(|failure| failure.code.clone())
                    .collect(),
            ));
        }
        let prepared = prepare_request(request)?;
        let started = Instant::now();
        let fetched = fetch_http_bytes_with_policy(&prepared.url, &self.remote_policy)
            .map_err(|error| OgcAdapterError::Transport(error.to_string()))?;
        let elapsed_ns = started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
        if fetched.bytes.is_empty() {
            return Err(OgcAdapterError::Response("empty response body".into()));
        }
        let content_type = fetched
            .content_type
            .as_deref()
            .and_then(|value| value.split(';').next())
            .map(str::trim)
            .unwrap_or("")
            .to_ascii_lowercase();
        let decoded_items = validate_response(request, &content_type, &fetched.bytes)?;
        let output_digest = format!("sha256:{:x}", Sha256::digest(&fetched.bytes));
        let mut source = SourceSnapshot::new(prepared.url.clone());
        source.checksum = Some(output_digest.clone());
        source.observed_checksum = Some(output_digest.clone());
        source.checksum_status = ChecksumVerification::Verified;
        let response_bytes = fetched.bytes.len() as u64;
        let io = IoReceipt::new(
            prepared.format,
            output_digest.clone(),
            response_bytes,
            response_bytes,
            prepared.selection,
            vec![IoRequestEvidence {
                start: 0,
                end: response_bytes - 1,
                response_bytes,
                http_status: Some(fetched.status),
            }],
            false,
            decoded_items,
            elapsed_ns,
            0,
            None,
        );
        let receipt = OgcServiceReceipt {
            schema_version: "0.1.0".into(),
            manifest_digest: self
                .manifest
                .digest()
                .map_err(|error| OgcAdapterError::Evidence(error.to_string()))?,
            admission,
            operation,
            request_url: prepared.url,
            source,
            crs: prepared.crs.clone(),
            coordinate_unit: prepared.crs.map(|crs| crs.coordinate_unit()),
            content_type,
            output_digest,
            io,
        };
        Ok(OgcResponse {
            bytes: fetched.bytes,
            receipt,
        })
    }
}

struct PreparedRequest {
    url: String,
    format: CloudFormat,
    selection: IoSelection,
    crs: Option<Crs>,
}

fn prepare_request(request: &OgcRequest) -> Result<PreparedRequest, OgcAdapterError> {
    match request {
        OgcRequest::GetCapabilities {
            endpoint,
            service,
            version,
        } => {
            let service = service.to_ascii_uppercase();
            if !matches!(service.as_str(), "WMS" | "WFS") {
                return Err(OgcAdapterError::Request(
                    "service must be WMS or WFS".into(),
                ));
            }
            let url = request_url(
                endpoint,
                &[
                    ("SERVICE", service.as_str()),
                    ("REQUEST", "GetCapabilities"),
                    ("VERSION", version),
                ],
            )?;
            Ok(PreparedRequest {
                url,
                format: if service == "WMS" {
                    CloudFormat::Wms
                } else {
                    CloudFormat::Wfs
                },
                selection: IoSelection::OgcCapabilities {
                    service,
                    version: version.clone(),
                },
                crs: None,
            })
        }
        OgcRequest::WmsGetMap(map) => prepare_wms_map(map),
        OgcRequest::WfsGetFeature(feature) => prepare_wfs_feature(feature),
    }
}

fn prepare_wms_map(map: &WmsGetMapRequest) -> Result<PreparedRequest, OgcAdapterError> {
    if map.version != "1.3.0"
        || map.layers.is_empty()
        || map.width == 0
        || map.height == 0
        || !matches!(map.format.as_str(), "image/png" | "image/jpeg")
    {
        return Err(OgcAdapterError::Request(
            "WMS requires version 1.3.0, layers, nonzero dimensions, and PNG/JPEG".into(),
        ));
    }
    validate_bbox(&map.bbox)?;
    map.crs
        .require_known()
        .map_err(|error| OgcAdapterError::Request(error.to_string()))?;
    let bbox = bbox_strings(map.bbox);
    let width = map.width.to_string();
    let height = map.height.to_string();
    let transparent = if map.transparent { "TRUE" } else { "FALSE" };
    let layers = map.layers.join(",");
    let styles = map.styles.join(",");
    let crs = map.crs.identifier();
    let bbox_parameter = bbox.join(",");
    let url = request_url(
        &map.endpoint,
        &[
            ("SERVICE", "WMS"),
            ("REQUEST", "GetMap"),
            ("VERSION", &map.version),
            ("LAYERS", &layers),
            ("STYLES", &styles),
            ("CRS", &crs),
            ("BBOX", &bbox_parameter),
            ("WIDTH", &width),
            ("HEIGHT", &height),
            ("FORMAT", &map.format),
            ("TRANSPARENT", transparent),
        ],
    )?;
    Ok(PreparedRequest {
        url,
        format: CloudFormat::Wms,
        selection: IoSelection::WmsMap {
            layers: map.layers.clone(),
            crs: map.crs.identifier(),
            bbox,
            width: map.width,
            height: map.height,
            format: map.format.clone(),
        },
        crs: Some(map.crs.clone()),
    })
}

fn prepare_wfs_feature(feature: &WfsGetFeatureRequest) -> Result<PreparedRequest, OgcAdapterError> {
    if feature.version != "2.0.0" || feature.type_names.is_empty() || feature.count == Some(0) {
        return Err(OgcAdapterError::Request(
            "WFS requires version 2.0.0, type names, and a positive count".into(),
        ));
    }
    feature
        .crs
        .require_known()
        .map_err(|error| OgcAdapterError::Request(error.to_string()))?;
    if let Some(bbox) = &feature.bbox {
        validate_bbox(bbox)?;
    }
    let bbox = feature.bbox.map(bbox_strings);
    let count = feature.count.map(|value| value.to_string());
    let mut parameters = vec![
        ("SERVICE", "WFS".to_string()),
        ("REQUEST", "GetFeature".to_string()),
        ("VERSION", feature.version.clone()),
        ("TYPENAMES", feature.type_names.join(",")),
        ("SRSNAME", feature.crs.identifier()),
        ("OUTPUTFORMAT", "application/geo+json".into()),
    ];
    if let Some(bbox) = &bbox {
        parameters.push(("BBOX", bbox.join(",")));
    }
    if let Some(count) = &count {
        parameters.push(("COUNT", count.clone()));
    }
    let parameter_refs: Vec<_> = parameters
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect();
    let url = request_url(&feature.endpoint, &parameter_refs)?;
    Ok(PreparedRequest {
        url,
        format: CloudFormat::Wfs,
        selection: IoSelection::WfsFeatures {
            type_names: feature.type_names.clone(),
            crs: feature.crs.identifier(),
            bbox,
            count: feature.count,
        },
        crs: Some(feature.crs.clone()),
    })
}

fn request_url(endpoint: &str, parameters: &[(&str, &str)]) -> Result<String, OgcAdapterError> {
    let mut url =
        url::Url::parse(endpoint).map_err(|error| OgcAdapterError::Request(error.to_string()))?;
    {
        let mut query = url.query_pairs_mut();
        for (key, value) in parameters {
            query.append_pair(key, value);
        }
    }
    Ok(url.to_string())
}

fn validate_bbox(bbox: &[f64; 4]) -> Result<(), OgcAdapterError> {
    if !bbox.iter().all(|value| value.is_finite()) || bbox[0] >= bbox[2] || bbox[1] >= bbox[3] {
        return Err(OgcAdapterError::Request("invalid bbox".into()));
    }
    Ok(())
}

fn bbox_strings(bbox: [f64; 4]) -> [String; 4] {
    bbox.map(|value| value.to_string())
}

fn validate_response(
    request: &OgcRequest,
    content_type: &str,
    bytes: &[u8],
) -> Result<u64, OgcAdapterError> {
    match request {
        OgcRequest::GetCapabilities { service, .. } => {
            if !matches!(content_type, "application/xml" | "text/xml") {
                return Err(OgcAdapterError::Response(format!(
                    "capabilities content type is {content_type:?}"
                )));
            }
            let xml = std::str::from_utf8(bytes)
                .map_err(|error| OgcAdapterError::Response(error.to_string()))?;
            if xml.contains("ServiceException") || xml.contains("ExceptionReport") {
                return Err(OgcAdapterError::Response(
                    "service returned an exception document".into(),
                ));
            }
            let marker = if service.eq_ignore_ascii_case("WMS") {
                "WMS_Capabilities"
            } else {
                "WFS_Capabilities"
            };
            if !xml.contains(marker) {
                return Err(OgcAdapterError::Response(format!("missing {marker} root")));
            }
            Ok(1)
        }
        OgcRequest::WmsGetMap(map) => {
            if content_type != map.format {
                return Err(OgcAdapterError::Response(format!(
                    "expected {}, got {content_type:?}",
                    map.format
                )));
            }
            let valid = match map.format.as_str() {
                "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
                "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
                _ => false,
            };
            if !valid {
                return Err(OgcAdapterError::Response(
                    "map bytes do not match declared image type".into(),
                ));
            }
            Ok(map.width as u64 * map.height as u64)
        }
        OgcRequest::WfsGetFeature(_) => {
            if !matches!(content_type, "application/json" | "application/geo+json") {
                return Err(OgcAdapterError::Response(format!(
                    "feature content type is {content_type:?}"
                )));
            }
            let value: serde_json::Value = serde_json::from_slice(bytes)
                .map_err(|error| OgcAdapterError::Response(error.to_string()))?;
            if value.get("type").and_then(serde_json::Value::as_str) != Some("FeatureCollection") {
                return Err(OgcAdapterError::Response(
                    "WFS JSON is not a FeatureCollection".into(),
                ));
            }
            Ok(value
                .get("features")
                .and_then(serde_json::Value::as_array)
                .map_or(0, |features| features.len() as u64))
        }
    }
}

/// Return the reviewed native WMS/WFS adapter manifest.
pub fn ogc_web_service_manifest() -> AdapterManifest {
    let capabilities = BTreeSet::from([Capability::NetworkRead]);
    let evidence_hooks = BTreeSet::from([
        EvidenceHook::InputDigests,
        EvidenceHook::OutputDigests,
        EvidenceHook::Parameters,
        EvidenceHook::ComponentIdentity,
        EvidenceHook::EnvironmentDigest,
        EvidenceHook::IoMetrics,
    ]);
    let operation = |operation_id: &str| AdapterOperation {
        operation_id: operation_id.into(),
        operation_version: "1.0.0".into(),
        inputs: vec![GeoContract::new(format!("{operation_id}.request"))],
        outputs: vec![GeoContract::new(format!("{operation_id}.response"))],
        capabilities: capabilities.clone(),
        determinism: Determinism::ToleranceBounded,
        evidence_hooks: evidence_hooks.clone(),
        opaque: false,
    };
    AdapterManifest {
        schema_version: ADAPTER_MANIFEST_SCHEMA_VERSION.into(),
        adapter_id: ADAPTER_ID.into(),
        adapter_version: ADAPTER_VERSION.into(),
        backend: BackendIdentity {
            family: BackendFamily::OgcWebService,
            engine_version: "WMS-1.3.0/WFS-2.0.0".into(),
            build_digest: OGC_ADAPTER_BUILD_DIGEST.into(),
            components: [("http-client".into(), "ureq-3".into())]
                .into_iter()
                .collect(),
        },
        license: "Apache-2.0 OR MIT".into(),
        operations: vec![
            operation(OgcOperation::GetCapabilities.operation_id()),
            operation(OgcOperation::WmsGetMap.operation_id()),
            operation(OgcOperation::WfsGetFeature.operation_id()),
        ],
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        thread,
    };

    use super::*;

    fn fixture(content_type: &str, body: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let content_type = content_type.to_string();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            serve(&mut stream, &content_type, &body);
        });
        format!("http://{address}/ogc")
    }

    fn serve(stream: &mut TcpStream, content_type: &str, body: &[u8]) {
        let mut request = [0_u8; 8192];
        let _ = stream.read(&mut request);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(response.as_bytes()).expect("headers");
        stream.write_all(body).expect("body");
    }

    fn adapter() -> OgcServiceAdapter {
        OgcServiceAdapter::new(RemoteAccessPolicy::from_env())
    }

    #[test]
    fn admits_and_receipts_wms_capabilities_map_and_wfs_features() {
        let capabilities = adapter()
            .execute(&OgcRequest::GetCapabilities {
                endpoint: fixture(
                    "application/xml",
                    br#"<WMS_Capabilities version="1.3.0"/>"#.to_vec(),
                ),
                service: "WMS".into(),
                version: "1.3.0".into(),
            })
            .expect("capabilities");
        assert!(capabilities.receipt.admission.verification_eligible);
        assert_eq!(capabilities.receipt.io.format, CloudFormat::Wms);

        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(b"fixture");
        let map = adapter()
            .execute(&OgcRequest::WmsGetMap(WmsGetMapRequest {
                endpoint: fixture("image/png", png),
                version: "1.3.0".into(),
                layers: vec!["verified:district".into()],
                styles: vec![],
                crs: Crs::nagoya_projected(),
                bbox: [0.0, 0.0, 20.0, 20.0],
                width: 256,
                height: 256,
                format: "image/png".into(),
                transparent: true,
            }))
            .expect("map");
        assert_eq!(map.receipt.io.decoded_items, 256 * 256);
        assert_eq!(map.receipt.coordinate_unit, Some(CoordinateUnit::Metres));

        let features = adapter()
            .execute(&OgcRequest::WfsGetFeature(WfsGetFeatureRequest {
                endpoint: fixture(
                    "application/geo+json",
                    br#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{},"geometry":null}]}"#.to_vec(),
                ),
                version: "2.0.0".into(),
                type_names: vec!["verified:poi".into()],
                crs: Crs::wgs84(),
                bbox: Some([136.0, 35.0, 137.0, 36.0]),
                count: Some(10),
            }))
            .expect("features");
        assert_eq!(features.receipt.io.decoded_items, 1);
        assert_eq!(features.receipt.io.format, CloudFormat::Wfs);
    }

    #[test]
    fn rejects_manifest_drift_bad_content_type_exception_and_invalid_bbox() {
        let mut manifest = ogc_web_service_manifest();
        manifest.backend.engine_version = "drifted".into();
        let mut policy = CapabilityPolicy::read_only_network(&manifest.adapter_id);
        policy
            .required_evidence_hooks
            .insert(EvidenceHook::Warnings);
        let drifted =
            OgcServiceAdapter::with_contracts(manifest, policy, RemoteAccessPolicy::from_env());
        let admission = drifted.execute(&OgcRequest::GetCapabilities {
            endpoint: fixture(
                "application/xml",
                br#"<WMS_Capabilities version="1.3.0"/>"#.to_vec(),
            ),
            service: "WMS".into(),
            version: "1.3.0".into(),
        });
        assert!(matches!(admission, Err(OgcAdapterError::Admission(_))));

        let wrong_type = adapter().execute(&OgcRequest::GetCapabilities {
            endpoint: fixture("text/html", b"not capabilities".to_vec()),
            service: "WMS".into(),
            version: "1.3.0".into(),
        });
        assert!(matches!(wrong_type, Err(OgcAdapterError::Response(_))));

        let exception = adapter().execute(&OgcRequest::GetCapabilities {
            endpoint: fixture(
                "application/xml",
                b"<ExceptionReport>denied</ExceptionReport>".to_vec(),
            ),
            service: "WFS".into(),
            version: "2.0.0".into(),
        });
        assert!(matches!(exception, Err(OgcAdapterError::Response(_))));

        let invalid_bbox = adapter().execute(&OgcRequest::WmsGetMap(WmsGetMapRequest {
            endpoint: "http://127.0.0.1:1/never".into(),
            version: "1.3.0".into(),
            layers: vec!["layer".into()],
            styles: vec![],
            crs: Crs::nagoya_projected(),
            bbox: [20.0, 0.0, 10.0, 1.0],
            width: 256,
            height: 256,
            format: "image/png".into(),
            transparent: false,
        }));
        assert!(matches!(invalid_bbox, Err(OgcAdapterError::Request(_))));
    }
}
