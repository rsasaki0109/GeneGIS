//! Plans, their Workflow Graph form, and execution through the Command bus.
//!
//! A [`ToolkitPlan`] is the planner/UI-facing description of an analysis.
//! It is compiled into a [`GeoWorkflow`] whose input contracts pin every
//! source layer by content digest and CRS. Execution happens only through
//! `Command::RunWorkflow`: the executor re-reads the *registered* workflow
//! (not the plan), so what runs is exactly what the digest authorizes. Any
//! failed verification check aborts the command before state is committed.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use genegis_core::{
    Command, CommandBus, CommandEnvelope, CommandOrigin, InputSnapshot, Project, WorkflowDigest,
    WorkflowExecution, WorkflowExecutionContext, WorkflowExecutionError, WorkflowExecutionEvent,
    WorkflowExecutor,
};
use genegis_crs::{ChecksumVerification, Crs, SourceSnapshot};
use genegis_workflow::{GeoWorkflow, WorkflowDataRef, WorkflowInputContract, WorkflowStep};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{Result, ToolkitError};
use crate::import::{import_bytes, ImportOptions, ImportReport};
use crate::layer::{canonical_json, sha256_bytes, Layer, LayerSummary};
use crate::ops::{self, Check};

/// Operation prefix used for toolkit nodes in the Workflow Graph.
pub const OPERATION_PREFIX: &str = "toolkit.";

/// One analysis step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanStep {
    /// Step identifier, unique within the plan (`[A-Za-z0-9_-]+`).
    pub id: String,
    /// Operation name from [`ops::catalog`].
    pub op: String,
    /// Input role → reference (`lyr_…` layer ID or an earlier step ID).
    #[serde(default)]
    pub inputs: BTreeMap<String, String>,
    /// Operation parameters.
    #[serde(default)]
    pub params: Value,
}

/// An analysis plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolkitPlan {
    /// Goal in the user's words.
    pub goal: String,
    /// Steps in dependency order.
    pub steps: Vec<PlanStep>,
    /// Step whose layer is the result (defaults to the last step).
    #[serde(default)]
    pub output: Option<String>,
    /// Assumptions the planner made.
    #[serde(default)]
    pub assumptions: Vec<String>,
}

/// Receipt for one executed step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepReceipt {
    /// Step ID.
    pub id: String,
    /// Operation.
    pub op: String,
    /// Resolved input layer digests by role.
    pub inputs: BTreeMap<String, String>,
    /// Output layer digest.
    pub output_digest: String,
    /// Output CRS.
    pub crs: String,
    /// Output feature count.
    pub feature_count: usize,
    /// Verification checks (all passed, otherwise the run is rejected).
    pub checks: Vec<Check>,
    /// Method notes.
    pub notes: Vec<String>,
}

/// Result of a verified toolkit run.
#[derive(Debug, Clone)]
pub struct ToolkitRun {
    /// Applied command ID.
    pub command_id: String,
    /// Digest of the authorized workflow.
    pub workflow_digest: String,
    /// Digest of outputs and evidence.
    pub result_digest: String,
    /// The executed workflow.
    pub workflow: GeoWorkflow,
    /// Per-step receipts in execution order.
    pub steps: Vec<StepReceipt>,
    /// Output layer.
    pub output: Layer,
    /// Every step's layer keyed by step ID (includes the output).
    pub step_layers: BTreeMap<String, Layer>,
}

/// Serializable receipt of a toolkit run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunReceipt {
    /// Applied command ID.
    pub command_id: String,
    /// Workflow digest.
    pub workflow_digest: String,
    /// Result digest.
    pub result_digest: String,
    /// Goal.
    pub goal: String,
    /// Planner assumptions.
    pub assumptions: Vec<String>,
    /// Step receipts.
    pub steps: Vec<StepReceipt>,
    /// Output layer summary.
    pub output: LayerSummary,
}

impl ToolkitRun {
    /// Serializable receipt.
    pub fn receipt(&self) -> RunReceipt {
        RunReceipt {
            command_id: self.command_id.clone(),
            workflow_digest: self.workflow_digest.clone(),
            result_digest: self.result_digest.clone(),
            goal: self.workflow.goal.clone(),
            assumptions: self.workflow.assumptions.clone(),
            steps: self.steps.clone(),
            output: self.output.summary(),
        }
    }
}

enum Ref<'a> {
    Layer(&'a str),
    Step(&'a str),
}

fn parse_ref(reference: &str) -> Ref<'_> {
    if let Some(id) = reference.strip_prefix("layer:") {
        Ref::Layer(id)
    } else if let Some(id) = reference.strip_prefix("step:") {
        Ref::Step(id)
    } else if reference.starts_with("lyr_") {
        Ref::Layer(reference)
    } else {
        Ref::Step(reference)
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Structural validation of a plan against the available layers.
pub fn validate_plan(plan: &ToolkitPlan, layers: &BTreeMap<String, Layer>) -> Result<String> {
    if plan.steps.is_empty() {
        return Err(ToolkitError::Plan("plan has no steps".into()));
    }
    if plan.steps.len() > 64 {
        return Err(ToolkitError::Plan("plan has more than 64 steps".into()));
    }
    let catalog = ops::catalog();
    let mut seen = BTreeSet::new();
    for step in &plan.steps {
        if !valid_id(&step.id) || step.id.starts_with("lyr_") {
            return Err(ToolkitError::Plan(format!("invalid step id {:?}", step.id)));
        }
        if !seen.insert(step.id.as_str()) {
            return Err(ToolkitError::Plan(format!("duplicate step id {}", step.id)));
        }
        let spec = catalog.iter().find(|s| s.name == step.op).ok_or_else(|| {
            ToolkitError::Plan(format!("step {}: unknown operation {}", step.id, step.op))
        })?;
        let roles: BTreeSet<&str> = step.inputs.keys().map(String::as_str).collect();
        let expected: BTreeSet<&str> = spec.inputs.iter().copied().collect();
        if roles != expected {
            return Err(ToolkitError::Plan(format!(
                "step {} ({}) needs inputs {:?}, got {:?}",
                step.id, step.op, expected, roles
            )));
        }
        for reference in step.inputs.values() {
            match parse_ref(reference) {
                Ref::Layer(id) if !layers.contains_key(id) => {
                    return Err(ToolkitError::UnknownReference(format!(
                        "step {}: layer {id} is not loaded",
                        step.id
                    )))
                }
                Ref::Step(id) if !seen.contains(id) || id == step.id => {
                    return Err(ToolkitError::UnknownReference(format!(
                        "step {}: {id} is not an earlier step or a loaded layer",
                        step.id
                    )))
                }
                _ => {}
            }
        }
        if !(step.params.is_null() || step.params.is_object()) {
            return Err(ToolkitError::Plan(format!(
                "step {}: params must be an object",
                step.id
            )));
        }
    }
    let output = plan
        .output
        .clone()
        .unwrap_or_else(|| plan.steps.last().expect("non-empty").id.clone());
    if !seen.contains(output.as_str()) {
        return Err(ToolkitError::Plan(format!("output {output} is not a step")));
    }
    // Every step must contribute to the output.
    let mut needed = BTreeSet::from([output.clone()]);
    for step in plan.steps.iter().rev() {
        if needed.contains(&step.id) {
            for reference in step.inputs.values() {
                if let Ref::Step(id) = parse_ref(reference) {
                    needed.insert(id.to_string());
                }
            }
        }
    }
    if let Some(unused) = plan.steps.iter().find(|s| !needed.contains(&s.id)) {
        return Err(ToolkitError::Plan(format!(
            "step {} does not contribute to the output",
            unused.id
        )));
    }
    Ok(output)
}

/// Immutable source identity of a stored layer.
pub fn layer_snapshot(layer: &Layer) -> SourceSnapshot {
    let digest = layer.digest();
    let mut source = SourceSnapshot::new(format!("layer://{digest}"));
    source.dataset_id = Some(layer.id());
    source.license = layer.provenance.license.clone();
    source.checksum = Some(digest.clone());
    source.expected_checksum = Some(digest.clone());
    source.observed_checksum = Some(digest);
    source.checksum_status = ChecksumVerification::Verified;
    source
}

fn core_crs(layer: &Layer) -> Result<Crs> {
    let crs = Crs::parse(&layer.crs).map_err(|e| ToolkitError::UnsupportedCrs(e.to_string()))?;
    crs.require_known()
        .map_err(|e| ToolkitError::UnsupportedCrs(e.to_string()))?;
    Ok(crs)
}

/// Compile a validated plan into a Workflow Graph.
pub fn plan_to_workflow(
    plan: &ToolkitPlan,
    layers: &BTreeMap<String, Layer>,
) -> Result<GeoWorkflow> {
    let output = validate_plan(plan, layers)?;
    let mut workflow = GeoWorkflow::new(plan.goal.clone());
    workflow.assumptions = plan.assumptions.clone();
    let mut referenced: BTreeSet<&str> = BTreeSet::new();
    for step in &plan.steps {
        for reference in step.inputs.values() {
            if let Ref::Layer(id) = parse_ref(reference) {
                referenced.insert(id);
            }
        }
    }
    for id in &referenced {
        let layer = &layers[*id];
        workflow.add_input_contract(
            WorkflowInputContract::new(*id)
                .with_crs(core_crs(layer)?)
                .with_source_snapshot(layer_snapshot(layer)),
        );
        workflow.inputs.push(json!({
            "layer_id": id,
            "name": layer.name,
            "crs": layer.crs,
            "digest": layer.digest(),
            "feature_count": layer.features.len(),
        }));
    }
    for step in &plan.steps {
        let mut dependencies = Vec::new();
        let mut inputs = Vec::new();
        let mut roles = serde_json::Map::new();
        for (role, reference) in &step.inputs {
            match parse_ref(reference) {
                Ref::Layer(id) => {
                    inputs.push(WorkflowDataRef::input(id));
                    roles.insert(role.clone(), json!({"layer": id}));
                }
                Ref::Step(id) => {
                    if !dependencies.contains(&id.to_string()) {
                        dependencies.push(id.to_string());
                    }
                    inputs.push(WorkflowDataRef::output(id, "layer"));
                    roles.insert(role.clone(), json!({"step": id}));
                }
            }
        }
        let parameters = json!({
            "inputs": roles,
            "params": if step.params.is_null() { json!({}) } else { step.params.clone() },
        });
        workflow.push_step(
            WorkflowStep::named(
                step.id.clone(),
                format!("{OPERATION_PREFIX}{}", step.op),
                parameters,
            )
            .with_dependencies(dependencies)
            .with_inputs(inputs)
            .with_outputs([WorkflowDataRef::output(step.id.as_str(), "layer")]),
        );
    }
    workflow.add_output_ref(WorkflowDataRef::output(output.as_str(), "layer"));
    workflow
        .outputs
        .push(json!({"step": output, "port": "layer"}));
    workflow
        .validate()
        .map_err(|e| ToolkitError::Plan(e.to_string()))?;
    Ok(workflow)
}

struct RunArtifacts {
    steps: Vec<StepReceipt>,
    step_layers: BTreeMap<String, Layer>,
    output_step: String,
}

struct PlanExecutor<'a> {
    layers: &'a BTreeMap<String, Layer>,
    artifacts: Mutex<Option<RunArtifacts>>,
}

fn failed(reason: impl Into<String>) -> WorkflowExecutionError {
    WorkflowExecutionError::Failed(reason.into())
}

impl WorkflowExecutor for PlanExecutor<'_> {
    fn execute(
        &self,
        workflow: &GeoWorkflow,
        context: &WorkflowExecutionContext,
    ) -> std::result::Result<WorkflowExecution, WorkflowExecutionError> {
        // Every graph input must still have exactly the authorized content.
        for contract in &workflow.input_contracts {
            let layer = self.layers.get(&contract.name).ok_or_else(|| {
                failed(format!(
                    "input layer {} is no longer available",
                    contract.name
                ))
            })?;
            let expected = contract
                .source_snapshot
                .as_ref()
                .and_then(|s| s.checksum.clone())
                .ok_or_else(|| failed(format!("input {} has no pinned digest", contract.name)))?;
            if layer.digest() != expected {
                return Err(failed(format!(
                    "input layer {} changed after authorization",
                    contract.name
                )));
            }
        }
        let order = workflow
            .topological_order()
            .map_err(|e| failed(e.to_string()))?;
        let mut produced: BTreeMap<String, Layer> = BTreeMap::new();
        let mut receipts = Vec::new();
        for node in order {
            let step = workflow
                .steps
                .iter()
                .find(|s| s.node_id() == node)
                .ok_or_else(|| failed(format!("node {} missing", node.as_str())))?;
            let op = step
                .operation
                .strip_prefix(OPERATION_PREFIX)
                .ok_or_else(|| failed(format!("{} is not a toolkit operation", step.operation)))?;
            let roles = step
                .parameters
                .get("inputs")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let mut inputs = BTreeMap::new();
            let mut input_digests = BTreeMap::new();
            for (role, source) in roles {
                let layer = if let Some(id) = source.get("layer").and_then(Value::as_str) {
                    self.layers
                        .get(id)
                        .ok_or_else(|| failed(format!("layer {id} missing")))?
                } else if let Some(id) = source.get("step").and_then(Value::as_str) {
                    produced
                        .get(id)
                        .ok_or_else(|| failed(format!("step {id} has not produced a layer")))?
                } else {
                    return Err(failed(format!("input {role} has no source")));
                };
                input_digests.insert(role.clone(), layer.digest());
                inputs.insert(role, layer.clone());
            }
            let params = step.parameters.get("params").cloned().unwrap_or(json!({}));
            let output = ops::run(op, &inputs, &params)
                .map_err(|e| failed(format!("step {}: {e}", node.as_str())))?;
            if let Some(check) = output.checks.iter().find(|c| !c.passed) {
                return Err(failed(format!(
                    "verification failed at step {} ({op}): {} — {}",
                    node.as_str(),
                    check.id,
                    check.detail
                )));
            }
            let mut layer = output.layer;
            layer.provenance.workflow_digest = Some(context.workflow_digest.to_string());
            receipts.push(StepReceipt {
                id: node.as_str().to_string(),
                op: op.to_string(),
                inputs: input_digests,
                output_digest: layer.digest(),
                crs: layer.crs.clone(),
                feature_count: layer.features.len(),
                checks: output.checks,
                notes: output.notes,
            });
            produced.insert(node.as_str().to_string(), layer);
        }
        let output_step = workflow
            .output_refs
            .first()
            .and_then(|r| r.node.as_ref())
            .map(|n| n.as_str().to_string())
            .ok_or_else(|| failed("workflow has no output"))?;
        let output = produced
            .get(&output_step)
            .ok_or_else(|| failed("output step produced nothing"))?;
        let evidence = serde_json::to_value(&receipts).map_err(|e| failed(e.to_string()))?;
        let result_digest =
            sha256_bytes(format!("{}\0{}", output.digest(), canonical_json(&evidence)).as_bytes());
        let summary = serde_json::to_value(output.summary()).map_err(|e| failed(e.to_string()))?;
        *self
            .artifacts
            .lock()
            .map_err(|_| failed("artifact lock poisoned"))? = Some(RunArtifacts {
            steps: receipts,
            step_layers: produced,
            output_step,
        });
        Ok(WorkflowExecution {
            result_digest,
            output: summary,
            evidence,
            events: vec![WorkflowExecutionEvent {
                kind: "toolkit_analysis".into(),
                source_uri: None,
                observed_at: context.command_timestamp,
                details: json!({"workflow_digest": context.workflow_digest, "command_id": context.command_id}),
            }],
        })
    }
}

/// Validate, compile, authorize, and execute a plan through `RunWorkflow`.
pub fn run_plan(plan: &ToolkitPlan, layers: &BTreeMap<String, Layer>) -> Result<ToolkitRun> {
    let workflow = plan_to_workflow(plan, layers)?;
    let workflow_digest = WorkflowDigest::new(
        workflow
            .stable_digest()
            .map_err(|e| ToolkitError::Plan(e.to_string()))?,
    );
    let mut envelope = CommandEnvelope::new(
        CommandOrigin::Ui,
        Command::RunWorkflow {
            workflow_id: workflow.id,
        },
    )
    .with_workflow_digest(workflow_digest.clone());
    for contract in &workflow.input_contracts {
        let layer = &layers[&contract.name];
        let snapshot = layer_snapshot(layer);
        envelope = envelope
            .with_source_snapshot(snapshot.clone())
            .with_input_snapshot(
                InputSnapshot::new(contract.name.clone(), snapshot).with_crs(core_crs(layer)?),
            );
    }
    let command_id = envelope.id;
    let executor = PlanExecutor {
        layers,
        artifacts: Mutex::new(None),
    };
    let mut project = Project::new("GeneGIS toolkit analysis");
    let mut bus = CommandBus::new(project.clone());
    bus.register_workflow(workflow.clone())
        .map_err(|e| ToolkitError::Command(e.to_string()))?;
    let execution = bus
        .apply_with_executor(&mut project, envelope, &executor)
        .map_err(|e| ToolkitError::Command(e.to_string()))?;
    let artifacts = executor
        .artifacts
        .into_inner()
        .map_err(|_| ToolkitError::Command("artifact lock poisoned".into()))?
        .ok_or_else(|| ToolkitError::Command("executor produced no artifacts".into()))?;
    let result_digest = execution
        .result_digest
        .ok_or_else(|| ToolkitError::Command("workflow returned no result digest".into()))?;
    let output = artifacts.step_layers[&artifacts.output_step].clone();
    Ok(ToolkitRun {
        command_id: command_id.to_string(),
        workflow_digest: workflow_digest.to_string(),
        result_digest,
        workflow,
        steps: artifacts.steps,
        output,
        step_layers: artifacts.step_layers,
    })
}

/// Receipt of an import executed through Command + Workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportReceipt {
    /// Applied command ID.
    pub command_id: String,
    /// Workflow digest.
    pub workflow_digest: String,
    /// Result digest (equals the layer digest).
    pub result_digest: String,
    /// What the importer did.
    pub report: ImportReport,
    /// Imported layer summary.
    pub layer: LayerSummary,
}

struct ImportExecutor<'a> {
    filename: &'a str,
    bytes: &'a [u8],
    options: &'a ImportOptions,
    result: Mutex<Option<(Layer, ImportReport)>>,
    error: Mutex<Option<ToolkitError>>,
}

impl WorkflowExecutor for ImportExecutor<'_> {
    fn execute(
        &self,
        _workflow: &GeoWorkflow,
        context: &WorkflowExecutionContext,
    ) -> std::result::Result<WorkflowExecution, WorkflowExecutionError> {
        let observed = sha256_bytes(self.bytes);
        let authorized = context
            .input_snapshots
            .iter()
            .find(|s| s.name == "upload")
            .and_then(|s| s.source.checksum.clone());
        if authorized.as_deref() != Some(observed.as_str()) {
            return Err(failed("uploaded bytes do not match the authorized digest"));
        }
        let (mut layer, report) = match import_bytes(self.filename, self.bytes, self.options) {
            Ok(result) => result,
            Err(error) => {
                let message = error.to_string();
                if let Ok(mut slot) = self.error.lock() {
                    *slot = Some(error);
                }
                return Err(failed(message));
            }
        };
        layer.provenance.workflow_digest = Some(context.workflow_digest.to_string());
        let digest = layer.digest();
        let output = serde_json::to_value(layer.summary()).map_err(|e| failed(e.to_string()))?;
        let evidence = serde_json::to_value(&report).map_err(|e| failed(e.to_string()))?;
        *self
            .result
            .lock()
            .map_err(|_| failed("import lock poisoned"))? = Some((layer, report));
        Ok(WorkflowExecution {
            result_digest: digest,
            output,
            evidence,
            events: vec![WorkflowExecutionEvent {
                kind: "source_read".into(),
                source_uri: Some(format!("upload://{}", self.filename)),
                observed_at: context.command_timestamp,
                details: json!({"bytes": self.bytes.len(), "sha256": observed}),
            }],
        })
    }
}

/// Import user data through Command + Workflow Graph.
pub fn import_through_workflow(
    filename: &str,
    bytes: &[u8],
    options: &ImportOptions,
) -> Result<(Layer, ImportReceipt)> {
    let digest = sha256_bytes(bytes);
    let mut source = SourceSnapshot::new(format!(
        "upload://{}",
        filename.rsplit(['/', '\\']).next().unwrap_or(filename)
    ));
    source.license = options.license.clone();
    source.checksum = Some(digest.clone());
    source.expected_checksum = Some(digest.clone());
    source.observed_checksum = Some(digest.clone());
    source.checksum_status = ChecksumVerification::Verified;
    let options_json =
        serde_json::to_value(options).map_err(|e| ToolkitError::Plan(e.to_string()))?;
    let mut workflow = GeoWorkflow::new(format!("{filename} を取り込む"));
    workflow.add_input_contract(
        WorkflowInputContract::new("upload")
            .with_value_unit("bytes")
            .with_source_snapshot(source.clone()),
    );
    let stages: [(&str, &str, Value); 5] = [
        (
            "detect-format",
            "toolkit.DetectFormat",
            json!({"filename": filename, "format": options.format}),
        ),
        (
            "decode-features",
            "toolkit.DecodeFeatures",
            json!({"encoding": options.encoding, "table": options.table, "x_field": options.x_field, "y_field": options.y_field, "wkt_field": options.wkt_field}),
        ),
        (
            "resolve-crs",
            "toolkit.ResolveCrs",
            json!({"crs_override": options.crs, "policy": "declared > format default > inferred (flagged) > fail closed"}),
        ),
        (
            "infer-schema",
            "toolkit.InferSchema",
            json!({"leading_zero_codes": "text"}),
        ),
        (
            "normalize-layer",
            "toolkit.NormalizeLayer",
            json!({"options": options_json, "digest": "genegis-layer-v1"}),
        ),
    ];
    let mut previous: Option<&str> = None;
    for (id, operation, parameters) in stages {
        let mut step = WorkflowStep::named(id, operation, parameters)
            .with_outputs([WorkflowDataRef::output(id, "result")]);
        step = match previous {
            None => step.with_inputs([WorkflowDataRef::input("upload")]),
            Some(prev) => step
                .with_dependencies([prev])
                .with_inputs([WorkflowDataRef::output(prev, "result")]),
        };
        workflow.push_step(step);
        previous = Some(id);
    }
    workflow.add_output_ref(WorkflowDataRef::output("normalize-layer", "result"));
    let workflow_digest = WorkflowDigest::new(
        workflow
            .stable_digest()
            .map_err(|e| ToolkitError::Plan(e.to_string()))?,
    );
    let envelope = CommandEnvelope::new(
        CommandOrigin::Ui,
        Command::RunWorkflow {
            workflow_id: workflow.id,
        },
    )
    .with_workflow_digest(workflow_digest.clone())
    .with_source_snapshot(source.clone())
    .with_input_snapshot(InputSnapshot::new("upload", source).with_value_unit("bytes"));
    let command_id = envelope.id;
    let executor = ImportExecutor {
        filename,
        bytes,
        options,
        result: Mutex::new(None),
        error: Mutex::new(None),
    };
    let mut project = Project::new("GeneGIS data import");
    let mut bus = CommandBus::new(project.clone());
    bus.register_workflow(workflow)
        .map_err(|e| ToolkitError::Command(e.to_string()))?;
    let execution = match bus.apply_with_executor(&mut project, envelope, &executor) {
        Ok(execution) => execution,
        Err(error) => {
            // Surface the importer's own typed error (CRS required, bad format, …).
            if let Some(typed) = executor.error.lock().ok().and_then(|mut slot| slot.take()) {
                return Err(typed);
            }
            return Err(ToolkitError::Command(error.to_string()));
        }
    };
    let (layer, report) = executor
        .result
        .into_inner()
        .map_err(|_| ToolkitError::Command("import lock poisoned".into()))?
        .ok_or_else(|| ToolkitError::Command("importer produced no layer".into()))?;
    let result_digest = execution
        .result_digest
        .ok_or_else(|| ToolkitError::Command("import returned no digest".into()))?;
    if result_digest != layer.digest() {
        return Err(ToolkitError::Verification("import digest mismatch".into()));
    }
    let receipt = ImportReceipt {
        command_id: command_id.to_string(),
        workflow_digest: workflow_digest.to_string(),
        result_digest,
        report,
        layer: layer.summary(),
    };
    Ok((layer, receipt))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::{CrsStatus, Feature};
    use geo_types::{polygon, Geometry, Point};

    fn store() -> BTreeMap<String, Layer> {
        let mut zones = Layer::new("zones", "EPSG:4326", CrsStatus::Declared);
        zones.features.push(Feature {
            id: 0,
            geometry: Some(Geometry::Polygon(polygon![(x: 136.90, y: 35.16), (x: 136.91, y: 35.16), (x: 136.91, y: 35.17), (x: 136.90, y: 35.17), (x: 136.90, y: 35.16)])),
            properties: BTreeMap::from([("pop".to_string(), Value::from(1000))]),
        });
        zones.refresh_schema();
        zones.set_field_unit("pop", "persons");
        let mut sites = Layer::new("sites", "EPSG:4326", CrsStatus::Declared);
        sites.features.push(Feature {
            id: 0,
            geometry: Some(Geometry::Point(Point::new(136.905, 35.165))),
            properties: BTreeMap::from([("name".to_string(), Value::from("A"))]),
        });
        sites.refresh_schema();
        BTreeMap::from([(zones.id(), zones), (sites.id(), sites)])
    }

    fn ids(layers: &BTreeMap<String, Layer>) -> (String, String) {
        let zones = layers.values().find(|l| l.name == "zones").unwrap().id();
        let sites = layers.values().find(|l| l.name == "sites").unwrap().id();
        (zones, sites)
    }

    fn population_plan(zones: &str, sites: &str) -> ToolkitPlan {
        serde_json::from_value(json!({
            "goal": "地点から300m以内の人口",
            "steps": [
                {"id": "buf", "op": "buffer", "inputs": {"layer": sites}, "params": {"distance": "300 m"}},
                {"id": "pop", "op": "spatial_join", "inputs": {"target": "buf", "join": zones},
                 "params": {"aggregates": [{"op": "area_weighted_sum", "field": "pop", "as": "population"}]}}
            ]
        }))
        .unwrap()
    }

    #[test]
    fn runs_a_plan_through_the_command_bus() {
        let layers = store();
        let (zones, sites) = ids(&layers);
        let run = run_plan(&population_plan(&zones, &sites), &layers).unwrap();
        assert_eq!(run.steps.len(), 2);
        assert!(run.steps.iter().all(|s| s.checks.iter().all(|c| c.passed)));
        assert!(
            run.output.features[0].properties["population"]
                .as_f64()
                .unwrap()
                > 200.0
        );
        assert_eq!(
            run.output.provenance.workflow_digest.as_deref(),
            Some(run.workflow_digest.as_str())
        );
        assert_eq!(run.output.provenance.parents.len(), 2);
        assert_eq!(run.workflow.input_contracts.len(), 2);
        // Deterministic: the same plan over the same data yields the same digests.
        let again = run_plan(&population_plan(&zones, &sites), &layers).unwrap();
        assert_eq!(again.workflow_digest, run.workflow_digest);
        assert_eq!(again.result_digest, run.result_digest);
    }

    #[test]
    fn rejects_bad_plans_before_execution() {
        let layers = store();
        let (zones, sites) = ids(&layers);
        let mut plan = population_plan(&zones, &sites);
        plan.steps[1]
            .inputs
            .insert("join".into(), "lyr_doesnotexist".into());
        assert!(matches!(
            run_plan(&plan, &layers),
            Err(ToolkitError::UnknownReference(_))
        ));

        let mut plan = population_plan(&zones, &sites);
        plan.steps[0].op = "teleport".into();
        assert!(matches!(
            run_plan(&plan, &layers),
            Err(ToolkitError::Plan(_))
        ));

        let mut plan = population_plan(&zones, &sites);
        plan.output = Some("buf".into());
        assert!(
            run_plan(&plan, &layers).is_err(),
            "unused step must be rejected"
        );

        let mut plan = population_plan(&zones, &sites);
        plan.steps[0].params = json!({"distance": 300});
        let error = run_plan(&plan, &layers).unwrap_err();
        assert!(error.to_string().contains("unit"), "{error}");
    }

    #[test]
    fn imports_through_the_command_bus() {
        let csv = "name,lon,lat\nA,136.9,35.1\n";
        let (layer, receipt) =
            import_through_workflow("a.csv", csv.as_bytes(), &ImportOptions::default()).unwrap();
        assert_eq!(receipt.result_digest, layer.digest());
        assert_eq!(
            layer.provenance.workflow_digest.as_deref(),
            Some(receipt.workflow_digest.as_str())
        );
        assert!(receipt.layer.crs_needs_confirmation);
        let error =
            import_through_workflow("p.csv", b"x,y\n-26000,-92000\n", &ImportOptions::default())
                .unwrap_err();
        assert!(matches!(error, ToolkitError::CrsRequired(_)), "{error}");
    }
}
