//! Command + Workflow evaluation of reproducible performance matrices.

use std::sync::Mutex;

use genegis_core::{
    evaluate_performance_matrix, Command, CommandBus, CommandEnvelope, CommandOrigin,
    InputSnapshot, PerformanceMatrixProfile, PerformanceMatrixReceipt, PerformanceMeasurement,
    Project, WorkflowDigest, WorkflowExecution, WorkflowExecutionContext, WorkflowExecutionError,
    WorkflowExecutionEvent, WorkflowExecutor,
};
use genegis_crs::{ChecksumVerification, SourceSnapshot};
use genegis_workflow::{performance_matrix_evaluation_template, GeoWorkflow};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::AnalysisError;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerformanceMatrixWorkflowReceipt {
    pub command_id: String,
    pub workflow_digest: WorkflowDigest,
    pub matrix: PerformanceMatrixReceipt,
}

struct MatrixExecutor {
    profile: PerformanceMatrixProfile,
    measurements: Vec<PerformanceMeasurement>,
    receipt: Mutex<Option<PerformanceMatrixReceipt>>,
}

impl WorkflowExecutor for MatrixExecutor {
    fn execute(
        &self,
        _workflow: &GeoWorkflow,
        context: &WorkflowExecutionContext,
    ) -> Result<WorkflowExecution, WorkflowExecutionError> {
        let receipt = evaluate_performance_matrix(&self.profile, self.measurements.clone())
            .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        let result_digest = receipt.receipt_digest.clone();
        *self.receipt.lock().map_err(|_| {
            WorkflowExecutionError::Failed("performance matrix lock poisoned".into())
        })? = Some(receipt.clone());
        Ok(WorkflowExecution {
            result_digest,
            output: serde_json::json!({"verdict": receipt.verdict, "regressions": receipt.regressions, "pending": receipt.pending}),
            evidence: serde_json::to_value(&receipt)
                .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?,
            events: vec![WorkflowExecutionEvent {
                kind: "performance_matrix_evaluated".into(),
                source_uri: Some("benchmark-profile://performance-matrix".into()),
                observed_at: context.command_timestamp,
                details: serde_json::json!({"command_id": context.command_id, "workflow_digest": context.workflow_digest}),
            }],
        })
    }
}

pub fn evaluate_performance_matrix_workflow(
    profile: PerformanceMatrixProfile,
    measurements: Vec<PerformanceMeasurement>,
) -> Result<PerformanceMatrixWorkflowReceipt, AnalysisError> {
    let profile_digest = digest(&profile)?;
    let mut source = SourceSnapshot::new("benchmark-profile://performance-matrix");
    source.checksum = Some(profile_digest.clone());
    source.observed_checksum = Some(profile_digest);
    source.checksum_status = ChecksumVerification::Verified;
    let workflow = performance_matrix_evaluation_template(source.clone());
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
    .with_input_snapshot(InputSnapshot::new("performance-matrix", source));
    let command_id = envelope.id.to_string();
    let executor = MatrixExecutor {
        profile,
        measurements,
        receipt: Mutex::new(None),
    };
    let mut project = Project::new("Performance matrix evaluation");
    let mut bus = CommandBus::new(project.clone());
    bus.register_workflow(workflow)
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    let execution = bus
        .apply_with_executor(&mut project, envelope, &executor)
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    let matrix = executor
        .receipt
        .into_inner()
        .map_err(|_| AnalysisError::Message("performance matrix lock poisoned".into()))?
        .ok_or_else(|| AnalysisError::Message("executor returned no performance matrix".into()))?;
    if execution.result_digest.as_deref() != Some(matrix.receipt_digest.as_str()) {
        return Err(AnalysisError::Message(
            "CommandBus and performance matrix digests differ".into(),
        ));
    }
    Ok(PerformanceMatrixWorkflowReceipt {
        command_id,
        workflow_digest,
        matrix,
    })
}

fn digest<T: Serialize>(value: &T) -> Result<String, AnalysisError> {
    let bytes =
        serde_json::to_vec(value).map_err(|error| AnalysisError::Message(error.to_string()))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests {
    use genegis_core::{
        PerformanceDimension, PerformanceEnvironment, PerformanceMatrixVerdict,
        PerformanceMeasurementStatus,
    };

    use super::*;

    #[test]
    fn pending_hardware_receipt_is_command_and_workflow_bound() {
        let profile: PerformanceMatrixProfile = serde_json::from_str(include_str!(
            "../../../benchmarks/profiles/local-first-nagoya.json"
        ))
        .expect("profile");
        let measurements = profile
            .metrics
            .iter()
            .map(|budget| PerformanceMeasurement {
                metric_id: budget.id.clone(),
                fixture_digest: budget
                    .fixture_digest
                    .clone()
                    .unwrap_or_else(|| profile.dataset_digest.clone()),
                evidence_digest: None,
                observed_at: "2026-08-26T10:00:00Z".into(),
                environment: PerformanceEnvironment {
                    os: "test".into(),
                    cpu: "test-cpu".into(),
                    gpu: None,
                    network_profile: "offline-fixture".into(),
                    logical_concurrency: 4,
                    build_digest: profile.build_digest.clone(),
                },
                status: if budget.dimension == PerformanceDimension::Gpu {
                    PerformanceMeasurementStatus::NotMeasured {
                        reason: "hardware receipt required".into(),
                    }
                } else {
                    PerformanceMeasurementStatus::Measured {
                        value: budget.threshold,
                        iterations: 3,
                    }
                },
            })
            .collect();
        let receipt =
            evaluate_performance_matrix_workflow(profile, measurements).expect("matrix workflow");
        assert_eq!(receipt.matrix.verdict, PerformanceMatrixVerdict::Pending);
        assert!(receipt.workflow_digest.as_str().starts_with("sha256:"));
    }
}
