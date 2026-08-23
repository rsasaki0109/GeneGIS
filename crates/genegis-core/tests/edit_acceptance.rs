use genegis_core::{
    AttributeField, AttributeType, AttributeValue, Command, CommandBus, CommandEnvelope,
    CommandOrigin, EditableGeometryKind, FeatureDraft, FeatureEdit, FeatureGeometry, FeatureSchema,
    MutationWorkflowBinding, Project, WorkflowDigest,
};
use genegis_crs::Crs;
use genegis_style::{
    render_evidence_svg, ArtifactSourceIdentity, ClassBreak, ClassificationMethod,
    EvidenceMapStyle, LabelStyle, LegendSpec, MapArtifact, MapLayout, SvgPolygon,
};
use genegis_workflow::{GeoWorkflow, WorkflowDataRef, WorkflowStep};
use std::collections::BTreeMap;
use uuid::Uuid;

fn workflow_for(operation: &str) -> (GeoWorkflow, MutationWorkflowBinding, WorkflowDigest) {
    let mut workflow = GeoWorkflow::new(format!("Authorize {operation}"));
    workflow.push_step(
        WorkflowStep::named("mutation", operation, serde_json::json!({"version": 1}))
            .with_outputs([WorkflowDataRef::output("mutation", "result")]),
    );
    workflow.add_output_ref(WorkflowDataRef::output("mutation", "result"));
    let digest = WorkflowDigest::new(workflow.stable_digest().unwrap());
    let binding = MutationWorkflowBinding {
        workflow_id: workflow.id,
        node_id: "mutation".into(),
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

fn square(id: &str, index: usize, population: i64) -> FeatureDraft {
    let x = (index % 10) as f64 * 110.0;
    let y = (index / 10) as f64 * 110.0;
    FeatureDraft {
        id: id.into(),
        geometry: FeatureGeometry::Polygon(vec![vec![
            [x, y],
            [x + 100.0, y],
            [x + 100.0, y + 100.0],
            [x, y + 100.0],
            [x, y],
        ]]),
        attributes: BTreeMap::from([
            ("code".into(), AttributeValue::Text(id.into())),
            ("population".into(), AttributeValue::Integer(population)),
        ]),
    }
}

fn style(layer_id: Uuid, style_id: Uuid, legend_x: u32) -> EvidenceMapStyle {
    EvidenceMapStyle {
        id: style_id,
        layer_id,
        field: "population".into(),
        value_unit: "persons".into(),
        classification: ClassificationMethod::Manual,
        breaks: vec![
            ClassBreak {
                minimum: 0.0,
                maximum: 100_000.0,
                fill: "#dbeafe".into(),
                label: "0–100k".into(),
            },
            ClassBreak {
                minimum: 100_000.0,
                maximum: 1_000_000.0,
                fill: "#1d4ed8".into(),
                label: "100k–1m".into(),
            },
        ],
        legend: LegendSpec {
            title: "Population".into(),
            unit: "persons".into(),
            x: legend_x,
            y: 50,
        },
        labels: Some(LabelStyle {
            field: "code".into(),
            font_family: "Noto Sans".into(),
            font_digest: format!("sha256:{}", "a".repeat(64)),
            size_px: 10.0,
            color: "#111111".into(),
        }),
    }
}

fn envelope(command: Command, digest: &WorkflowDigest) -> CommandEnvelope {
    CommandEnvelope::new(CommandOrigin::Ui, command).with_workflow_digest(digest.clone())
}

fn artifact(project: &Project) -> MapArtifact {
    let editable = &project.workspace().editable_layers[0];
    let style = &project.workspace().map_styles[0];
    let features = editable
        .features
        .values()
        .map(|feature| {
            let FeatureGeometry::Polygon(rings) = &feature.geometry else {
                panic!("fixture remains polygonal")
            };
            let population = match feature.attributes["population"] {
                AttributeValue::Integer(value) => value as f64,
                _ => panic!("population schema"),
            };
            SvgPolygon {
                id: feature.id.clone(),
                exterior: rings[0].clone(),
                value: population,
                label: Some(feature.id.clone()),
            }
        })
        .collect::<Vec<_>>();
    render_evidence_svg(
        style,
        &MapLayout {
            width_px: 1024,
            height_px: 768,
            title: "Operational editing replay".into(),
            crs: "EPSG:6675".into(),
            coordinate_unit: "metre".into(),
            renderer: "genegis-svg/0.1.0".into(),
            padding_px: 40,
        },
        &features,
        vec![ArtifactSourceIdentity {
            source: "fixture://phase-12/editing".into(),
            digest: format!("sha256:{}", "b".repeat(64)),
            license: "CC0-1.0".into(),
        }],
        &project.state_digest(),
    )
    .unwrap()
}

type InitializedEditState = (
    Project,
    CommandBus,
    Uuid,
    (MutationWorkflowBinding, WorkflowDigest),
    (MutationWorkflowBinding, WorkflowDigest),
    (MutationWorkflowBinding, WorkflowDigest),
);

fn initialized() -> InitializedEditState {
    let mut project = Project::new("phase-12-editing");
    let mut bus = CommandBus::new(project.clone());
    let add = CommandEnvelope::new(
        CommandOrigin::System,
        Command::AddLayer {
            name: "wards".into(),
            source_id: Uuid::nil(),
        },
    );
    let layer_id = add.id;
    bus.apply(&mut project, add).unwrap();
    let (init_workflow, init_binding, init_digest) = workflow_for("InitializeEditableLayer");
    bus.register_workflow(init_workflow).unwrap();
    bus.apply(
        &mut project,
        envelope(
            Command::InitializeEditableLayer {
                layer_id,
                crs: Crs::nagoya_projected(),
                geometry_kind: EditableGeometryKind::Polygon,
                schema: schema(),
                workflow: init_binding,
            },
            &init_digest,
        ),
    )
    .unwrap();
    let (create_workflow, create_binding, create_digest) = workflow_for("EditFeatureCreate");
    let (update_workflow, update_binding, update_digest) = workflow_for("EditFeatureUpdate");
    let (style_workflow, style_binding, style_digest) = workflow_for("SetEvidenceMapStyle");
    bus.register_workflow(create_workflow).unwrap();
    bus.register_workflow(update_workflow).unwrap();
    bus.register_workflow(style_workflow).unwrap();
    (
        project,
        bus,
        layer_id,
        (create_binding, create_digest),
        (update_binding, update_digest),
        (style_binding, style_digest),
    )
}

#[test]
fn hundred_mixed_commands_replay_to_identical_project_workflow_and_svg_digests() {
    let (mut project, mut bus, layer_id, create, update, style_binding) = initialized();
    for index in 0..50 {
        bus.apply(
            &mut project,
            envelope(
                Command::EditFeatures {
                    layer_id,
                    expected_layer_revision: index as u64,
                    edit: FeatureEdit::Create {
                        feature: square(&format!("f{index:02}"), index, 50_000 + index as i64),
                    },
                    workflow: create.0.clone(),
                },
                &create.1,
            ),
        )
        .unwrap();
    }
    for index in 0..25 {
        bus.apply(
            &mut project,
            envelope(
                Command::EditFeatures {
                    layer_id,
                    expected_layer_revision: 50 + index as u64,
                    edit: FeatureEdit::Update {
                        feature: square(&format!("f{index:02}"), index, 200_000 + index as i64),
                        expected_feature_revision: 1,
                    },
                    workflow: update.0.clone(),
                },
                &update.1,
            ),
        )
        .unwrap();
    }
    let style_id = Uuid::from_u128(0xfeed);
    for index in 0..25 {
        bus.apply(
            &mut project,
            envelope(
                Command::SetEvidenceMapStyle {
                    style: style(layer_id, style_id, 20 + index),
                    workflow: style_binding.0.clone(),
                },
                &style_binding.1,
            ),
        )
        .unwrap();
    }
    assert_eq!(
        bus.history().len(),
        102,
        "add + initialize + 100 mixed commands"
    );
    let project_digest = project.state_digest();
    let workflow_digests = [create.1, update.1, style_binding.1];
    let first_artifact = artifact(&project);
    let second_artifact = artifact(&project);
    assert_eq!(
        first_artifact.artifact_digest,
        second_artifact.artifact_digest
    );
    assert_eq!(
        first_artifact.metadata_digest,
        second_artifact.metadata_digest
    );

    let path =
        std::env::temp_dir().join(format!("genegis-edit-acceptance-{}.json", Uuid::new_v4()));
    bus.persist(&path).unwrap();
    let mut loaded = CommandBus::load(&path).unwrap();
    let replayed = loaded.replay().unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(replayed.state_digest(), project_digest);
    assert_eq!(
        artifact(&replayed).artifact_digest,
        first_artifact.artifact_digest
    );
    assert!(workflow_digests
        .iter()
        .all(|digest| digest.as_str().starts_with("sha256:")));
    println!(
        "{}",
        serde_json::json!({
            "mixed_commands": 100,
            "project_digest": project_digest,
            "workflow_digests": workflow_digests,
            "svg_artifact_digest": first_artifact.artifact_digest,
            "svg_metadata_digest": first_artifact.metadata_digest,
            "replay_match": true
        })
    );
}

fn assert_rejected(bus: &mut CommandBus, project: &mut Project, command: CommandEnvelope) {
    let before = project.state_digest();
    let cursor = bus.cursor();
    assert!(bus.apply(project, command).is_err());
    assert_eq!(project.state_digest(), before);
    assert_eq!(bus.cursor(), cursor);
}

#[test]
fn thirty_negative_edit_style_and_workflow_cases_have_zero_false_accepts() {
    let (mut project, mut bus, layer_id, create, _update, style_binding) = initialized();
    bus.apply(
        &mut project,
        envelope(
            Command::EditFeatures {
                layer_id,
                expected_layer_revision: 0,
                edit: FeatureEdit::Create {
                    feature: square("existing", 0, 10),
                },
                workflow: create.0.clone(),
            },
            &create.1,
        ),
    )
    .unwrap();
    let mut rejected = 0;
    let mut invalid_geometry = Vec::new();
    let mut draft = square("unclosed", 1, 10);
    let FeatureGeometry::Polygon(rings) = &mut draft.geometry else {
        unreachable!()
    };
    rings[0].pop();
    invalid_geometry.push(draft);
    let mut draft = square("too-short", 1, 10);
    draft.geometry = FeatureGeometry::Polygon(vec![vec![[0.0, 0.0], [1.0, 0.0], [0.0, 0.0]]]);
    invalid_geometry.push(draft);
    let mut draft = square("zero-area", 1, 10);
    draft.geometry =
        FeatureGeometry::Polygon(vec![vec![[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [0.0, 0.0]]]);
    invalid_geometry.push(draft);
    let mut draft = square("nan", 1, 10);
    draft.geometry = FeatureGeometry::Point([f64::NAN, 0.0]);
    invalid_geometry.push(draft);
    let mut draft = square("empty-multi", 1, 10);
    draft.geometry = FeatureGeometry::MultiPolygon(vec![]);
    invalid_geometry.push(draft);
    let mut draft = square("wrong-point", 1, 10);
    draft.geometry = FeatureGeometry::Point([0.0, 0.0]);
    invalid_geometry.push(draft);
    let mut draft = square("wrong-line", 1, 10);
    draft.geometry = FeatureGeometry::LineString(vec![[0.0, 0.0], [1.0, 1.0]]);
    invalid_geometry.push(draft);
    let mut draft = square("empty-polygon", 1, 10);
    draft.geometry = FeatureGeometry::Polygon(vec![]);
    invalid_geometry.push(draft);
    let mut draft = square("negative-after-hole", 1, 10);
    draft.geometry = FeatureGeometry::Polygon(vec![
        vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]],
        vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0], [0.0, 0.0]],
    ]);
    invalid_geometry.push(draft);
    invalid_geometry.push(square("existing", 2, 10));
    for feature in invalid_geometry {
        assert_rejected(
            &mut bus,
            &mut project,
            envelope(
                Command::EditFeatures {
                    layer_id,
                    expected_layer_revision: 1,
                    edit: FeatureEdit::Create { feature },
                    workflow: create.0.clone(),
                },
                &create.1,
            ),
        );
        rejected += 1;
    }

    for stale in [0_u64, 2, 3, 4, 5] {
        assert_rejected(
            &mut bus,
            &mut project,
            envelope(
                Command::EditFeatures {
                    layer_id,
                    expected_layer_revision: stale,
                    edit: FeatureEdit::Create {
                        feature: square(&format!("stale-{stale}"), 3, 10),
                    },
                    workflow: create.0.clone(),
                },
                &create.1,
            ),
        );
        rejected += 1;
    }

    let mut schema_drafts = Vec::new();
    let mut draft = square("missing", 4, 10);
    draft.attributes.remove("population");
    schema_drafts.push(draft);
    let mut draft = square("extra", 4, 10);
    draft
        .attributes
        .insert("extra".into(), AttributeValue::Text("x".into()));
    schema_drafts.push(draft);
    let mut draft = square("null-key", 4, 10);
    draft.attributes.insert("code".into(), AttributeValue::Null);
    schema_drafts.push(draft);
    let mut draft = square("null-pop", 4, 10);
    draft
        .attributes
        .insert("population".into(), AttributeValue::Null);
    schema_drafts.push(draft);
    let mut draft = square("wrong-type", 4, 10);
    draft
        .attributes
        .insert("population".into(), AttributeValue::Text("10".into()));
    schema_drafts.push(draft);
    for feature in schema_drafts {
        assert_rejected(
            &mut bus,
            &mut project,
            envelope(
                Command::EditFeatures {
                    layer_id,
                    expected_layer_revision: 1,
                    edit: FeatureEdit::Create { feature },
                    workflow: create.0.clone(),
                },
                &create.1,
            ),
        );
        rejected += 1;
    }

    let good_command = || Command::EditFeatures {
        layer_id,
        expected_layer_revision: 1,
        edit: FeatureEdit::Create {
            feature: square("workflow-negative", 5, 10),
        },
        workflow: create.0.clone(),
    };
    assert_rejected(
        &mut bus,
        &mut project,
        CommandEnvelope::new(CommandOrigin::Plugin, good_command()),
    );
    rejected += 1;
    assert_rejected(
        &mut bus,
        &mut project,
        envelope(
            good_command(),
            &WorkflowDigest::new(format!("sha256:{}", "f".repeat(64))),
        ),
    );
    rejected += 1;
    let mut wrong_node = good_command();
    if let Command::EditFeatures { workflow, .. } = &mut wrong_node {
        workflow.node_id = "missing".into();
    }
    assert_rejected(&mut bus, &mut project, envelope(wrong_node, &create.1));
    rejected += 1;
    let (wrong_workflow, wrong_binding, wrong_digest) = workflow_for("EditFeatureDelete");
    bus.register_workflow(wrong_workflow).unwrap();
    let mut wrong_operation = good_command();
    if let Command::EditFeatures { workflow, .. } = &mut wrong_operation {
        *workflow = wrong_binding;
    }
    assert_rejected(
        &mut bus,
        &mut project,
        envelope(wrong_operation, &wrong_digest),
    );
    rejected += 1;
    let mut unknown = good_command();
    if let Command::EditFeatures { workflow, .. } = &mut unknown {
        workflow.workflow_id = Uuid::new_v4();
    }
    assert_rejected(&mut bus, &mut project, envelope(unknown, &create.1));
    rejected += 1;

    let base_style = style(layer_id, Uuid::from_u128(123), 20);
    let mut styles = Vec::new();
    let mut invalid = base_style.clone();
    invalid.breaks[1].minimum = 100_001.0;
    styles.push(invalid);
    let mut invalid = base_style.clone();
    invalid.breaks[0].fill = "blue".into();
    styles.push(invalid);
    let mut invalid = base_style.clone();
    invalid.labels.as_mut().unwrap().font_digest = "unknown".into();
    styles.push(invalid);
    let mut invalid = base_style.clone();
    invalid.legend.unit = "people".into();
    styles.push(invalid);
    let mut invalid = base_style;
    invalid.breaks[0].minimum = f64::NAN;
    styles.push(invalid);
    for style in styles {
        assert_rejected(
            &mut bus,
            &mut project,
            envelope(
                Command::SetEvidenceMapStyle {
                    style,
                    workflow: style_binding.0.clone(),
                },
                &style_binding.1,
            ),
        );
        rejected += 1;
    }
    assert_eq!(rejected, 30);
    assert_eq!(project.workspace().editable_layers[0].features.len(), 1);
    println!(
        "{}",
        serde_json::json!({"negative_cases": rejected, "false_accepts": 0})
    );
}
