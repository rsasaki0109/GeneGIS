use genegis_core::{
    AttributeField, AttributeType, AttributeValue, Command, CommandBus, CommandEnvelope,
    CommandError, CommandOrigin, EditableGeometryKind, FeatureDraft, FeatureEdit, FeatureGeometry,
    FeatureSchema, MutationWorkflowBinding, Project, WorkflowDigest,
};
use genegis_crs::Crs;
use genegis_workflow::{GeoWorkflow, WorkflowDataRef, WorkflowStep};
use std::collections::BTreeMap;
use uuid::Uuid;

fn workflow_for(operation: &str) -> (GeoWorkflow, MutationWorkflowBinding, WorkflowDigest) {
    let mut workflow = GeoWorkflow::new(format!("Authorize {operation}"));
    let step = WorkflowStep::named("edit", operation, serde_json::json!({"version": 1}))
        .with_outputs([WorkflowDataRef::output("edit", "result")]);
    workflow.push_step(step);
    workflow.add_output_ref(WorkflowDataRef::output("edit", "result"));
    let digest = WorkflowDigest::new(workflow.stable_digest().unwrap());
    let binding = MutationWorkflowBinding {
        workflow_id: workflow.id,
        node_id: "edit".into(),
    };
    (workflow, binding, digest)
}

fn schema() -> FeatureSchema {
    FeatureSchema {
        primary_key: "code".into(),
        fields: vec![
            AttributeField {
                name: "code".into(),
                value_type: AttributeType::Text,
                nullable: false,
            },
            AttributeField {
                name: "population".into(),
                value_type: AttributeType::Integer,
                nullable: false,
            },
        ],
    }
}

fn feature(id: &str, population: i64) -> FeatureDraft {
    FeatureDraft {
        id: id.into(),
        geometry: FeatureGeometry::Polygon(vec![vec![
            [0.0, 0.0],
            [100.0, 0.0],
            [100.0, 100.0],
            [0.0, 100.0],
            [0.0, 0.0],
        ]]),
        attributes: BTreeMap::from([
            ("code".into(), AttributeValue::Text(id.into())),
            ("population".into(), AttributeValue::Integer(population)),
        ]),
    }
}

fn apply_workflow_command(
    bus: &mut CommandBus,
    project: &mut Project,
    workflow: GeoWorkflow,
    digest: WorkflowDigest,
    command: Command,
) -> Result<(), CommandError> {
    bus.register_workflow(workflow)?;
    bus.apply(
        project,
        CommandEnvelope::new(CommandOrigin::Ui, command).with_workflow_digest(digest),
    )?;
    Ok(())
}

#[test]
fn edit_requires_matching_dag_and_supports_undo_redo_replay() {
    let mut project = Project::new("editing");
    let mut bus = CommandBus::new(project.clone());
    let add = CommandEnvelope::new(
        CommandOrigin::Cli,
        Command::AddLayer {
            name: "wards".into(),
            source_id: Uuid::nil(),
        },
    );
    let layer_id = add.id;
    bus.apply(&mut project, add).unwrap();

    let (workflow, binding, digest) = workflow_for("InitializeEditableLayer");
    apply_workflow_command(
        &mut bus,
        &mut project,
        workflow,
        digest,
        Command::InitializeEditableLayer {
            layer_id,
            crs: Crs::nagoya_projected(),
            geometry_kind: EditableGeometryKind::Polygon,
            schema: schema(),
            workflow: binding,
        },
    )
    .unwrap();

    let (workflow, binding, digest) = workflow_for("EditFeatureCreate");
    apply_workflow_command(
        &mut bus,
        &mut project,
        workflow,
        digest,
        Command::EditFeatures {
            layer_id,
            expected_layer_revision: 0,
            edit: FeatureEdit::Create {
                feature: feature("23101", 250_000),
            },
            workflow: binding,
        },
    )
    .unwrap();
    let edited_digest = project.state_digest();
    assert_eq!(project.workspace().editable_layers[0].features.len(), 1);
    assert_eq!(project.workspace().provenance.entries.len(), 3);
    assert!(project.workspace().provenance.entries[2]
        .details
        .get("workflow_digest")
        .is_some());

    bus.undo(&mut project).unwrap();
    assert!(project.workspace().editable_layers[0].features.is_empty());
    bus.redo(&mut project).unwrap();
    assert_eq!(project.state_digest(), edited_digest);

    let path = std::env::temp_dir().join(format!("genegis-edit-{}.json", Uuid::new_v4()));
    bus.persist(&path).unwrap();
    let mut loaded = CommandBus::load(&path).unwrap();
    let replayed = loaded.replay().unwrap();
    assert_eq!(replayed.state_digest(), edited_digest);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn stale_revision_wrong_node_and_invalid_geometry_do_not_mutate_state() {
    let mut project = Project::new("editing-negative");
    let mut bus = CommandBus::new(project.clone());
    let add = CommandEnvelope::new(
        CommandOrigin::Cli,
        Command::AddLayer {
            name: "wards".into(),
            source_id: Uuid::nil(),
        },
    );
    let layer_id = add.id;
    bus.apply(&mut project, add).unwrap();
    let (workflow, binding, digest) = workflow_for("InitializeEditableLayer");
    apply_workflow_command(
        &mut bus,
        &mut project,
        workflow,
        digest,
        Command::InitializeEditableLayer {
            layer_id,
            crs: Crs::nagoya_projected(),
            geometry_kind: EditableGeometryKind::Polygon,
            schema: schema(),
            workflow: binding,
        },
    )
    .unwrap();

    let before = project.state_digest();
    let (workflow, mut binding, digest) = workflow_for("EditFeatureCreate");
    binding.node_id = "arbitrary-node".into();
    bus.register_workflow(workflow).unwrap();
    let result = bus.apply(
        &mut project,
        CommandEnvelope::new(
            CommandOrigin::Plugin,
            Command::EditFeatures {
                layer_id,
                expected_layer_revision: 0,
                edit: FeatureEdit::Create {
                    feature: feature("23101", 250_000),
                },
                workflow: binding,
            },
        )
        .with_workflow_digest(digest),
    );
    assert!(matches!(
        result,
        Err(CommandError::MutationWorkflowBinding { .. })
    ));
    assert_eq!(project.state_digest(), before);

    let (workflow, binding, digest) = workflow_for("EditFeatureCreate");
    let mut invalid = feature("23101", 250_000);
    let FeatureGeometry::Polygon(rings) = &mut invalid.geometry else {
        unreachable!()
    };
    rings[0].pop();
    let result = apply_workflow_command(
        &mut bus,
        &mut project,
        workflow,
        digest,
        Command::EditFeatures {
            layer_id,
            expected_layer_revision: 0,
            edit: FeatureEdit::Create { feature: invalid },
            workflow: binding,
        },
    );
    assert!(matches!(
        result,
        Err(CommandError::FeatureEditRejected { .. })
    ));
    assert_eq!(project.state_digest(), before);
}
