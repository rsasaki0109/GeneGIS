//! Digest-bound narrative map project views executed through Command + Workflow.

use std::{collections::BTreeSet, sync::Mutex};

use genegis_core::{
    Command, CommandBus, CommandEnvelope, CommandOrigin, InputSnapshot, Project, WorkflowDigest,
    WorkflowExecution, WorkflowExecutionContext, WorkflowExecutionError, WorkflowExecutionEvent,
    WorkflowExecutor,
};
use genegis_crs::SourceSnapshot;
use genegis_workflow::{narrative_project_view_template, GeoWorkflow};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Narrative project-view schema version.
pub const NARRATIVE_VIEW_SCHEMA_VERSION: &str = "0.1.0";

/// One layer state restored when a narrative frame is selected.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NarrativeLayerState {
    /// Stable project layer identity.
    pub layer_id: String,
    /// Whether the layer is visible.
    pub visible: bool,
    /// Layer opacity in the inclusive range 0–1.
    pub opacity: f64,
    /// Exact analytical result used by the layer.
    pub result_digest: String,
    /// Exact style document identity.
    pub style_digest: String,
}

/// Restorable 2D/3D map camera and layer state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NarrativeMapState {
    /// WGS84 center as longitude, latitude.
    pub center: [f64; 2],
    /// Web-map zoom in the inclusive range 0–24.
    pub zoom: f64,
    /// Clockwise bearing in degrees.
    pub bearing: f64,
    /// Camera pitch in the inclusive range 0–85 degrees.
    pub pitch: f64,
    /// Optional ISO/RFC3339 temporal cursor or domain epoch identity.
    pub temporal_cursor: Option<String>,
    /// Ordered layer states.
    pub layers: Vec<NarrativeLayerState>,
}

/// Content-addressed media reference; bytes are not copied into the view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NarrativeMediaReference {
    /// Portable relative path or HTTPS URL.
    pub uri: String,
    /// Exact referenced-byte identity.
    pub content_digest: String,
    /// Declared media type.
    pub media_type: String,
    /// Accessible alternative text.
    pub alt_text: String,
}

/// Optional dashboard binding for one frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NarrativeDashboardReference {
    /// Dashboard document identity.
    pub dashboard_digest: String,
    /// Result identity consumed by the dashboard.
    pub result_digest: String,
}

/// One ordered narrative frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NarrativeFrame {
    /// Stable frame identity.
    pub id: String,
    /// User-facing heading.
    pub title: String,
    /// Narrative text stored as plain text.
    pub text: String,
    /// Restorable map state rather than a copied screenshot.
    pub map: NarrativeMapState,
    /// Content-addressed media references.
    pub media: Vec<NarrativeMediaReference>,
    /// Optional digest-bound dashboard.
    pub dashboard: Option<NarrativeDashboardReference>,
}

/// Unsealed narrative composition request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NarrativeProjectViewDraft {
    /// Human-readable view title.
    pub title: String,
    /// Exact verified result anchoring the view.
    pub result_digest: String,
    /// Exact source for the anchored result.
    pub result_source: SourceSnapshot,
    /// Ordered narrative frames.
    pub frames: Vec<NarrativeFrame>,
}

/// Sealed digest-bound narrative project view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NarrativeProjectView {
    /// View schema version.
    pub schema_version: String,
    /// Human-readable title.
    pub title: String,
    /// Verified result anchoring all frames.
    pub result_digest: String,
    /// Exact source for the anchored result.
    pub result_source: SourceSnapshot,
    /// Ordered frames.
    pub frames: Vec<NarrativeFrame>,
    /// Canonical semantic identity of this complete view.
    pub view_digest: String,
}

/// Command + Workflow composition result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NarrativeCompositionReceipt {
    /// Applied command identity.
    pub command_id: String,
    /// Exact composition workflow identity.
    pub workflow_digest: WorkflowDigest,
    /// Sealed project view.
    pub view: NarrativeProjectView,
}

/// Fail-closed narrative composition error.
#[derive(Debug, Error)]
pub enum NarrativeError {
    /// Draft or sealed view violates the narrative contract.
    #[error("invalid narrative project view: {0}")]
    Invalid(String),
    /// Canonical serialization failed.
    #[error("narrative serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    /// Command or workflow execution failed.
    #[error("narrative workflow failed: {0}")]
    Workflow(String),
}

/// Seal and verify a draft without executing it.
pub fn seal_narrative_project_view(
    draft: NarrativeProjectViewDraft,
) -> Result<NarrativeProjectView, NarrativeError> {
    validate_draft(&draft)?;
    let mut view = NarrativeProjectView {
        schema_version: NARRATIVE_VIEW_SCHEMA_VERSION.into(),
        title: draft.title,
        result_digest: draft.result_digest,
        result_source: draft.result_source,
        frames: draft.frames,
        view_digest: String::new(),
    };
    view.view_digest = canonical_view_digest(&view)?;
    verify_narrative_project_view(&view)?;
    Ok(view)
}

/// Recompute all semantic and digest constraints for a sealed view.
pub fn verify_narrative_project_view(view: &NarrativeProjectView) -> Result<(), NarrativeError> {
    if view.schema_version != NARRATIVE_VIEW_SCHEMA_VERSION {
        return Err(NarrativeError::Invalid("unsupported schema version".into()));
    }
    validate_draft(&NarrativeProjectViewDraft {
        title: view.title.clone(),
        result_digest: view.result_digest.clone(),
        result_source: view.result_source.clone(),
        frames: view.frames.clone(),
    })?;
    if canonical_view_digest(view)? != view.view_digest {
        return Err(NarrativeError::Invalid("view digest mismatch".into()));
    }
    Ok(())
}

struct NarrativeExecutor {
    draft: NarrativeProjectViewDraft,
    view: Mutex<Option<NarrativeProjectView>>,
}

impl WorkflowExecutor for NarrativeExecutor {
    fn execute(
        &self,
        _workflow: &GeoWorkflow,
        context: &WorkflowExecutionContext,
    ) -> Result<WorkflowExecution, WorkflowExecutionError> {
        let view = seal_narrative_project_view(self.draft.clone())
            .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        let result_digest = view.view_digest.clone();
        let evidence = serde_json::to_value(&view)
            .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        let output = serde_json::json!({
            "view_digest": view.view_digest,
            "subject_result_digest": view.result_digest,
            "frame_count": view.frames.len(),
            "copy_screenshots": false,
        });
        *self
            .view
            .lock()
            .map_err(|_| WorkflowExecutionError::Failed("narrative lock poisoned".into()))? =
            Some(view);
        Ok(WorkflowExecution {
            result_digest,
            output,
            evidence,
            events: vec![WorkflowExecutionEvent {
                kind: "narrative_project_view_composed".into(),
                source_uri: Some(self.draft.result_source.uri.clone()),
                observed_at: context.command_timestamp,
                details: serde_json::json!({
                    "command_id": context.command_id,
                    "workflow_digest": context.workflow_digest,
                }),
            }],
        })
    }
}

/// Compose a narrative view exclusively through Command + Workflow Graph.
pub fn compose_narrative_project_view(
    draft: NarrativeProjectViewDraft,
) -> Result<NarrativeCompositionReceipt, NarrativeError> {
    validate_draft(&draft)?;
    let workflow = narrative_project_view_template(
        draft.result_source.clone(),
        &draft.result_digest,
        draft.frames.len() as u32,
    );
    let workflow_digest = WorkflowDigest::new(
        workflow
            .stable_digest()
            .map_err(|error| NarrativeError::Workflow(error.to_string()))?,
    );
    let envelope = CommandEnvelope::new(
        CommandOrigin::Ui,
        Command::RunWorkflow {
            workflow_id: workflow.id,
        },
    )
    .with_workflow_digest(workflow_digest.clone())
    .with_source_snapshot(draft.result_source.clone())
    .with_input_snapshot(InputSnapshot::new(
        "verified-result",
        draft.result_source.clone(),
    ));
    let command_id = envelope.id;
    let executor = NarrativeExecutor {
        draft,
        view: Mutex::new(None),
    };
    let mut project = Project::new("Narrative project view");
    let mut bus = CommandBus::new(project.clone());
    bus.register_workflow(workflow)
        .map_err(|error| NarrativeError::Workflow(error.to_string()))?;
    let execution = bus
        .apply_with_executor(&mut project, envelope, &executor)
        .map_err(|error| NarrativeError::Workflow(error.to_string()))?;
    let view = executor
        .view
        .into_inner()
        .map_err(|_| NarrativeError::Workflow("narrative lock poisoned".into()))?
        .ok_or_else(|| NarrativeError::Workflow("executor returned no view".into()))?;
    if execution.result_digest.as_deref() != Some(view.view_digest.as_str()) {
        return Err(NarrativeError::Workflow(
            "CommandBus and narrative view digests differ".into(),
        ));
    }
    Ok(NarrativeCompositionReceipt {
        command_id: command_id.to_string(),
        workflow_digest,
        view,
    })
}

fn validate_draft(draft: &NarrativeProjectViewDraft) -> Result<(), NarrativeError> {
    if draft.title.trim().is_empty() || draft.frames.is_empty() {
        return Err(NarrativeError::Invalid(
            "title and at least one frame are required".into(),
        ));
    }
    require_digest(&draft.result_digest, "result digest")?;
    if draft.result_source.uri.trim().is_empty()
        || !draft.result_source.checksum_status.is_verified()
        || draft.result_source.observed_checksum.as_deref() != Some(draft.result_digest.as_str())
    {
        return Err(NarrativeError::Invalid(
            "result source must be checksum-verified against result digest".into(),
        ));
    }
    let mut ids = BTreeSet::new();
    for frame in &draft.frames {
        if frame.id.trim().is_empty()
            || !ids.insert(frame.id.as_str())
            || frame.title.trim().is_empty()
            || frame.text.trim().is_empty()
        {
            return Err(NarrativeError::Invalid(
                "frame ids must be unique and frame text must be non-empty".into(),
            ));
        }
        validate_map(&frame.map, &draft.result_digest)?;
        for media in &frame.media {
            require_digest(&media.content_digest, "media digest")?;
            if media.media_type.trim().is_empty()
                || media.alt_text.trim().is_empty()
                || !portable_media_uri(&media.uri)
            {
                return Err(NarrativeError::Invalid(
                    "media requires a safe URI, media type, digest, and alt text".into(),
                ));
            }
        }
        if let Some(dashboard) = &frame.dashboard {
            require_digest(&dashboard.dashboard_digest, "dashboard digest")?;
            if dashboard.result_digest != draft.result_digest {
                return Err(NarrativeError::Invalid(
                    "dashboard is bound to a different result".into(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_map(map: &NarrativeMapState, result_digest: &str) -> Result<(), NarrativeError> {
    let [longitude, latitude] = map.center;
    if !longitude.is_finite()
        || !(-180.0..=180.0).contains(&longitude)
        || !latitude.is_finite()
        || !(-90.0..=90.0).contains(&latitude)
        || !map.zoom.is_finite()
        || !(0.0..=24.0).contains(&map.zoom)
        || !map.bearing.is_finite()
        || !(-360.0..=360.0).contains(&map.bearing)
        || !map.pitch.is_finite()
        || !(0.0..=85.0).contains(&map.pitch)
    {
        return Err(NarrativeError::Invalid("invalid map camera".into()));
    }
    let mut layers = BTreeSet::new();
    for layer in &map.layers {
        if layer.layer_id.trim().is_empty()
            || !layers.insert(layer.layer_id.as_str())
            || !layer.opacity.is_finite()
            || !(0.0..=1.0).contains(&layer.opacity)
            || layer.result_digest != result_digest
        {
            return Err(NarrativeError::Invalid(
                "layer identity, opacity, or result binding is invalid".into(),
            ));
        }
        require_digest(&layer.style_digest, "style digest")?;
    }
    Ok(())
}

fn portable_media_uri(uri: &str) -> bool {
    let value = uri.trim();
    if value.starts_with("https://") {
        return true;
    }
    !value.is_empty()
        && !value.starts_with('/')
        && !value.starts_with('\\')
        && !value.contains(':')
        && !value.contains('\\')
        && !value.split('/').any(|component| component == "..")
}

fn require_digest(value: &str, label: &str) -> Result<(), NarrativeError> {
    let valid = value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()));
    if valid {
        Ok(())
    } else {
        Err(NarrativeError::Invalid(format!("invalid {label}")))
    }
}

fn canonical_view_digest(view: &NarrativeProjectView) -> Result<String, NarrativeError> {
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

    fn digest(value: &[u8]) -> String {
        format!("sha256:{:x}", Sha256::digest(value))
    }

    fn draft() -> NarrativeProjectViewDraft {
        let result_digest = digest(b"verified-result");
        let mut source = SourceSnapshot::new("capsule://verified-result");
        source.checksum = Some(result_digest.clone());
        source.observed_checksum = Some(result_digest.clone());
        source.checksum_status = ChecksumVerification::Verified;
        NarrativeProjectViewDraft {
            title: "名古屋の人口密度".into(),
            result_digest: result_digest.clone(),
            result_source: source,
            frames: vec![NarrativeFrame {
                id: "overview".into(),
                title: "全市の分布".into(),
                text: "検証済み人口密度の全市分布です。".into(),
                map: NarrativeMapState {
                    center: [136.9066, 35.1815],
                    zoom: 10.5,
                    bearing: 0.0,
                    pitch: 20.0,
                    temporal_cursor: None,
                    layers: vec![NarrativeLayerState {
                        layer_id: "density".into(),
                        visible: true,
                        opacity: 0.9,
                        result_digest: result_digest.clone(),
                        style_digest: digest(b"density-style"),
                    }],
                },
                media: vec![NarrativeMediaReference {
                    uri: "media/method.svg".into(),
                    content_digest: digest(b"method-svg"),
                    media_type: "image/svg+xml".into(),
                    alt_text: "人口密度の算出方法".into(),
                }],
                dashboard: Some(NarrativeDashboardReference {
                    dashboard_digest: digest(b"dashboard"),
                    result_digest,
                }),
            }],
        }
    }

    #[test]
    fn composes_digest_bound_view_through_command_workflow() {
        let receipt = compose_narrative_project_view(draft()).expect("compose");
        uuid::Uuid::parse_str(&receipt.command_id).expect("command id");
        assert!(receipt.workflow_digest.as_str().starts_with("sha256:"));
        verify_narrative_project_view(&receipt.view).expect("verify");
        assert!(receipt.view.view_digest.starts_with("sha256:"));
    }

    #[test]
    fn rejects_tampering_local_media_and_cross_result_dashboard() {
        let mut view = seal_narrative_project_view(draft()).expect("seal");
        view.frames[0].text.push_str("tampered");
        assert!(verify_narrative_project_view(&view).is_err());

        let mut invalid = draft();
        invalid.frames[0].media[0].uri = "C:\\private\\shot.png".into();
        assert!(seal_narrative_project_view(invalid).is_err());

        let mut invalid = draft();
        invalid.frames[0]
            .dashboard
            .as_mut()
            .expect("dashboard")
            .result_digest = digest(b"other-result");
        assert!(seal_narrative_project_view(invalid).is_err());
    }
}
