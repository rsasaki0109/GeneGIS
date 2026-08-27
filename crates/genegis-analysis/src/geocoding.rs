//! Command + Workflow execution boundary for governed geocoding.

use std::sync::Mutex;

use genegis_adapter::{
    GeocodingAdapter, GeocodingPrivacyPolicy, GeocodingProvider, GeocodingRatePolicy,
    GeocodingReceipt, GeocodingRequest, GeocodingResponse,
};
use genegis_core::{
    Command, CommandBus, CommandEnvelope, CommandOrigin, InputSnapshot, Project, WorkflowDigest,
    WorkflowExecution, WorkflowExecutionContext, WorkflowExecutionError, WorkflowExecutionEvent,
    WorkflowExecutor,
};
use genegis_workflow::{geocoding_template, GeoWorkflow};
use serde::{Deserialize, Serialize};

use crate::AnalysisError;

/// Command/workflow result paired with provider candidates and evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeocodingWorkflowResult {
    /// Applied command identity.
    pub command_id: String,
    /// Stable digest of the exact governed workflow.
    pub workflow_digest: WorkflowDigest,
    /// Canonical candidate-result digest.
    pub result_digest: String,
    /// Results in request order.
    pub results: Vec<genegis_adapter::GeocodeQueryResult>,
    /// Adapter admission, policies, source, confidence, and I/O evidence.
    pub receipt: GeocodingReceipt,
}

struct GeocodingWorkflowExecutor {
    adapter: GeocodingAdapter,
    provider: GeocodingProvider,
    request: GeocodingRequest,
    privacy: GeocodingPrivacyPolicy,
    rate: GeocodingRatePolicy,
    response: Mutex<Option<GeocodingResponse>>,
}

impl WorkflowExecutor for GeocodingWorkflowExecutor {
    fn execute(
        &self,
        _workflow: &GeoWorkflow,
        context: &WorkflowExecutionContext,
    ) -> Result<WorkflowExecution, WorkflowExecutionError> {
        let response = self
            .adapter
            .execute(&self.request, &self.provider, self.privacy, &self.rate)
            .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        let receipt = serde_json::to_value(&response.receipt)
            .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        let result_digest = response.receipt.output_digest.clone();
        let source_uri = response.receipt.source.uri.clone();
        let output = serde_json::json!({
            "provider_id": response.receipt.provider_id,
            "query_count": response.results.len(),
            "candidate_count": response.results.iter().map(|result| result.candidates.len()).sum::<usize>(),
            "matched_queries": response.receipt.matched_queries,
            "ambiguous_queries": response.receipt.ambiguous_queries,
            "unmatched_queries": response.receipt.unmatched_queries,
            "crs": response.receipt.crs,
            "coordinate_unit": response.receipt.coordinate_unit,
            "policy_digest": response.receipt.policy_digest,
        });
        *self.response.lock().map_err(|_| {
            WorkflowExecutionError::Failed("geocoding response lock poisoned".into())
        })? = Some(response);
        Ok(WorkflowExecution {
            result_digest,
            output,
            evidence: receipt,
            events: vec![WorkflowExecutionEvent {
                kind: "geocoding".into(),
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

/// Execute one geocoding request exclusively through Command + Workflow Graph.
pub fn execute_geocoding_workflow(
    request: GeocodingRequest,
    provider: GeocodingProvider,
    privacy: GeocodingPrivacyPolicy,
    rate: GeocodingRatePolicy,
) -> Result<GeocodingWorkflowResult, AnalysisError> {
    let source = provider.source_snapshot();
    let (provider_id, mode) = provider_contract(&provider, request.mode);
    let privacy_name = serde_json::to_value(privacy)
        .map_err(|error| AnalysisError::Message(error.to_string()))?
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    let workflow = geocoding_template(
        mode,
        provider_id,
        source.clone(),
        request.queries.len() as u32,
        request.max_candidates,
        &privacy_name,
        rate.minimum_interval_ms,
    );
    let workflow_digest = WorkflowDigest::new(
        workflow
            .stable_digest()
            .map_err(|error| AnalysisError::Message(error.to_string()))?,
    );
    let snapshot = InputSnapshot::new("queries", source.clone());
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
    let executor = GeocodingWorkflowExecutor {
        adapter: GeocodingAdapter::new(),
        provider,
        request,
        privacy,
        rate,
        response: Mutex::new(None),
    };
    let mut project = Project::new("Governed geocoding");
    let mut bus = CommandBus::new(project.clone());
    bus.register_workflow(workflow)
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    let execution = bus
        .apply_with_executor(&mut project, envelope, &executor)
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    let response = executor
        .response
        .into_inner()
        .map_err(|_| AnalysisError::Message("geocoding response lock poisoned".into()))?
        .ok_or_else(|| AnalysisError::Message("geocoding executor returned no response".into()))?;
    let result_digest = execution
        .result_digest
        .ok_or_else(|| AnalysisError::Message("geocoding workflow returned no digest".into()))?;
    if result_digest != response.receipt.output_digest {
        return Err(AnalysisError::Message(
            "geocoding workflow and adapter result digests differ".into(),
        ));
    }
    Ok(GeocodingWorkflowResult {
        command_id: command_id.to_string(),
        workflow_digest,
        result_digest,
        results: response.results,
        receipt: response.receipt,
    })
}

fn provider_contract(
    provider: &GeocodingProvider,
    mode: genegis_adapter::GeocodingMode,
) -> (&str, &'static str) {
    let provider_id = match provider {
        GeocodingProvider::OfflineGazetteer { provider_id, .. }
        | GeocodingProvider::HttpJson { provider_id, .. } => provider_id.as_str(),
    };
    let mode = match mode {
        genegis_adapter::GeocodingMode::Interactive => "interactive",
        genegis_adapter::GeocodingMode::Batch => "batch",
    };
    (provider_id, mode)
}

#[cfg(test)]
mod tests {
    use genegis_adapter::{GazetteerEntry, GeocodingMode, GeocodingQuery};
    use genegis_crs::{ChecksumVerification, SourceSnapshot};
    use sha2::{Digest, Sha256};

    use super::*;

    #[test]
    fn geocoding_runs_through_command_workflow_with_exact_receipt_digest() {
        let digest = format!("sha256:{:x}", Sha256::digest(b"nagoya-gazetteer-v1"));
        let mut source = SourceSnapshot::new("fixture://nagoya-gazetteer/v1");
        source.license = Some("CC0-1.0".into());
        source.checksum = Some(digest.clone());
        source.observed_checksum = Some(digest);
        source.checksum_status = ChecksumVerification::Verified;
        let provider = GeocodingProvider::OfflineGazetteer {
            provider_id: "fixture.nagoya".into(),
            version: "1".into(),
            source,
            entries: vec![GazetteerEntry {
                feature_id: "station:nagoya".into(),
                label: "名古屋駅".into(),
                aliases: vec!["Nagoya Station".into()],
                longitude: 136.8815,
                latitude: 35.1709,
            }],
        };
        let result = execute_geocoding_workflow(
            GeocodingRequest {
                mode: GeocodingMode::Interactive,
                queries: vec![GeocodingQuery {
                    id: "q1".into(),
                    text: "名古屋駅".into(),
                }],
                language: "ja".into(),
                max_candidates: 1,
            },
            provider,
            GeocodingPrivacyPolicy::LocalOnly,
            GeocodingRatePolicy::default(),
        )
        .expect("workflow");
        assert_eq!(result.result_digest, result.receipt.output_digest);
        assert!(result.receipt.admission.admitted);
        assert_eq!(result.results[0].candidates[0].confidence, 1.0);
    }
}
