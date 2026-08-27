//! Command + Workflow boundary for city-scale 2D/3D stream planning.

use std::sync::Mutex;

use genegis_core::{
    Command, CommandBus, CommandEnvelope, CommandOrigin, InputSnapshot, Project, WorkflowDigest,
    WorkflowExecution, WorkflowExecutionContext, WorkflowExecutionError, WorkflowExecutionEvent,
    WorkflowExecutor,
};
use genegis_crs::{ChecksumVerification, SourceSnapshot};
use genegis_render::{
    plan_city_scene_frame, CitySceneFramePlan, CitySceneManifest, CityStreamBudget,
    SharedSpatialViewState,
};
use genegis_workflow::{city_scene_stream_template, GeoWorkflow};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::AnalysisError;

/// Receipted city frame plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CitySceneWorkflowResult {
    /// Applied command identity.
    pub command_id: String,
    /// Exact stream-planning workflow identity.
    pub workflow_digest: WorkflowDigest,
    /// Sealed frame plan.
    pub plan: CitySceneFramePlan,
}

struct CitySceneExecutor {
    manifest: CitySceneManifest,
    view: SharedSpatialViewState,
    budget: CityStreamBudget,
    plan: Mutex<Option<CitySceneFramePlan>>,
}

impl WorkflowExecutor for CitySceneExecutor {
    fn execute(
        &self,
        _workflow: &GeoWorkflow,
        context: &WorkflowExecutionContext,
    ) -> Result<WorkflowExecution, WorkflowExecutionError> {
        let plan = plan_city_scene_frame(&self.manifest, self.view.clone(), self.budget.clone())
            .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        let result_digest = plan.plan_digest.clone();
        let evidence = serde_json::to_value(&plan)
            .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        *self
            .plan
            .lock()
            .map_err(|_| WorkflowExecutionError::Failed("city scene lock poisoned".into()))? =
            Some(plan.clone());
        Ok(WorkflowExecution {
            result_digest,
            output: serde_json::json!({
                "selected_tiles": plan.selected_tiles.len(),
                "deferred_tiles": plan.deferred_tiles.len(),
                "transfer_bytes": plan.transfer_bytes,
                "crs": plan.view.crs,
                "selected_features_present": plan.selected_features_present,
                "temporal_cursor": plan.view.temporal_cursor,
            }),
            evidence,
            events: vec![WorkflowExecutionEvent {
                kind: "city_scene_frame_planned".into(),
                source_uri: Some("manifest://city-scene".into()),
                observed_at: context.command_timestamp,
                details: serde_json::json!({
                    "command_id": context.command_id,
                    "workflow_digest": context.workflow_digest,
                }),
            }],
        })
    }
}

/// Plan a city frame exclusively through Command + Workflow Graph.
pub fn plan_city_scene_workflow(
    manifest: CitySceneManifest,
    view: SharedSpatialViewState,
    budget: CityStreamBudget,
) -> Result<CitySceneWorkflowResult, AnalysisError> {
    let manifest_bytes =
        serde_json::to_vec(&manifest).map_err(|error| AnalysisError::Message(error.to_string()))?;
    let manifest_digest = format!("sha256:{:x}", Sha256::digest(manifest_bytes));
    let mut source = SourceSnapshot::new("manifest://city-scene");
    source.checksum = Some(manifest_digest.clone());
    source.observed_checksum = Some(manifest_digest);
    source.checksum_status = ChecksumVerification::Verified;
    let workflow = city_scene_stream_template(source.clone());
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
    .with_input_snapshot(InputSnapshot::new("city-scene", source));
    let command_id = envelope.id;
    let executor = CitySceneExecutor {
        manifest,
        view,
        budget,
        plan: Mutex::new(None),
    };
    let mut project = Project::new("City scene frame plan");
    let mut bus = CommandBus::new(project.clone());
    bus.register_workflow(workflow)
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    let execution = bus
        .apply_with_executor(&mut project, envelope, &executor)
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    let plan = executor
        .plan
        .into_inner()
        .map_err(|_| AnalysisError::Message("city scene lock poisoned".into()))?
        .ok_or_else(|| AnalysisError::Message("executor returned no city plan".into()))?;
    if execution.result_digest.as_deref() != Some(plan.plan_digest.as_str()) {
        return Err(AnalysisError::Message(
            "CommandBus and city frame plan digests differ".into(),
        ));
    }
    Ok(CitySceneWorkflowResult {
        command_id: command_id.to_string(),
        workflow_digest,
        plan,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use genegis_crs::{ChecksumVerification, Crs};
    use genegis_render::{CityLayer, CityLayerKind, CityTile};

    use super::*;

    #[test]
    fn plans_city_frame_through_command_bus() {
        let content_digest = format!("sha256:{:x}", Sha256::digest(b"root"));
        let mut source = SourceSnapshot::new("fixture://city/buildings");
        source.checksum = Some(content_digest.clone());
        source.observed_checksum = Some(content_digest.clone());
        source.checksum_status = ChecksumVerification::Verified;
        let result = plan_city_scene_workflow(
            CitySceneManifest {
                schema_version: "0.1.0".into(),
                layers: vec![CityLayer {
                    id: "buildings".into(),
                    kind: CityLayerKind::Buildings3dTiles,
                    format: "3dtiles-1.1".into(),
                    source,
                }],
                tiles: vec![CityTile {
                    id: "root".into(),
                    layer_id: "buildings".into(),
                    parent_id: None,
                    geometric_error_m: 1.0,
                    camera_distance_m: 100.0,
                    content_bytes: 64,
                    content_digest,
                    feature_ids: vec!["building-1".into()],
                }],
            },
            SharedSpatialViewState {
                crs: Crs::nagoya_projected(),
                camera_position: [0.0, -100.0, 50.0],
                camera_target: [0.0; 3],
                field_of_view_degrees: 60.0,
                viewport_height_px: 720,
                selected_feature_ids: BTreeSet::from(["building-1".into()]),
                temporal_cursor: Some("2026-08-26T10:00:00Z".into()),
            },
            CityStreamBudget {
                maximum_tiles: 1,
                maximum_transfer_bytes: 64,
                maximum_screen_space_error: 16.0,
            },
        )
        .expect("command workflow plan");

        assert_eq!(result.plan.selected_tiles, vec!["root"]);
        assert_eq!(result.plan.selected_features_present, vec!["building-1"]);
        assert!(result.workflow_digest.as_str().starts_with("sha256:"));
    }
}
