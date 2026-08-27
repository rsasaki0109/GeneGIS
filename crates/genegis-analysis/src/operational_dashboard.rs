//! Versioned operational map/dashboard views over live incremental results.

use std::{collections::BTreeSet, sync::Mutex};

use chrono::DateTime;
use genegis_core::{
    Command, CommandBus, CommandEnvelope, CommandOrigin, InputSnapshot, Project, WorkflowDigest,
    WorkflowExecution, WorkflowExecutionContext, WorkflowExecutionError, WorkflowExecutionEvent,
    WorkflowExecutor,
};
use genegis_crs::SourceSnapshot;
use genegis_workflow::{operational_dashboard_view_template, GeoWorkflow};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::DashboardWidget;

/// Operational dashboard view schema version.
pub const OPERATIONAL_DASHBOARD_SCHEMA_VERSION: &str = "0.1.0";

/// One digest-bound operational map layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationalMapLayer {
    /// Stable layer identity.
    pub id: String,
    /// Result rendered by this layer.
    pub result_digest: String,
    /// Exact portrayal identity.
    pub style_digest: String,
    /// Current visibility.
    pub visible: bool,
    /// Opacity in the inclusive range 0–1.
    pub opacity: f64,
}

/// Cursor/watermark and incremental scheduler state shown beside the map.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationalStatus {
    /// Committed feed cursor.
    pub cursor: u64,
    /// Committed event-time watermark.
    pub watermark: String,
    /// Whether the underlying feed was fresh at evaluation.
    pub fresh: bool,
    /// Exact incremental scheduler state.
    pub incremental_state_digest: String,
    /// Exact feed response source.
    pub feed_source: SourceSnapshot,
}

/// Alert history entry displayed in an operational view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationalAlertHistoryEntry {
    /// Stable alert identity.
    pub alert_digest: String,
    /// Triggering result identity.
    pub triggering_result_digest: String,
    /// RFC 3339 trigger time.
    pub triggered_at: String,
    /// Open, acknowledged, or resolved state.
    pub state: String,
}

/// Unsealed next operational view version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationalDashboardDraft {
    /// Exact live/incremental result shown by all components.
    pub result_digest: String,
    /// Map layers linked to the result.
    pub map_layers: Vec<OperationalMapLayer>,
    /// KPI/chart widgets linked to the result.
    pub widgets: Vec<DashboardWidget>,
    /// Feed and incremental status.
    pub status: OperationalStatus,
    /// Append-only alert history.
    pub alert_history: Vec<OperationalAlertHistoryEntry>,
    /// Previous view used to prove monotone version/cursor/history updates.
    pub previous: Option<Box<OperationalDashboardView>>,
}

/// Sealed one-version view of map, charts, status, and alert history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationalDashboardView {
    /// View schema version.
    pub schema_version: String,
    /// Monotone view version.
    pub version: u64,
    /// Previous view identity, absent only for version one.
    pub previous_view_digest: Option<String>,
    /// Exact live/incremental result shown by all components.
    pub result_digest: String,
    /// Digest-bound map layers.
    pub map_layers: Vec<OperationalMapLayer>,
    /// Linked KPI/chart widgets.
    pub widgets: Vec<DashboardWidget>,
    /// Feed and incremental status.
    pub status: OperationalStatus,
    /// Append-only alert history.
    pub alert_history: Vec<OperationalAlertHistoryEntry>,
    /// Canonical complete view identity.
    pub view_digest: String,
}

/// Command + Workflow composition receipt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationalDashboardReceipt {
    /// Applied command identity.
    pub command_id: String,
    /// Exact operational view workflow identity.
    pub workflow_digest: WorkflowDigest,
    /// Sealed view.
    pub view: OperationalDashboardView,
}

/// Fail-closed operational view error.
#[derive(Debug, Error)]
pub enum OperationalDashboardError {
    /// View identity/linkage/version semantics are invalid.
    #[error("invalid operational dashboard: {0}")]
    Invalid(String),
    /// Canonical serialization failed.
    #[error("operational dashboard serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    /// Command or workflow execution failed.
    #[error("operational dashboard workflow failed: {0}")]
    Workflow(String),
}

/// Seal a draft after checking all links and monotone update rules.
pub fn seal_operational_dashboard(
    draft: OperationalDashboardDraft,
) -> Result<OperationalDashboardView, OperationalDashboardError> {
    validate_draft(&draft)?;
    let (version, previous_view_digest) = draft.previous.as_ref().map_or((1, None), |previous| {
        (previous.version + 1, Some(previous.view_digest.clone()))
    });
    let mut view = OperationalDashboardView {
        schema_version: OPERATIONAL_DASHBOARD_SCHEMA_VERSION.into(),
        version,
        previous_view_digest,
        result_digest: draft.result_digest,
        map_layers: draft.map_layers,
        widgets: draft.widgets,
        status: draft.status,
        alert_history: draft.alert_history,
        view_digest: String::new(),
    };
    view.view_digest = view_digest(&view)?;
    verify_operational_dashboard(&view)?;
    Ok(view)
}

/// Verify one sealed view independently of prior storage.
pub fn verify_operational_dashboard(
    view: &OperationalDashboardView,
) -> Result<(), OperationalDashboardError> {
    if view.schema_version != OPERATIONAL_DASHBOARD_SCHEMA_VERSION
        || view.version == 0
        || (view.version == 1) != view.previous_view_digest.is_none()
    {
        return Err(OperationalDashboardError::Invalid(
            "schema or version chain is invalid".into(),
        ));
    }
    validate_components(
        &view.result_digest,
        &view.map_layers,
        &view.widgets,
        &view.status,
        &view.alert_history,
    )?;
    if view.view_digest != view_digest(view)? {
        return Err(OperationalDashboardError::Invalid(
            "view digest mismatch".into(),
        ));
    }
    Ok(())
}

struct OperationalExecutor {
    draft: OperationalDashboardDraft,
    view: Mutex<Option<OperationalDashboardView>>,
}

impl WorkflowExecutor for OperationalExecutor {
    fn execute(
        &self,
        _workflow: &GeoWorkflow,
        context: &WorkflowExecutionContext,
    ) -> Result<WorkflowExecution, WorkflowExecutionError> {
        let view = seal_operational_dashboard(self.draft.clone())
            .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        let result_digest = view.view_digest.clone();
        let evidence = serde_json::to_value(&view)
            .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        let output = serde_json::json!({
            "view_digest": view.view_digest,
            "result_digest": view.result_digest,
            "version": view.version,
            "cursor": view.status.cursor,
            "watermark": view.status.watermark,
            "alert_count": view.alert_history.len(),
        });
        let source_uri = view.status.feed_source.uri.clone();
        *self.view.lock().map_err(|_| {
            WorkflowExecutionError::Failed("operational dashboard lock poisoned".into())
        })? = Some(view);
        Ok(WorkflowExecution {
            result_digest,
            output,
            evidence,
            events: vec![WorkflowExecutionEvent {
                kind: "operational_dashboard_version_committed".into(),
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

/// Compose the next operational view exclusively through Command + Workflow.
pub fn compose_operational_dashboard(
    draft: OperationalDashboardDraft,
) -> Result<OperationalDashboardReceipt, OperationalDashboardError> {
    validate_draft(&draft)?;
    let version = draft
        .previous
        .as_ref()
        .map_or(1, |previous| previous.version + 1);
    let workflow = operational_dashboard_view_template(
        draft.status.feed_source.clone(),
        &draft.result_digest,
        version,
    );
    let workflow_digest = WorkflowDigest::new(
        workflow
            .stable_digest()
            .map_err(|error| OperationalDashboardError::Workflow(error.to_string()))?,
    );
    let envelope = CommandEnvelope::new(
        CommandOrigin::Ui,
        Command::RunWorkflow {
            workflow_id: workflow.id,
        },
    )
    .with_workflow_digest(workflow_digest.clone())
    .with_source_snapshot(draft.status.feed_source.clone())
    .with_input_snapshot(InputSnapshot::new(
        "operational-result",
        draft.status.feed_source.clone(),
    ));
    let command_id = envelope.id;
    let executor = OperationalExecutor {
        draft,
        view: Mutex::new(None),
    };
    let mut project = Project::new("Operational dashboard view");
    let mut bus = CommandBus::new(project.clone());
    bus.register_workflow(workflow)
        .map_err(|error| OperationalDashboardError::Workflow(error.to_string()))?;
    let execution = bus
        .apply_with_executor(&mut project, envelope, &executor)
        .map_err(|error| OperationalDashboardError::Workflow(error.to_string()))?;
    let view = executor
        .view
        .into_inner()
        .map_err(|_| OperationalDashboardError::Workflow("view lock poisoned".into()))?
        .ok_or_else(|| OperationalDashboardError::Workflow("executor returned no view".into()))?;
    if execution.result_digest.as_deref() != Some(view.view_digest.as_str()) {
        return Err(OperationalDashboardError::Workflow(
            "CommandBus and operational view digests differ".into(),
        ));
    }
    Ok(OperationalDashboardReceipt {
        command_id: command_id.to_string(),
        workflow_digest,
        view,
    })
}

fn validate_draft(draft: &OperationalDashboardDraft) -> Result<(), OperationalDashboardError> {
    validate_components(
        &draft.result_digest,
        &draft.map_layers,
        &draft.widgets,
        &draft.status,
        &draft.alert_history,
    )?;
    if let Some(previous) = &draft.previous {
        verify_operational_dashboard(previous)?;
        let previous_watermark = DateTime::parse_from_rfc3339(&previous.status.watermark)
            .map_err(|_| OperationalDashboardError::Invalid("invalid previous watermark".into()))?;
        let next_watermark = DateTime::parse_from_rfc3339(&draft.status.watermark)
            .map_err(|_| OperationalDashboardError::Invalid("invalid next watermark".into()))?;
        if draft.status.cursor < previous.status.cursor
            || next_watermark < previous_watermark
            || !draft.alert_history.starts_with(&previous.alert_history)
            || draft.alert_history[previous.alert_history.len()..]
                .iter()
                .any(|alert| alert.triggering_result_digest != draft.result_digest)
        {
            return Err(OperationalDashboardError::Invalid(
                "cursor, watermark, or alert history regressed".into(),
            ));
        }
    }
    Ok(())
}

fn validate_components(
    result_digest: &str,
    layers: &[OperationalMapLayer],
    widgets: &[DashboardWidget],
    status: &OperationalStatus,
    alerts: &[OperationalAlertHistoryEntry],
) -> Result<(), OperationalDashboardError> {
    require_digest(result_digest, "result")?;
    require_digest(&status.incremental_state_digest, "incremental state")?;
    DateTime::parse_from_rfc3339(&status.watermark)
        .map_err(|_| OperationalDashboardError::Invalid("invalid watermark".into()))?;
    if layers.is_empty() || widgets.is_empty() || !status.feed_source.checksum_status.is_verified()
    {
        return Err(OperationalDashboardError::Invalid(
            "map, widgets, and verified feed source are required".into(),
        ));
    }
    let mut layer_ids = BTreeSet::new();
    for layer in layers {
        if layer.id.trim().is_empty()
            || !layer_ids.insert(layer.id.as_str())
            || layer.result_digest != result_digest
            || !layer.opacity.is_finite()
            || !(0.0..=1.0).contains(&layer.opacity)
        {
            return Err(OperationalDashboardError::Invalid(
                "map layer identity, result, or opacity is invalid".into(),
            ));
        }
        require_digest(&layer.style_digest, "style")?;
    }
    let mut widget_ids = BTreeSet::new();
    for widget in widgets {
        let id = match widget {
            DashboardWidget::Kpi { id, .. }
            | DashboardWidget::Histogram { id, .. }
            | DashboardWidget::CategoryBreakdown { id, .. } => id,
        };
        if id.trim().is_empty() || !widget_ids.insert(id.as_str()) {
            return Err(OperationalDashboardError::Invalid(
                "widget ids must be non-empty and unique".into(),
            ));
        }
    }
    for alert in alerts {
        require_digest(&alert.alert_digest, "alert")?;
        require_digest(&alert.triggering_result_digest, "alert triggering result")?;
        if !matches!(alert.state.as_str(), "open" | "acknowledged" | "resolved")
            || DateTime::parse_from_rfc3339(&alert.triggered_at).is_err()
        {
            return Err(OperationalDashboardError::Invalid(
                "alert history entry is not linked to this result".into(),
            ));
        }
    }
    Ok(())
}

fn require_digest(value: &str, label: &str) -> Result<(), OperationalDashboardError> {
    if value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        Ok(())
    } else {
        Err(OperationalDashboardError::Invalid(format!(
            "invalid {label} digest"
        )))
    }
}

fn view_digest(view: &OperationalDashboardView) -> Result<String, serde_json::Error> {
    let mut semantic = view.clone();
    semantic.view_digest.clear();
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(&semantic)?)
    ))
}

#[cfg(test)]
mod tests {
    use genegis_crs::ChecksumVerification;

    use super::*;

    fn hash(value: &str) -> String {
        format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
    }

    fn draft(previous: Option<OperationalDashboardView>, cursor: u64) -> OperationalDashboardDraft {
        let result_digest = hash(&format!("result-{cursor}"));
        let response_digest = hash(&format!("response-{cursor}"));
        let mut source = SourceSnapshot::new("fixture://live-sensor");
        source.checksum = Some(response_digest.clone());
        source.observed_checksum = Some(response_digest);
        source.checksum_status = ChecksumVerification::Verified;
        let mut history = previous
            .as_ref()
            .map(|view| view.alert_history.clone())
            .unwrap_or_default();
        history.push(OperationalAlertHistoryEntry {
            alert_digest: hash(&format!("alert-{cursor}")),
            triggering_result_digest: result_digest.clone(),
            triggered_at: "2026-08-26T10:01:00Z".into(),
            state: "open".into(),
        });
        OperationalDashboardDraft {
            result_digest: result_digest.clone(),
            map_layers: vec![OperationalMapLayer {
                id: "sensor-map".into(),
                result_digest: result_digest.clone(),
                style_digest: hash("sensor-style"),
                visible: true,
                opacity: 1.0,
            }],
            widgets: vec![DashboardWidget::Kpi {
                id: "sensor-count".into(),
                label: "Sensors".into(),
                value: cursor as f64,
                unit: "count".into(),
            }],
            status: OperationalStatus {
                cursor,
                watermark: format!("2026-08-26T10:{cursor:02}:00Z"),
                fresh: true,
                incremental_state_digest: hash(&format!("state-{cursor}")),
                feed_source: source,
            },
            alert_history: history,
            previous: previous.map(Box::new),
        }
    }

    #[test]
    fn versions_linked_map_widgets_status_and_alert_history_through_workflow() {
        let first = compose_operational_dashboard(draft(None, 1)).expect("v1");
        let second = compose_operational_dashboard(draft(Some(first.view.clone()), 2)).expect("v2");
        assert_eq!(second.view.version, 2);
        assert_eq!(
            second.view.previous_view_digest.as_deref(),
            Some(first.view.view_digest.as_str())
        );
        assert_eq!(second.view.alert_history.len(), 2);
        verify_operational_dashboard(&second.view).expect("verify");
        uuid::Uuid::parse_str(&second.command_id).expect("command id");
    }

    #[test]
    fn rejects_cursor_history_and_digest_tampering() {
        let first = seal_operational_dashboard(draft(None, 3)).expect("v1");
        let mut regressed = draft(Some(first.clone()), 2);
        regressed.alert_history.clear();
        assert!(seal_operational_dashboard(regressed).is_err());
        let mut tampered = first;
        tampered.widgets.clear();
        assert!(verify_operational_dashboard(&tampered).is_err());
    }
}
