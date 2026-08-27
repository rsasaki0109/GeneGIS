//! Deterministic, verified spatial alerts and append-only acknowledgements.

use std::{collections::BTreeSet, sync::Mutex};

use chrono::DateTime;
use genegis_core::{
    Command, CommandBus, CommandEnvelope, CommandOrigin, InputSnapshot, Project, WorkflowDigest,
    WorkflowExecution, WorkflowExecutionContext, WorkflowExecutionError, WorkflowExecutionEvent,
    WorkflowExecutor,
};
use genegis_crs::{ChecksumVerification, SourceSnapshot};
use genegis_workflow::{
    alert_acknowledgement_template, verified_alert_evaluation_template, GeoWorkflow,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Verified alert schema version.
pub const VERIFIED_ALERT_SCHEMA_VERSION: &str = "0.1.0";

/// Deterministic comparison operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertComparison {
    /// Metric is strictly greater than threshold.
    GreaterThan,
    /// Metric is greater than or equal to threshold.
    GreaterThanOrEqual,
    /// Metric is strictly less than threshold.
    LessThan,
    /// Metric is less than or equal to threshold.
    LessThanOrEqual,
}

/// Closed deterministic alert-rule set; there is no LLM judgement variant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum VerifiedAlertRule {
    /// Numeric threshold rule.
    Threshold {
        /// Metric field.
        field: String,
        /// Metric unit.
        unit: String,
        /// Comparison operator.
        comparison: AlertComparison,
        /// Threshold value.
        threshold: f64,
    },
    /// Absolute z-score anomaly against a content-addressed baseline.
    ZScoreAnomaly {
        /// Metric field.
        field: String,
        /// Metric unit.
        unit: String,
        /// Baseline model/data identity.
        baseline_digest: String,
        /// Baseline mean.
        mean: f64,
        /// Positive baseline standard deviation.
        standard_deviation: f64,
        /// Positive absolute z-score threshold.
        absolute_z_threshold: f64,
    },
}

/// Versioned deterministic alert policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifiedAlertPolicy {
    /// Stable policy identity.
    pub id: String,
    /// Rule evaluated by the verifier.
    pub rule: VerifiedAlertRule,
    /// Informational severity label.
    pub severity: String,
    /// Require the triggering input window to be fresh.
    pub require_fresh: bool,
}

/// Metric value entering alert evaluation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlertMetric {
    /// Metric field.
    pub field: String,
    /// Numeric value.
    pub value: f64,
    /// Metric unit.
    pub unit: String,
}

/// Exact triggering spatial data window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlertTriggeringWindow {
    /// Exclusive input cursor.
    pub cursor_start: u64,
    /// Inclusive committed cursor.
    pub cursor_end: u64,
    /// Start event-time watermark.
    pub watermark_start: String,
    /// End event-time watermark.
    pub watermark_end: String,
    /// Whether freshness policy passed upstream.
    pub fresh: bool,
    /// Immutable observation snapshots used by the evaluation.
    pub snapshot_digests: Vec<String>,
    /// Exact result computed from these snapshots.
    pub result_digest: String,
    /// Checksum-verified feed response source.
    pub source: SourceSnapshot,
}

/// One deterministic verifier assertion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlertVerificationCheck {
    /// Stable check identity.
    pub name: String,
    /// Whether it passed.
    pub passed: bool,
    /// Safe diagnostic detail.
    pub detail: String,
}

/// Append-only human acknowledgement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlertAcknowledgement {
    /// Stable acknowledgement identity.
    pub id: String,
    /// Human/service actor identity.
    pub actor: String,
    /// RFC 3339 acknowledgement time.
    pub acknowledged_at: String,
    /// SHA-256 of the acknowledgement note; raw note is not retained.
    pub note_digest: String,
}

/// Sealed triggered alert with full verification and acknowledgement history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifiedAlertRecord {
    /// Alert schema version.
    pub schema_version: String,
    /// Stable identity of the original trigger, independent of acknowledgements.
    pub alert_id: String,
    /// Policy bytes identity.
    pub policy_digest: String,
    /// Exact policy.
    pub policy: VerifiedAlertPolicy,
    /// Exact triggering window.
    pub triggering_window: AlertTriggeringWindow,
    /// Evaluated metric.
    pub metric: AlertMetric,
    /// Threshold value or absolute z-score produced by the rule.
    pub evaluation_value: f64,
    /// Deterministic verifier identity.
    pub verifier: String,
    /// Verifier checks.
    pub checks: Vec<AlertVerificationCheck>,
    /// RFC 3339 trigger time.
    pub triggered_at: String,
    /// Append-only acknowledgements.
    pub acknowledgements: Vec<AlertAcknowledgement>,
    /// Digest of the current record including acknowledgement history.
    pub record_digest: String,
}

/// Result of one evaluation, triggered or not.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifiedAlertEvaluation {
    /// Applied command identity.
    pub command_id: String,
    /// Exact evaluation Workflow Graph identity.
    pub workflow_digest: WorkflowDigest,
    /// Whether the rule triggered.
    pub triggered: bool,
    /// Triggered sealed alert, absent for a non-triggering evaluation.
    pub alert: Option<VerifiedAlertRecord>,
    /// Digest of the complete evaluation result.
    pub evaluation_digest: String,
}

/// Result of an acknowledgement command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlertAcknowledgementReceipt {
    /// Applied command identity.
    pub command_id: String,
    /// Exact acknowledgement Workflow Graph identity.
    pub workflow_digest: WorkflowDigest,
    /// Updated alert record.
    pub alert: VerifiedAlertRecord,
}

/// Fail-closed alert error.
#[derive(Debug, Error)]
pub enum VerifiedAlertError {
    /// Policy, window, metric, verifier, or acknowledgement is invalid.
    #[error("invalid verified alert: {0}")]
    Invalid(String),
    /// Canonical serialization failed.
    #[error("verified alert serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    /// Command or workflow execution failed.
    #[error("verified alert workflow failed: {0}")]
    Workflow(String),
}

#[derive(Clone)]
struct EvaluationInput {
    policy: VerifiedAlertPolicy,
    metric: AlertMetric,
    window: AlertTriggeringWindow,
    evaluated_at: String,
}

struct EvaluationExecutor {
    input: EvaluationInput,
    result: Mutex<Option<(bool, Option<VerifiedAlertRecord>, String)>>,
}

impl WorkflowExecutor for EvaluationExecutor {
    fn execute(
        &self,
        _workflow: &GeoWorkflow,
        context: &WorkflowExecutionContext,
    ) -> Result<WorkflowExecution, WorkflowExecutionError> {
        let (triggered, alert, evaluation_digest) = evaluate(&self.input)
            .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        let evidence = serde_json::json!({
            "triggered": triggered,
            "alert": alert,
            "evaluation_digest": evaluation_digest,
        });
        *self.result.lock().map_err(|_| {
            WorkflowExecutionError::Failed("alert evaluation lock poisoned".into())
        })? = Some((triggered, alert.clone(), evaluation_digest.clone()));
        Ok(WorkflowExecution {
            result_digest: evaluation_digest,
            output: serde_json::json!({
                "triggered": triggered,
                "alert_id": alert.as_ref().map(|alert| alert.alert_id.as_str()),
                "llm_judgement": false,
            }),
            evidence,
            events: vec![WorkflowExecutionEvent {
                kind: "verified_alert_evaluated".into(),
                source_uri: Some(self.input.window.source.uri.clone()),
                observed_at: context.command_timestamp,
                details: serde_json::json!({
                    "command_id": context.command_id,
                    "workflow_digest": context.workflow_digest,
                }),
            }],
        })
    }
}

/// Evaluate a deterministic rule exclusively through Command + Workflow.
pub fn evaluate_verified_alert(
    policy: VerifiedAlertPolicy,
    metric: AlertMetric,
    window: AlertTriggeringWindow,
    evaluated_at: impl Into<String>,
) -> Result<VerifiedAlertEvaluation, VerifiedAlertError> {
    let input = EvaluationInput {
        policy,
        metric,
        window,
        evaluated_at: evaluated_at.into(),
    };
    validate_input(&input)?;
    let policy_digest = digest(&input.policy)?;
    let workflow = verified_alert_evaluation_template(
        input.window.source.clone(),
        &policy_digest,
        &input.window.result_digest,
    );
    let workflow_digest = WorkflowDigest::new(
        workflow
            .stable_digest()
            .map_err(|error| VerifiedAlertError::Workflow(error.to_string()))?,
    );
    let envelope = CommandEnvelope::new(
        CommandOrigin::Ui,
        Command::RunWorkflow {
            workflow_id: workflow.id,
        },
    )
    .with_workflow_digest(workflow_digest.clone())
    .with_source_snapshot(input.window.source.clone())
    .with_input_snapshot(InputSnapshot::new(
        "alert-window",
        input.window.source.clone(),
    ));
    let command_id = envelope.id;
    let executor = EvaluationExecutor {
        input,
        result: Mutex::new(None),
    };
    let mut project = Project::new("Verified alert evaluation");
    let mut bus = CommandBus::new(project.clone());
    bus.register_workflow(workflow)
        .map_err(|error| VerifiedAlertError::Workflow(error.to_string()))?;
    let execution = bus
        .apply_with_executor(&mut project, envelope, &executor)
        .map_err(|error| VerifiedAlertError::Workflow(error.to_string()))?;
    let (triggered, alert, evaluation_digest) = executor
        .result
        .into_inner()
        .map_err(|_| VerifiedAlertError::Workflow("evaluation lock poisoned".into()))?
        .ok_or_else(|| VerifiedAlertError::Workflow("executor returned no evaluation".into()))?;
    if execution.result_digest.as_deref() != Some(evaluation_digest.as_str()) {
        return Err(VerifiedAlertError::Workflow(
            "CommandBus and evaluation digests differ".into(),
        ));
    }
    Ok(VerifiedAlertEvaluation {
        command_id: command_id.to_string(),
        workflow_digest,
        triggered,
        alert,
        evaluation_digest,
    })
}

struct AcknowledgementExecutor {
    alert: VerifiedAlertRecord,
    acknowledgement: AlertAcknowledgement,
    result: Mutex<Option<VerifiedAlertRecord>>,
}

impl WorkflowExecutor for AcknowledgementExecutor {
    fn execute(
        &self,
        _workflow: &GeoWorkflow,
        context: &WorkflowExecutionContext,
    ) -> Result<WorkflowExecution, WorkflowExecutionError> {
        let updated = append_acknowledgement(self.alert.clone(), self.acknowledgement.clone())
            .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        let result_digest = updated.record_digest.clone();
        let evidence = serde_json::to_value(&updated)
            .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        *self.result.lock().map_err(|_| {
            WorkflowExecutionError::Failed("acknowledgement lock poisoned".into())
        })? = Some(updated.clone());
        Ok(WorkflowExecution {
            result_digest,
            output: serde_json::json!({
                "alert_id": updated.alert_id,
                "record_digest": updated.record_digest,
                "acknowledgement_count": updated.acknowledgements.len(),
            }),
            evidence,
            events: vec![WorkflowExecutionEvent {
                kind: "verified_alert_acknowledged".into(),
                source_uri: Some(format!("alert://{}", updated.alert_id)),
                observed_at: context.command_timestamp,
                details: serde_json::json!({
                    "command_id": context.command_id,
                    "workflow_digest": context.workflow_digest,
                }),
            }],
        })
    }
}

/// Append one acknowledgement exclusively through Command + Workflow.
pub fn acknowledge_verified_alert(
    alert: VerifiedAlertRecord,
    acknowledgement: AlertAcknowledgement,
) -> Result<AlertAcknowledgementReceipt, VerifiedAlertError> {
    verify_alert_record(&alert)?;
    let mut source = SourceSnapshot::new(format!("alert://{}", alert.alert_id));
    source.checksum = Some(alert.record_digest.clone());
    source.observed_checksum = Some(alert.record_digest.clone());
    source.checksum_status = ChecksumVerification::Verified;
    let workflow = alert_acknowledgement_template(
        source.clone(),
        &alert.record_digest,
        alert.acknowledgements.len() as u32 + 1,
    );
    let workflow_digest = WorkflowDigest::new(
        workflow
            .stable_digest()
            .map_err(|error| VerifiedAlertError::Workflow(error.to_string()))?,
    );
    let envelope = CommandEnvelope::new(
        CommandOrigin::Ui,
        Command::RunWorkflow {
            workflow_id: workflow.id,
        },
    )
    .with_workflow_digest(workflow_digest.clone())
    .with_source_snapshot(source.clone())
    .with_input_snapshot(InputSnapshot::new("verified-alert", source));
    let command_id = envelope.id;
    let executor = AcknowledgementExecutor {
        alert,
        acknowledgement,
        result: Mutex::new(None),
    };
    let mut project = Project::new("Alert acknowledgement");
    let mut bus = CommandBus::new(project.clone());
    bus.register_workflow(workflow)
        .map_err(|error| VerifiedAlertError::Workflow(error.to_string()))?;
    let execution = bus
        .apply_with_executor(&mut project, envelope, &executor)
        .map_err(|error| VerifiedAlertError::Workflow(error.to_string()))?;
    let alert = executor
        .result
        .into_inner()
        .map_err(|_| VerifiedAlertError::Workflow("acknowledgement lock poisoned".into()))?
        .ok_or_else(|| VerifiedAlertError::Workflow("executor returned no alert".into()))?;
    if execution.result_digest.as_deref() != Some(alert.record_digest.as_str()) {
        return Err(VerifiedAlertError::Workflow(
            "CommandBus and acknowledged alert digests differ".into(),
        ));
    }
    Ok(AlertAcknowledgementReceipt {
        command_id: command_id.to_string(),
        workflow_digest,
        alert,
    })
}

/// Independently verify trigger identity, policy, checks, and acknowledgement chain.
pub fn verify_alert_record(alert: &VerifiedAlertRecord) -> Result<(), VerifiedAlertError> {
    if alert.schema_version != VERIFIED_ALERT_SCHEMA_VERSION
        || alert.checks.is_empty()
        || alert.checks.iter().any(|check| !check.passed)
        || alert.verifier != "genegis-deterministic-alert-verifier/1"
        || alert.policy_digest != digest(&alert.policy)?
    {
        return Err(VerifiedAlertError::Invalid(
            "schema, policy, verifier, or checks are invalid".into(),
        ));
    }
    validate_window(&alert.triggering_window)?;
    let expected_alert_id = trigger_digest(alert)?;
    if alert.alert_id != expected_alert_id || alert.record_digest != record_digest(alert)? {
        return Err(VerifiedAlertError::Invalid(
            "alert or record digest mismatch".into(),
        ));
    }
    let mut ids = BTreeSet::new();
    let mut previous = DateTime::parse_from_rfc3339(&alert.triggered_at)
        .map_err(|_| VerifiedAlertError::Invalid("invalid trigger time".into()))?;
    for acknowledgement in &alert.acknowledgements {
        let at = DateTime::parse_from_rfc3339(&acknowledgement.acknowledged_at)
            .map_err(|_| VerifiedAlertError::Invalid("invalid acknowledgement time".into()))?;
        if acknowledgement.id.trim().is_empty()
            || acknowledgement.actor.trim().is_empty()
            || !ids.insert(acknowledgement.id.as_str())
            || at < previous
        {
            return Err(VerifiedAlertError::Invalid(
                "acknowledgement chain is invalid".into(),
            ));
        }
        require_digest(&acknowledgement.note_digest, "note")?;
        previous = at;
    }
    Ok(())
}

fn evaluate(
    input: &EvaluationInput,
) -> Result<(bool, Option<VerifiedAlertRecord>, String), VerifiedAlertError> {
    validate_input(input)?;
    let (triggered, evaluation_value, rule_detail) = match &input.policy.rule {
        VerifiedAlertRule::Threshold {
            comparison,
            threshold,
            ..
        } => {
            let triggered = match comparison {
                AlertComparison::GreaterThan => input.metric.value > *threshold,
                AlertComparison::GreaterThanOrEqual => input.metric.value >= *threshold,
                AlertComparison::LessThan => input.metric.value < *threshold,
                AlertComparison::LessThanOrEqual => input.metric.value <= *threshold,
            };
            (
                triggered,
                input.metric.value,
                format!("metric compared with {threshold}"),
            )
        }
        VerifiedAlertRule::ZScoreAnomaly {
            mean,
            standard_deviation,
            absolute_z_threshold,
            ..
        } => {
            let score = ((input.metric.value - mean) / standard_deviation).abs();
            (
                score >= *absolute_z_threshold,
                score,
                format!("absolute z-score {score} compared with {absolute_z_threshold}"),
            )
        }
    };
    let checks = vec![
        AlertVerificationCheck {
            name: "deterministic_rule".into(),
            passed: true,
            detail: "closed numeric threshold/z-score rule; no LLM judgement".into(),
        },
        AlertVerificationCheck {
            name: "triggering_window".into(),
            passed: true,
            detail: format!(
                "cursor {}..{}; {} snapshots",
                input.window.cursor_start,
                input.window.cursor_end,
                input.window.snapshot_digests.len()
            ),
        },
        AlertVerificationCheck {
            name: "rule_evaluation".into(),
            passed: true,
            detail: rule_detail,
        },
    ];
    let policy_digest = digest(&input.policy)?;
    let mut alert = triggered.then(|| VerifiedAlertRecord {
        schema_version: VERIFIED_ALERT_SCHEMA_VERSION.into(),
        alert_id: String::new(),
        policy_digest,
        policy: input.policy.clone(),
        triggering_window: input.window.clone(),
        metric: input.metric.clone(),
        evaluation_value,
        verifier: "genegis-deterministic-alert-verifier/1".into(),
        checks,
        triggered_at: input.evaluated_at.clone(),
        acknowledgements: vec![],
        record_digest: String::new(),
    });
    if let Some(alert) = &mut alert {
        alert.alert_id = trigger_digest(alert)?;
        alert.record_digest = record_digest(alert)?;
        verify_alert_record(alert)?;
    }
    let evaluation_digest = digest(&serde_json::json!({
        "policy": input.policy,
        "metric": input.metric,
        "window": input.window,
        "evaluated_at": input.evaluated_at,
        "triggered": triggered,
        "alert": alert,
    }))?;
    Ok((triggered, alert, evaluation_digest))
}

fn append_acknowledgement(
    mut alert: VerifiedAlertRecord,
    acknowledgement: AlertAcknowledgement,
) -> Result<VerifiedAlertRecord, VerifiedAlertError> {
    verify_alert_record(&alert)?;
    alert.acknowledgements.push(acknowledgement);
    alert.record_digest = record_digest(&alert)?;
    verify_alert_record(&alert)?;
    Ok(alert)
}

fn validate_input(input: &EvaluationInput) -> Result<(), VerifiedAlertError> {
    validate_window(&input.window)?;
    DateTime::parse_from_rfc3339(&input.evaluated_at)
        .map_err(|_| VerifiedAlertError::Invalid("invalid evaluation time".into()))?;
    if input.policy.id.trim().is_empty()
        || input.policy.severity.trim().is_empty()
        || !input.metric.value.is_finite()
    {
        return Err(VerifiedAlertError::Invalid(
            "policy identity, severity, or metric is invalid".into(),
        ));
    }
    if input.policy.require_fresh && !input.window.fresh {
        return Err(VerifiedAlertError::Invalid(
            "policy requires a fresh triggering window".into(),
        ));
    }
    let (field, unit) = match &input.policy.rule {
        VerifiedAlertRule::Threshold {
            field,
            unit,
            threshold,
            ..
        } => {
            if !threshold.is_finite() {
                return Err(VerifiedAlertError::Invalid(
                    "threshold is not finite".into(),
                ));
            }
            (field, unit)
        }
        VerifiedAlertRule::ZScoreAnomaly {
            field,
            unit,
            baseline_digest,
            mean,
            standard_deviation,
            absolute_z_threshold,
        } => {
            require_digest(baseline_digest, "baseline")?;
            if !mean.is_finite()
                || !standard_deviation.is_finite()
                || *standard_deviation <= 0.0
                || !absolute_z_threshold.is_finite()
                || *absolute_z_threshold <= 0.0
            {
                return Err(VerifiedAlertError::Invalid(
                    "z-score baseline parameters are invalid".into(),
                ));
            }
            (field, unit)
        }
    };
    if field != &input.metric.field || unit != &input.metric.unit {
        return Err(VerifiedAlertError::Invalid(
            "policy and metric field/unit differ".into(),
        ));
    }
    Ok(())
}

fn validate_window(window: &AlertTriggeringWindow) -> Result<(), VerifiedAlertError> {
    require_digest(&window.result_digest, "result")?;
    if window.cursor_end < window.cursor_start
        || window.snapshot_digests.is_empty()
        || !window.source.checksum_status.is_verified()
    {
        return Err(VerifiedAlertError::Invalid(
            "triggering window cursor, snapshots, or source is invalid".into(),
        ));
    }
    let start = DateTime::parse_from_rfc3339(&window.watermark_start)
        .map_err(|_| VerifiedAlertError::Invalid("invalid start watermark".into()))?;
    let end = DateTime::parse_from_rfc3339(&window.watermark_end)
        .map_err(|_| VerifiedAlertError::Invalid("invalid end watermark".into()))?;
    if end < start {
        return Err(VerifiedAlertError::Invalid("watermark regressed".into()));
    }
    for snapshot in &window.snapshot_digests {
        require_digest(snapshot, "snapshot")?;
    }
    Ok(())
}

fn trigger_digest(alert: &VerifiedAlertRecord) -> Result<String, serde_json::Error> {
    digest(&serde_json::json!({
        "schema_version": alert.schema_version,
        "policy_digest": alert.policy_digest,
        "policy": alert.policy,
        "triggering_window": alert.triggering_window,
        "metric": alert.metric,
        "evaluation_value": alert.evaluation_value,
        "verifier": alert.verifier,
        "checks": alert.checks,
        "triggered_at": alert.triggered_at,
    }))
}

fn record_digest(alert: &VerifiedAlertRecord) -> Result<String, serde_json::Error> {
    digest(&serde_json::json!({
        "alert_id": alert.alert_id,
        "acknowledgements": alert.acknowledgements,
    }))
}

fn require_digest(value: &str, label: &str) -> Result<(), VerifiedAlertError> {
    if value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        Ok(())
    } else {
        Err(VerifiedAlertError::Invalid(format!(
            "invalid {label} digest"
        )))
    }
}

fn digest(value: &impl Serialize) -> Result<String, serde_json::Error> {
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(value)?)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(value: &str) -> String {
        format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
    }

    fn window(fresh: bool) -> AlertTriggeringWindow {
        let response = hash("response");
        let mut source = SourceSnapshot::new("fixture://hazard");
        source.checksum = Some(response.clone());
        source.observed_checksum = Some(response);
        source.checksum_status = ChecksumVerification::Verified;
        AlertTriggeringWindow {
            cursor_start: 10,
            cursor_end: 11,
            watermark_start: "2026-08-26T10:00:00Z".into(),
            watermark_end: "2026-08-26T10:05:00Z".into(),
            fresh,
            snapshot_digests: vec![hash("observation-11")],
            result_digest: hash("hazard-result"),
            source,
        }
    }

    #[test]
    fn threshold_and_anomaly_alerts_are_deterministic_and_receipted() {
        let threshold = evaluate_verified_alert(
            VerifiedAlertPolicy {
                id: "flood-depth".into(),
                rule: VerifiedAlertRule::Threshold {
                    field: "depth".into(),
                    unit: "metres".into(),
                    comparison: AlertComparison::GreaterThanOrEqual,
                    threshold: 0.5,
                },
                severity: "warning".into(),
                require_fresh: true,
            },
            AlertMetric {
                field: "depth".into(),
                value: 0.7,
                unit: "metres".into(),
            },
            window(true),
            "2026-08-26T10:06:00Z",
        )
        .expect("threshold");
        assert!(threshold.triggered);
        assert!(threshold
            .alert
            .as_ref()
            .expect("alert")
            .checks
            .iter()
            .all(|check| check.passed));

        let anomaly = evaluate_verified_alert(
            VerifiedAlertPolicy {
                id: "sensor-anomaly".into(),
                rule: VerifiedAlertRule::ZScoreAnomaly {
                    field: "pm25".into(),
                    unit: "ug/m3".into(),
                    baseline_digest: hash("baseline"),
                    mean: 10.0,
                    standard_deviation: 2.0,
                    absolute_z_threshold: 3.0,
                },
                severity: "advisory".into(),
                require_fresh: true,
            },
            AlertMetric {
                field: "pm25".into(),
                value: 17.0,
                unit: "ug/m3".into(),
            },
            window(true),
            "2026-08-26T10:06:00Z",
        )
        .expect("anomaly");
        assert!(anomaly.triggered);
        assert_eq!(anomaly.alert.expect("alert").evaluation_value, 3.5);
    }

    #[test]
    fn acknowledgement_is_append_only_and_stale_or_tampered_alerts_fail() {
        let evaluation = evaluate_verified_alert(
            VerifiedAlertPolicy {
                id: "depth".into(),
                rule: VerifiedAlertRule::Threshold {
                    field: "depth".into(),
                    unit: "metres".into(),
                    comparison: AlertComparison::GreaterThan,
                    threshold: 0.2,
                },
                severity: "warning".into(),
                require_fresh: true,
            },
            AlertMetric {
                field: "depth".into(),
                value: 0.4,
                unit: "metres".into(),
            },
            window(true),
            "2026-08-26T10:06:00Z",
        )
        .expect("evaluation");
        let original = evaluation.alert.expect("alert");
        let receipt = acknowledge_verified_alert(
            original.clone(),
            AlertAcknowledgement {
                id: "ack-1".into(),
                actor: "operator-a".into(),
                acknowledged_at: "2026-08-26T10:07:00Z".into(),
                note_digest: hash("investigating"),
            },
        )
        .expect("acknowledge");
        assert_eq!(receipt.alert.alert_id, original.alert_id);
        assert_ne!(receipt.alert.record_digest, original.record_digest);
        assert_eq!(receipt.alert.acknowledgements.len(), 1);

        let mut tampered = receipt.alert;
        tampered.metric.value = 99.0;
        assert!(verify_alert_record(&tampered).is_err());

        let stale = evaluate_verified_alert(
            original.policy,
            original.metric,
            window(false),
            "2026-08-26T10:06:00Z",
        );
        assert!(stale.is_err());
    }
}
