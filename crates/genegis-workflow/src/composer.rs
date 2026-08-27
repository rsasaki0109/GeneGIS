//! Typed, undoable no-code edits over the same `GeoWorkflow` execution IR.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    copc_change_detect_template, dashboard_export_template, external_stac_fetch_template,
    geocoding_template, local_cog_metadata_template, nagoya_evacuation_template,
    nagoya_flood_exposure_template, nagoya_geoparquet_density_template, nagoya_geoparquet_template,
    nagoya_population_density_template, nagoya_xmin_city_template,
    sentinel_ndvi_timeseries_template, GeoWorkflow, ReviewStatus, WorkflowDataRef, WorkflowNodeId,
    WorkflowValidationError,
};
use genegis_crs::SourceSnapshot;

/// User-facing analytical families covered by reviewed templates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisTemplateCategory {
    /// Population or event density.
    Density,
    /// Hazard or asset exposure.
    Exposure,
    /// Travel-time or network accessibility.
    Accessibility,
    /// Temporal or point-cloud change.
    Change,
    /// Multi-constraint site or route suitability.
    Suitability,
    /// Distance, nearest, or service-area proximity.
    Proximity,
    /// Spatial or zonal aggregation.
    Aggregation,
    /// Place/address to WGS84 candidate resolution.
    Geocoding,
    /// Data access, inspection, dashboard, or catalog utility.
    Utility,
}

/// Metadata for one reviewed workflow available to the no-code UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedWorkflowTemplate {
    /// Stable catalog identity.
    pub id: String,
    /// User-facing goal.
    pub title: String,
    /// Number of typed graph nodes.
    pub node_count: usize,
    /// Stable digest of the reviewed graph.
    pub workflow_digest: String,
    /// Analytical outcomes this template covers.
    pub categories: Vec<AnalysisTemplateCategory>,
    /// Receipt/evidence profile required from its executor.
    pub verification_profile: String,
}

/// One typed no-code action. UI callbacks serialize this enum before mutation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComposerCommand {
    /// Change the user-facing workflow goal.
    SetGoal { goal: String },
    /// Add a reviewed operation node copied from a template palette.
    AddReviewedNode {
        template_id: String,
        source_node_id: String,
        new_node_id: String,
    },
    /// Remove a node and all incoming/outgoing references.
    RemoveNode { node_id: String },
    /// Connect one exported source port to a target node.
    Connect {
        source_node_id: String,
        source_port: String,
        target_node_id: String,
    },
    /// Remove the source-to-target dependency and data reference.
    Disconnect {
        source_node_id: String,
        target_node_id: String,
    },
    /// Replace one node's JSON parameters.
    SetParameters {
        node_id: String,
        parameters: serde_json::Value,
    },
    /// Undo the last successful edit.
    Undo,
    /// Redo the last undone edit.
    Redo,
}

/// Auditable action record retained by the composer session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComposerEvent {
    /// Unique UI action identity.
    pub id: Uuid,
    /// Typed action applied.
    pub command: ComposerCommand,
    /// Stable digest when the resulting draft is executable, otherwise absent.
    pub executable_digest: Option<String>,
}

/// No-code draft plus reversible state and typed event history.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowComposer {
    /// Current graph draft.
    pub workflow: GeoWorkflow,
    /// Append-only typed UI events.
    pub events: Vec<ComposerEvent>,
    #[serde(skip)]
    undo: Vec<GeoWorkflow>,
    #[serde(skip)]
    redo: Vec<GeoWorkflow>,
}

/// Fail-closed no-code edit or execution-admission error.
#[derive(Debug, Error)]
pub enum ComposerError {
    /// Unknown reviewed template.
    #[error("unknown reviewed workflow template {0:?}")]
    UnknownTemplate(String),
    /// Node or output port does not exist.
    #[error("composer target not found: {0}")]
    NotFound(String),
    /// Node identifier is blank, unsafe, or already present.
    #[error("invalid composer node id: {0}")]
    InvalidNodeId(String),
    /// Undo or redo stack is empty.
    #[error("composer history has no {0} state")]
    EmptyHistory(&'static str),
    /// Draft failed graph or semantic validation before execution.
    #[error("composer draft is not executable: {0}")]
    InvalidWorkflow(#[from] WorkflowValidationError),
    /// Stable workflow digest generation failed.
    #[error("composer digest failed: {0}")]
    Digest(String),
}

impl WorkflowComposer {
    /// Create a draft from a runtime-resolved reviewed graph with immutable sources.
    pub fn from_reviewed_workflow(mut workflow: GeoWorkflow) -> Result<Self, ComposerError> {
        workflow.validate()?;
        workflow
            .stable_digest()
            .map_err(|error| ComposerError::Digest(error.to_string()))?;
        workflow.review_status = ReviewStatus::Draft;
        Ok(Self {
            workflow,
            events: vec![],
            undo: vec![],
            redo: vec![],
        })
    }

    /// Create a no-code draft from a reviewed, already-valid template.
    pub fn from_template(template_id: &str) -> Result<Self, ComposerError> {
        Self::from_reviewed_workflow(template_workflow(template_id)?)
    }

    /// Apply one typed UI action and retain reversible state.
    pub fn apply(&mut self, command: ComposerCommand) -> Result<ComposerEvent, ComposerError> {
        match command.clone() {
            ComposerCommand::Undo => {
                let previous = self.undo.pop().ok_or(ComposerError::EmptyHistory("undo"))?;
                self.redo.push(self.workflow.clone());
                self.workflow = previous;
            }
            ComposerCommand::Redo => {
                let next = self.redo.pop().ok_or(ComposerError::EmptyHistory("redo"))?;
                self.undo.push(self.workflow.clone());
                self.workflow = next;
            }
            edit => {
                let before = self.workflow.clone();
                apply_edit(&mut self.workflow, edit)?;
                self.undo.push(before);
                self.redo.clear();
            }
        }
        let executable_digest = self
            .workflow
            .validate()
            .ok()
            .and_then(|_| self.workflow.stable_digest().ok());
        let event = ComposerEvent {
            id: Uuid::new_v4(),
            command,
            executable_digest,
        };
        self.events.push(event.clone());
        Ok(event)
    }

    /// Validate every CRS/unit/source/schema/edge contract and return a run-ready graph.
    pub fn workflow_for_execution(&self) -> Result<GeoWorkflow, ComposerError> {
        self.workflow.validate()?;
        self.workflow
            .stable_digest()
            .map_err(|error| ComposerError::Digest(error.to_string()))?;
        Ok(self.workflow.clone())
    }
}

/// Return the reviewed template palette with stable graph identities.
pub fn reviewed_workflow_templates() -> Vec<ReviewedWorkflowTemplate> {
    template_ids()
        .iter()
        .filter_map(|id| template_workflow(id).ok().map(|workflow| (*id, workflow)))
        .map(|(id, workflow)| ReviewedWorkflowTemplate {
            id: id.into(),
            title: workflow.goal.clone(),
            node_count: workflow.steps.len(),
            workflow_digest: workflow.stable_digest().expect("reviewed template digest"),
            categories: template_categories(id),
            verification_profile: template_verification_profile(id).into(),
        })
        .collect()
}

fn apply_edit(workflow: &mut GeoWorkflow, command: ComposerCommand) -> Result<(), ComposerError> {
    match command {
        ComposerCommand::SetGoal { goal } => {
            if goal.trim().is_empty() {
                return Err(ComposerError::InvalidNodeId("blank goal".into()));
            }
            workflow.goal = goal.trim().into();
        }
        ComposerCommand::AddReviewedNode {
            template_id,
            source_node_id,
            new_node_id,
        } => {
            validate_node_id(&new_node_id)?;
            if workflow
                .steps
                .iter()
                .any(|step| step.stable_id == new_node_id)
            {
                return Err(ComposerError::InvalidNodeId(new_node_id));
            }
            let template = template_workflow(&template_id)?;
            let mut node = template
                .steps
                .into_iter()
                .find(|step| step.stable_id == source_node_id)
                .ok_or_else(|| ComposerError::NotFound(source_node_id.clone()))?;
            node.id.0 = Uuid::new_v4();
            node.stable_id = new_node_id;
            node.depends_on.clear();
            node.inputs.retain(WorkflowDataRef::is_graph_input);
            workflow.steps.push(node);
        }
        ComposerCommand::RemoveNode { node_id } => {
            let before = workflow.steps.len();
            workflow.steps.retain(|step| step.stable_id != node_id);
            if workflow.steps.len() == before {
                return Err(ComposerError::NotFound(node_id));
            }
            for step in &mut workflow.steps {
                step.depends_on
                    .retain(|dependency| dependency.as_str() != node_id);
                step.inputs.retain(|input| {
                    input
                        .node
                        .as_ref()
                        .is_none_or(|node| node.as_str() != node_id)
                });
            }
            workflow.output_refs.retain(|output| {
                output
                    .node
                    .as_ref()
                    .is_none_or(|node| node.as_str() != node_id)
            });
        }
        ComposerCommand::Connect {
            source_node_id,
            source_port,
            target_node_id,
        } => {
            if source_node_id == target_node_id {
                return Err(ComposerError::InvalidNodeId(source_node_id));
            }
            let source = workflow
                .steps
                .iter()
                .find(|step| step.stable_id == source_node_id)
                .ok_or_else(|| ComposerError::NotFound(source_node_id.clone()))?;
            if !source.outputs.iter().any(|output| {
                output
                    .node
                    .as_ref()
                    .is_some_and(|node| node.as_str() == source_node_id)
                    && output.port == source_port
            }) {
                return Err(ComposerError::NotFound(format!(
                    "{source_node_id}.{source_port}"
                )));
            }
            let target = workflow
                .steps
                .iter_mut()
                .find(|step| step.stable_id == target_node_id)
                .ok_or_else(|| ComposerError::NotFound(target_node_id.clone()))?;
            if !target
                .depends_on
                .iter()
                .any(|node| node.as_str() == source_node_id)
            {
                target
                    .depends_on
                    .push(WorkflowNodeId::new(source_node_id.clone()));
            }
            if !target.inputs.iter().any(|input| {
                input
                    .node
                    .as_ref()
                    .is_some_and(|node| node.as_str() == source_node_id)
                    && input.port == source_port
            }) {
                target
                    .inputs
                    .push(WorkflowDataRef::output(source_node_id, source_port));
            }
        }
        ComposerCommand::Disconnect {
            source_node_id,
            target_node_id,
        } => {
            let target = workflow
                .steps
                .iter_mut()
                .find(|step| step.stable_id == target_node_id)
                .ok_or_else(|| ComposerError::NotFound(target_node_id.clone()))?;
            target
                .depends_on
                .retain(|node| node.as_str() != source_node_id);
            target.inputs.retain(|input| {
                input
                    .node
                    .as_ref()
                    .is_none_or(|node| node.as_str() != source_node_id)
            });
        }
        ComposerCommand::SetParameters {
            node_id,
            parameters,
        } => {
            if !parameters.is_object() {
                return Err(ComposerError::InvalidNodeId(
                    "parameters must be an object".into(),
                ));
            }
            let node = workflow
                .steps
                .iter_mut()
                .find(|step| step.stable_id == node_id)
                .ok_or_else(|| ComposerError::NotFound(node_id))?;
            node.parameters = parameters;
        }
        ComposerCommand::Undo | ComposerCommand::Redo => unreachable!("history handled by caller"),
    }
    Ok(())
}

fn validate_node_id(value: &str) -> Result<(), ComposerError> {
    if value.is_empty()
        || !value.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
    {
        return Err(ComposerError::InvalidNodeId(value.into()));
    }
    Ok(())
}

fn template_ids() -> &'static [&'static str] {
    &[
        "nagoya-density",
        "nagoya-geoparquet",
        "nagoya-geoparquet-density",
        "local-cog-metadata",
        "external-stac",
        "dashboard-export",
        "flood-exposure",
        "xmin-city",
        "evacuation",
        "ndvi-timeseries",
        "pointcloud-change",
        "geocoding",
    ]
}

fn template_workflow(id: &str) -> Result<GeoWorkflow, ComposerError> {
    let workflow = match id {
        "nagoya-density" => nagoya_population_density_template(),
        "nagoya-geoparquet" => nagoya_geoparquet_template(),
        "nagoya-geoparquet-density" => nagoya_geoparquet_density_template(),
        "local-cog-metadata" => local_cog_metadata_template(),
        "external-stac" => external_stac_fetch_template(),
        "dashboard-export" => dashboard_export_template(),
        "flood-exposure" => nagoya_flood_exposure_template(),
        "xmin-city" => nagoya_xmin_city_template(),
        "evacuation" => nagoya_evacuation_template(),
        "ndvi-timeseries" => sentinel_ndvi_timeseries_template(),
        "pointcloud-change" => copc_change_detect_template(),
        "geocoding" => geocoding_template(
            "interactive",
            "provider-slot",
            SourceSnapshot::from_uri(
                "provider://geocoding/runtime-bound",
                Some("sha256:2a8c9e31f42ba3eb211df915f0989d459a199636682c7387e143b328759a88a5"),
                Some("contract-1"),
            ),
            1,
            3,
            "local_only",
            100,
        ),
        _ => return Err(ComposerError::UnknownTemplate(id.into())),
    };
    Ok(workflow)
}

fn template_categories(id: &str) -> Vec<AnalysisTemplateCategory> {
    use AnalysisTemplateCategory::*;
    match id {
        "nagoya-density" | "nagoya-geoparquet-density" => vec![Density, Aggregation],
        "flood-exposure" => vec![Exposure, Aggregation],
        "xmin-city" => vec![Accessibility, Proximity],
        "evacuation" => vec![Accessibility, Suitability, Proximity],
        "ndvi-timeseries" => vec![Change, Aggregation],
        "pointcloud-change" => vec![Change],
        "geocoding" => vec![Geocoding],
        _ => vec![Utility],
    }
}

fn template_verification_profile(id: &str) -> &'static str {
    match id {
        "geocoding" => "adapter_admission+privacy+rate+confidence+source_receipt",
        "flood-exposure" => "source_snapshot+crs+units+independent_aggregation",
        "xmin-city" | "evacuation" => "source_snapshot+projected_crs+route_cost_receipt",
        "ndvi-timeseries" | "pointcloud-change" => "epoch_sources+crs+units+change_receipt",
        "nagoya-density" | "nagoya-geoparquet-density" => {
            "source_snapshot+crs+units+independent_density_verifier"
        }
        _ => "source_snapshot+workflow_digest+execution_receipt",
    }
}

#[cfg(test)]
mod tests {
    use genegis_crs::CoordinateUnit;

    use super::*;

    #[test]
    fn exposes_ten_plus_reviewed_templates_and_undoable_typed_edits() {
        let templates = reviewed_workflow_templates();
        assert!(templates.len() >= 10);
        assert!(templates
            .iter()
            .all(|template| template.workflow_digest.starts_with("sha256:")));
        assert!(templates
            .iter()
            .all(|template| !template.verification_profile.is_empty()));
        let categories = templates
            .iter()
            .flat_map(|template| template.categories.iter().copied())
            .collect::<Vec<_>>();
        for required in [
            AnalysisTemplateCategory::Density,
            AnalysisTemplateCategory::Exposure,
            AnalysisTemplateCategory::Accessibility,
            AnalysisTemplateCategory::Change,
            AnalysisTemplateCategory::Suitability,
            AnalysisTemplateCategory::Proximity,
            AnalysisTemplateCategory::Aggregation,
            AnalysisTemplateCategory::Geocoding,
        ] {
            assert!(categories.contains(&required), "missing {required:?}");
        }

        let mut composer = WorkflowComposer::from_template("nagoya-density").expect("draft");
        let original_goal = composer.workflow.goal.clone();
        composer
            .apply(ComposerCommand::SetGoal {
                goal: "Custom density".into(),
            })
            .expect("edit");
        assert_eq!(composer.workflow.goal, "Custom density");
        composer.apply(ComposerCommand::Undo).expect("undo");
        assert_eq!(composer.workflow.goal, original_goal);
        composer.apply(ComposerCommand::Redo).expect("redo");
        assert_eq!(composer.workflow.goal, "Custom density");
        composer.workflow_for_execution().expect("run-ready graph");
    }

    #[test]
    fn invalid_crs_unit_and_cycle_fail_before_execution() {
        let mut composer = WorkflowComposer::from_template("nagoya-density").expect("draft");
        composer.workflow.input_contracts[0].coordinate_unit = Some(CoordinateUnit::Metres);
        assert!(composer.workflow_for_execution().is_err());

        let mut composer = WorkflowComposer::from_template("nagoya-density").expect("draft");
        let first = composer.workflow.steps[0].stable_id.clone();
        let last = composer
            .workflow
            .steps
            .last()
            .expect("last")
            .stable_id
            .clone();
        let port = composer.workflow.steps.last().expect("last").outputs[0]
            .port
            .clone();
        composer
            .apply(ComposerCommand::Connect {
                source_node_id: last,
                source_port: port,
                target_node_id: first,
            })
            .expect("draft edge");
        assert!(composer.workflow_for_execution().is_err());

        let mut composer = WorkflowComposer::from_template("nagoya-density").expect("draft");
        let source = composer.workflow.steps[0].stable_id.clone();
        let target = composer.workflow.steps[1].stable_id.clone();
        assert!(composer
            .apply(ComposerCommand::Connect {
                source_node_id: source,
                source_port: "undeclared-schema-port".into(),
                target_node_id: target,
            })
            .is_err());
    }
}
