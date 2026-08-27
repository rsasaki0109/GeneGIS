//! Command + Workflow execution boundary for cursor-bounded live feeds.

use std::sync::Mutex;

use genegis_adapter::{
    FeedFreshnessPolicy, FeedObservationSnapshot, LiveFeedAdapter, LiveFeedReceipt,
    LiveFeedRequest, LiveFeedResponse,
};
use genegis_core::{
    Command, CommandBus, CommandEnvelope, CommandOrigin, InputSnapshot, Project, WorkflowDigest,
    WorkflowExecution, WorkflowExecutionContext, WorkflowExecutionError, WorkflowExecutionEvent,
    WorkflowExecutor,
};
use genegis_crs::SourceSnapshot;
use genegis_storage::RemoteAccessPolicy;
use genegis_workflow::{live_feed_ingest_template, GeoWorkflow};
use serde::{Deserialize, Serialize};

use crate::AnalysisError;

/// Command/workflow result for one committed live-feed page.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveFeedWorkflowResult {
    /// Applied command identity.
    pub command_id: String,
    /// Exact ingestion workflow identity.
    pub workflow_digest: WorkflowDigest,
    /// Canonical immutable snapshot-set digest.
    pub result_digest: String,
    /// Content-addressed observation snapshots.
    pub snapshots: Vec<FeedObservationSnapshot>,
    /// Adapter, cursor, watermark, freshness, source, and I/O evidence.
    pub receipt: LiveFeedReceipt,
}

struct LiveFeedExecutor {
    adapter: LiveFeedAdapter,
    request: LiveFeedRequest,
    policy: FeedFreshnessPolicy,
    response: Mutex<Option<LiveFeedResponse>>,
}

impl WorkflowExecutor for LiveFeedExecutor {
    fn execute(
        &self,
        _workflow: &GeoWorkflow,
        context: &WorkflowExecutionContext,
    ) -> Result<WorkflowExecution, WorkflowExecutionError> {
        let response = self
            .adapter
            .execute(&self.request, &self.policy)
            .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        let evidence = serde_json::to_value(&response.receipt)
            .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        let result_digest = response.receipt.output_digest.clone();
        let output = serde_json::json!({
            "domain": response.receipt.domain,
            "provider_id": response.receipt.provider_id,
            "requested_cursor": response.receipt.requested_cursor,
            "next_cursor": response.receipt.next_cursor,
            "next_watermark": response.receipt.next_watermark,
            "fresh": response.receipt.fresh,
            "observation_count": response.snapshots.len(),
        });
        let source_uri = response.receipt.source.uri.clone();
        *self
            .response
            .lock()
            .map_err(|_| WorkflowExecutionError::Failed("live-feed lock poisoned".into()))? =
            Some(response);
        Ok(WorkflowExecution {
            result_digest,
            output,
            evidence,
            events: vec![WorkflowExecutionEvent {
                kind: "live_feed_window_committed".into(),
                source_uri: Some(source_uri),
                observed_at: context.command_timestamp,
                details: serde_json::json!({
                    "command_id": context.command_id,
                    "workflow_digest": context.workflow_digest,
                }),
            }],
        })
    }
}

/// Execute one feed page exclusively through Command + Workflow Graph.
pub fn execute_live_feed_workflow(
    request: LiveFeedRequest,
    policy: FeedFreshnessPolicy,
    remote_policy: RemoteAccessPolicy,
) -> Result<LiveFeedWorkflowResult, AnalysisError> {
    let source = SourceSnapshot::new(request.endpoint.clone());
    let domain = serde_json::to_value(request.domain)
        .map_err(|error| AnalysisError::Message(error.to_string()))?
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    let workflow = live_feed_ingest_template(
        &domain,
        &request.provider_id,
        source.clone(),
        request.after_cursor,
        &request.watermark,
        request.limit,
    );
    let workflow_digest = WorkflowDigest::new(
        workflow
            .stable_digest()
            .map_err(|error| AnalysisError::Message(error.to_string()))?,
    );
    let envelope = CommandEnvelope::new(
        CommandOrigin::Ui,
        Command::RunWorkflow {
            workflow_id: workflow.id,
        },
    )
    .with_workflow_digest(workflow_digest.clone())
    .with_source_snapshot(source.clone())
    .with_input_snapshot(InputSnapshot::new("live-feed", source));
    let command_id = envelope.id;
    let executor = LiveFeedExecutor {
        adapter: LiveFeedAdapter::new(remote_policy),
        request,
        policy,
        response: Mutex::new(None),
    };
    let mut project = Project::new("Live spatial feed");
    let mut bus = CommandBus::new(project.clone());
    bus.register_workflow(workflow)
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    let execution = bus
        .apply_with_executor(&mut project, envelope, &executor)
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    let response = executor
        .response
        .into_inner()
        .map_err(|_| AnalysisError::Message("live-feed lock poisoned".into()))?
        .ok_or_else(|| AnalysisError::Message("live-feed executor returned no response".into()))?;
    let result_digest = execution
        .result_digest
        .ok_or_else(|| AnalysisError::Message("live-feed workflow returned no digest".into()))?;
    if result_digest != response.receipt.output_digest {
        return Err(AnalysisError::Message(
            "live-feed workflow and adapter result digests differ".into(),
        ));
    }
    Ok(LiveFeedWorkflowResult {
        command_id: command_id.to_string(),
        workflow_digest,
        result_digest,
        snapshots: response.snapshots,
        receipt: response.receipt,
    })
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::{Shutdown, TcpListener},
        thread,
    };

    use genegis_adapter::FeedDomain;

    use super::*;

    #[test]
    fn commits_cursor_only_through_command_workflow() {
        let body = serde_json::json!({
            "next_cursor": 8,
            "watermark": "2026-08-26T10:00:00Z",
            "observations": [{
                "id": "sensor-8", "sequence": 8,
                "observed_at": "2026-08-26T09:59:30Z",
                "crs": "EPSG:4326",
                "geometry": {"type": "Point", "coordinates": [136.9, 35.18]},
                "values": {"pm25": 8.0, "unit": "ug/m3"},
                "source_revision": "fixture-v1"
            }]
        });
        let bytes = serde_json::to_vec(&body).expect("json");
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut request = Vec::new();
            let mut chunk = [0_u8; 2048];
            loop {
                let read = stream.read(&mut chunk).expect("read");
                request.extend_from_slice(&chunk[..read]);
                let Some(end) = request.windows(4).position(|value| value == b"\r\n\r\n") else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                if request.len() >= end + 4 + length {
                    break;
                }
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            );
            stream.write_all(response.as_bytes()).expect("headers");
            stream.write_all(&bytes).expect("body");
            stream.flush().expect("flush");
            stream.shutdown(Shutdown::Write).expect("shutdown");
        });
        let result = execute_live_feed_workflow(
            LiveFeedRequest {
                domain: FeedDomain::Sensor,
                endpoint: format!("http://{address}/feed"),
                provider_id: "fixture.sensor".into(),
                provider_version: "1".into(),
                after_cursor: 7,
                watermark: "2026-08-26T09:55:00Z".into(),
                limit: 10,
                evaluated_at: "2026-08-26T10:01:00Z".into(),
            },
            FeedFreshnessPolicy::default(),
            RemoteAccessPolicy::from_env(),
        )
        .expect("workflow");
        uuid::Uuid::parse_str(&result.command_id).expect("command id");
        assert_eq!(result.receipt.next_cursor, 8);
        assert_eq!(result.result_digest, result.receipt.output_digest);
        assert!(result.receipt.fresh);
    }
}
