//! Command + Workflow execution boundary for receipted WMS/WFS reads.

use std::sync::Mutex;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use genegis_adapter::{OgcRequest, OgcResponse, OgcServiceAdapter, OgcServiceReceipt};
use genegis_core::{
    Command, CommandBus, CommandEnvelope, CommandOrigin, InputSnapshot, Project, WorkflowDigest,
    WorkflowExecution, WorkflowExecutionContext, WorkflowExecutionError, WorkflowExecutionEvent,
    WorkflowExecutor,
};
use genegis_crs::{Crs, SourceSnapshot};
use genegis_storage::RemoteAccessPolicy;
use genegis_workflow::{ogc_service_read_template, GeoWorkflow};
use serde::{Deserialize, Serialize};

use crate::AnalysisError;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OgcWorkflowResult {
    pub command_id: String,
    pub workflow_digest: WorkflowDigest,
    pub result_digest: String,
    pub content_type: String,
    pub body_base64: String,
    pub receipt: OgcServiceReceipt,
}

struct OgcWorkflowExecutor {
    adapter: OgcServiceAdapter,
    request: OgcRequest,
    response: Mutex<Option<OgcResponse>>,
}

impl WorkflowExecutor for OgcWorkflowExecutor {
    fn execute(
        &self,
        _workflow: &GeoWorkflow,
        context: &WorkflowExecutionContext,
    ) -> Result<WorkflowExecution, WorkflowExecutionError> {
        let response = self
            .adapter
            .execute(&self.request)
            .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        let receipt = serde_json::to_value(&response.receipt)
            .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        let result_digest = response.receipt.output_digest.clone();
        let source_uri = response.receipt.request_url.clone();
        let output = serde_json::json!({
            "operation": response.receipt.operation,
            "content_type": response.receipt.content_type,
            "response_bytes": response.bytes.len(),
            "crs": response.receipt.crs,
            "coordinate_unit": response.receipt.coordinate_unit,
            "io_receipt_digest": response.receipt.io.digest().map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?,
        });
        *self
            .response
            .lock()
            .map_err(|_| WorkflowExecutionError::Failed("OGC response lock poisoned".into()))? =
            Some(response);
        Ok(WorkflowExecution {
            result_digest,
            output,
            evidence: receipt,
            events: vec![WorkflowExecutionEvent {
                kind: "ogc_service_read".into(),
                source_uri: Some(source_uri),
                observed_at: context.command_timestamp,
                details: serde_json::json!({
                    "workflow_digest": context.workflow_digest,
                    "command_id": context.command_id,
                }),
            }],
        })
    }
}

pub fn execute_ogc_workflow(
    request: OgcRequest,
    remote_policy: RemoteAccessPolicy,
) -> Result<OgcWorkflowResult, AnalysisError> {
    let (service, operation, endpoint, crs) = request_contract(&request);
    let source = SourceSnapshot::new(endpoint);
    let workflow = ogc_service_read_template(service, operation, source.clone(), crs.clone());
    let workflow_digest = WorkflowDigest::new(
        workflow
            .stable_digest()
            .map_err(|error| AnalysisError::Message(error.to_string()))?,
    );
    let mut snapshot = InputSnapshot::new("service", source.clone());
    if let Some(crs) = crs {
        snapshot = snapshot.with_crs(crs);
    }
    let envelope = CommandEnvelope::new(
        CommandOrigin::Ui,
        Command::RunWorkflow {
            workflow_id: workflow.id,
        },
    )
    .with_workflow_digest(workflow_digest.clone())
    .with_source_snapshot(source)
    .with_input_snapshot(snapshot);
    let command_id = envelope.id;
    let executor = OgcWorkflowExecutor {
        adapter: OgcServiceAdapter::new(remote_policy),
        request,
        response: Mutex::new(None),
    };
    let mut project = Project::new("OGC service read");
    let mut bus = CommandBus::new(project.clone());
    bus.register_workflow(workflow)
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    let execution = bus
        .apply_with_executor(&mut project, envelope, &executor)
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    let response = executor
        .response
        .into_inner()
        .map_err(|_| AnalysisError::Message("OGC response lock poisoned".into()))?
        .ok_or_else(|| AnalysisError::Message("OGC executor returned no response".into()))?;
    let result_digest = execution
        .result_digest
        .ok_or_else(|| AnalysisError::Message("OGC workflow returned no digest".into()))?;
    if result_digest != response.receipt.output_digest {
        return Err(AnalysisError::Message(
            "OGC workflow and adapter result digests differ".into(),
        ));
    }
    Ok(OgcWorkflowResult {
        command_id: command_id.to_string(),
        workflow_digest,
        result_digest,
        content_type: response.receipt.content_type.clone(),
        body_base64: BASE64.encode(&response.bytes),
        receipt: response.receipt,
    })
}

fn request_contract(request: &OgcRequest) -> (&str, &str, String, Option<Crs>) {
    match request {
        OgcRequest::GetCapabilities {
            endpoint, service, ..
        } => (service, "GetCapabilities", endpoint.clone(), None),
        OgcRequest::WmsGetMap(map) => {
            ("WMS", "GetMap", map.endpoint.clone(), Some(map.crs.clone()))
        }
        OgcRequest::WfsGetFeature(feature) => (
            "WFS",
            "GetFeature",
            feature.endpoint.clone(),
            Some(feature.crs.clone()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::{Shutdown, TcpListener, TcpStream},
        thread,
    };

    use genegis_adapter::{OgcOperation, WfsGetFeatureRequest};
    use genegis_storage::CloudFormat;

    use super::*;

    fn fixture(content_type: &str, body: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let content_type = content_type.to_string();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            serve(&mut stream, &content_type, &body);
        });
        format!("http://{address}/wfs")
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
        stream.flush().expect("flush");
        stream.shutdown(Shutdown::Write).expect("shutdown");
    }

    fn feature_request(endpoint: String) -> OgcRequest {
        OgcRequest::WfsGetFeature(WfsGetFeatureRequest {
            endpoint,
            version: "2.0.0".into(),
            type_names: vec!["verified:poi".into()],
            crs: Crs::wgs84(),
            bbox: Some([136.0, 35.0, 137.0, 36.0]),
            count: Some(10),
        })
    }

    #[test]
    fn executes_wfs_only_through_command_workflow_and_receipts_result() {
        let body = br#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{"category":"school"},"geometry":null}]}"#.to_vec();
        let result = execute_ogc_workflow(
            feature_request(fixture("application/geo+json", body.clone())),
            RemoteAccessPolicy::from_env(),
        )
        .expect("receipted WFS workflow");

        uuid::Uuid::parse_str(&result.command_id).expect("command UUID");
        assert!(result.workflow_digest.as_str().starts_with("sha256:"));
        assert_eq!(result.result_digest, result.receipt.output_digest);
        assert_eq!(result.receipt.operation, OgcOperation::WfsGetFeature);
        assert!(result.receipt.admission.admitted);
        assert!(result.receipt.admission.verification_eligible);
        assert_eq!(result.receipt.io.format, CloudFormat::Wfs);
        assert_eq!(result.receipt.io.decoded_items, 1);
        assert_eq!(BASE64.decode(&result.body_base64).expect("base64"), body);
        assert_eq!(result.receipt.source.checksum, Some(result.result_digest));
    }

    #[test]
    fn command_workflow_fails_closed_on_service_exception() {
        let request = OgcRequest::GetCapabilities {
            endpoint: fixture(
                "application/xml",
                b"<ExceptionReport>denied</ExceptionReport>".to_vec(),
            ),
            service: "WFS".into(),
            version: "2.0.0".into(),
        };
        let error = execute_ogc_workflow(request, RemoteAccessPolicy::from_env())
            .expect_err("exception document must fail");
        assert!(
            error.to_string().to_ascii_lowercase().contains("exception"),
            "{error}"
        );
    }
}
