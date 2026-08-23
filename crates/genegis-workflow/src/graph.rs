use genegis_contract::{
    AggregationBasis, AxisOrder, CoverageContract, GeoContract, GeometryKind, KeyUniqueness,
    MeasureContract, MeasureKind, MeasureTerm, NullPolicy, QualityContract, QualityTolerance,
    SourceContract, SpatialContract, TemporalContract, TemporalGranularity,
};
use genegis_crs::{CoordinateUnit, Crs, SourceSnapshot};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use thiserror::Error;
use uuid::Uuid;

use crate::{ReviewStatus, WorkflowDataRef, WorkflowNodeId, WorkflowPortContract, WorkflowStep};

/// CRS, unit, and source requirements for a graph input.
///
/// The legacy `inputs: Vec<Value>` field remains available for payloads and
/// compatibility. New graph edges refer to these contracts by name through a
/// [`WorkflowDataRef`] with `node: None`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowInputContract {
    /// Name used by graph input references.
    pub name: String,
    /// CRS required for a spatial input, if the value is spatial.
    #[serde(default)]
    pub crs: Option<Crs>,
    /// Coordinate-axis unit required by the CRS.
    #[serde(default)]
    pub coordinate_unit: Option<CoordinateUnit>,
    /// Unit of a non-coordinate value (for example `persons` or `km²`).
    #[serde(default)]
    pub value_unit: Option<String>,
    /// Immutable source identity used to reproduce this input.
    #[serde(default, alias = "source")]
    pub source_snapshot: Option<SourceSnapshot>,
    /// Full versioned semantic contract. Legacy fields above remain readable
    /// and must agree with this contract when both are present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geo_contract: Option<GeoContract>,
}

impl WorkflowInputContract {
    /// Construct a named input contract.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            crs: None,
            coordinate_unit: None,
            value_unit: None,
            source_snapshot: None,
            geo_contract: None,
        }
    }

    /// Attach a CRS and derive its coordinate unit.
    pub fn with_crs(mut self, crs: Crs) -> Self {
        self.coordinate_unit = Some(crs.coordinate_unit());
        self.crs = Some(crs);
        self
    }

    /// Attach an explicit coordinate unit.
    pub fn with_coordinate_unit(mut self, unit: CoordinateUnit) -> Self {
        self.coordinate_unit = Some(unit);
        self
    }

    /// Attach a value unit.
    pub fn with_value_unit(mut self, unit: impl Into<String>) -> Self {
        self.value_unit = Some(unit.into());
        self
    }

    /// Attach a source snapshot.
    pub fn with_source_snapshot(mut self, source: SourceSnapshot) -> Self {
        self.source_snapshot = Some(source);
        self
    }

    /// Attach a full geospatial semantic contract.
    pub fn with_geo_contract(mut self, contract: GeoContract) -> Self {
        self.geo_contract = Some(contract);
        self
    }
}

/// Validation failure raised before a workflow can be executed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WorkflowValidationError {
    /// The graph contains no executable nodes.
    #[error("workflow has no nodes")]
    EmptyGraph,
    /// Two nodes claim the same stable ID.
    #[error("duplicate workflow node ID: {id}")]
    DuplicateNodeId { id: String },
    /// A dependency points to the node itself.
    #[error("workflow node {node} depends on itself")]
    SelfDependency { node: String },
    /// A dependency points to a node not present in the graph.
    #[error("workflow node {node} has unresolved dependency {dependency}")]
    UnresolvedDependency { node: String, dependency: String },
    /// An input reference points to a node not present in the graph.
    #[error("workflow node {node} has unresolved input reference {dependency}")]
    UnresolvedInputReference { node: String, dependency: String },
    /// A node has an input reference but no corresponding dependency edge.
    #[error("workflow node {node} input reference {dependency} is missing a dependency edge")]
    InputWithoutDependency { node: String, dependency: String },
    /// An input reference points to a port that the producing node does not
    /// export.
    #[error("workflow node {node} references unknown input port {dependency}.{port}")]
    UnknownInputPort {
        node: String,
        dependency: String,
        port: String,
    },
    /// A node has a blank output port.
    #[error("workflow node {node} has an empty output port")]
    EmptyOutputPort { node: String },
    /// A new-schema node did not declare any output references.
    #[error("workflow node {node} has no output references")]
    MissingOutputReferences { node: String },
    /// A graph input reference has no matching contract.
    #[error("workflow node {node} references unknown input contract {input}")]
    UnknownInputContract { node: String, input: String },
    /// Two input contracts claim the same name.
    #[error("duplicate workflow input contract: {name}")]
    DuplicateInputContract { name: String },
    /// A CRS/unit/source contract is inconsistent or incomplete.
    #[error("invalid workflow input contract {input}: {reason}")]
    InvalidInputContract { input: String, reason: String },
    /// A semantic contract attached to a node output is invalid or references
    /// a port the node does not export.
    #[error("invalid workflow output contract {node}.{port}: {reason}")]
    InvalidOutputContract {
        /// Stable workflow node identifier.
        node: String,
        /// Exported output port.
        port: String,
        /// Structured validation explanation.
        reason: String,
    },
    /// An output reference points to a missing node.
    #[error("workflow output reference points to unresolved node {node}")]
    UnresolvedOutputReference { node: String },
    /// An output reference points to a port not exported by that node.
    #[error("workflow output reference {node}.{port} is not exported by the node")]
    UnknownOutputPort { node: String, port: String },
    /// The dependency graph contains a cycle.
    #[error("workflow dependency cycle detected involving: {nodes:?}")]
    Cycle { nodes: Vec<String> },
    /// More than one disconnected weak component exists.
    #[error("workflow contains disconnected nodes: {nodes:?}")]
    DisconnectedNodes { nodes: Vec<String> },
    /// A node cannot be reached from any graph root.
    #[error("workflow contains unreachable nodes: {nodes:?}")]
    UnreachableNodes { nodes: Vec<String> },
}

/// GeoWorkflow IR — goal, assumptions, steps, outputs, citations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoWorkflow {
    pub id: Uuid,
    pub goal: String,
    pub assumptions: Vec<String>,
    pub inputs: Vec<serde_json::Value>,
    /// Typed contracts for graph input references. Added with a serde default
    /// so workflows written before the DAG schema remain readable.
    #[serde(default)]
    pub input_contracts: Vec<WorkflowInputContract>,
    pub steps: Vec<WorkflowStep>,
    pub outputs: Vec<serde_json::Value>,
    /// Explicit graph outputs. The legacy `outputs` payload is retained for
    /// result metadata and compatibility.
    #[serde(default)]
    pub output_refs: Vec<WorkflowDataRef>,
    pub citations: Vec<Citation>,
    pub review_status: ReviewStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Citation {
    pub title: String,
    pub url: Option<String>,
    pub license: Option<String>,
    pub retrieved_at: Option<String>,
}

impl GeoWorkflow {
    pub fn new(goal: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            goal: goal.into(),
            assumptions: Vec::new(),
            inputs: Vec::new(),
            input_contracts: Vec::new(),
            steps: Vec::new(),
            outputs: Vec::new(),
            output_refs: Vec::new(),
            citations: Vec::new(),
            review_status: ReviewStatus::Draft,
        }
    }

    pub fn push_step(&mut self, step: WorkflowStep) {
        self.steps.push(step);
    }

    /// Add a typed input contract to the graph.
    pub fn add_input_contract(&mut self, contract: WorkflowInputContract) {
        self.input_contracts.push(contract);
    }

    /// Add an explicit graph output reference.
    pub fn add_output_ref(&mut self, output: WorkflowDataRef) {
        self.output_refs.push(output);
    }

    /// Validate the workflow graph before execution.
    pub fn validate(&self) -> Result<(), WorkflowValidationError> {
        self.topological_order().map(|_| ())
    }

    /// Return node IDs in deterministic topological order.
    pub fn topological_order(&self) -> Result<Vec<WorkflowNodeId>, WorkflowValidationError> {
        if self.steps.is_empty() {
            return Err(WorkflowValidationError::EmptyGraph);
        }

        let legacy_linear = self.is_legacy_linear();
        let mut node_ids = Vec::with_capacity(self.steps.len());
        let mut nodes = BTreeMap::new();
        for (index, step) in self.steps.iter().enumerate() {
            let node_id = step.node_id();
            if node_id.is_empty() {
                // `WorkflowStep::node_id` derives IDs for old JSON. This branch
                // is retained for malformed explicit IDs containing whitespace.
                return Err(WorkflowValidationError::UnresolvedDependency {
                    node: format!("index-{index}"),
                    dependency: "empty stable ID".into(),
                });
            }
            if nodes.insert(node_id.clone(), index).is_some() {
                return Err(WorkflowValidationError::DuplicateNodeId {
                    id: node_id.as_str().into(),
                });
            }
            node_ids.push(node_id);
        }

        self.validate_input_contracts()?;

        let mut dependencies: Vec<Vec<WorkflowNodeId>> = Vec::with_capacity(self.steps.len());
        let mut adjacency: BTreeMap<WorkflowNodeId, BTreeSet<WorkflowNodeId>> = BTreeMap::new();
        let mut indegree: BTreeMap<WorkflowNodeId, usize> =
            node_ids.iter().cloned().map(|node| (node, 0)).collect();

        for (index, step) in self.steps.iter().enumerate() {
            let node = &node_ids[index];
            let deps = if legacy_linear && index > 0 {
                vec![node_ids[index - 1].clone()]
            } else {
                step.depends_on.clone()
            };
            let inputs = if legacy_linear && index > 0 {
                vec![WorkflowDataRef::output(
                    node_ids[index - 1].clone(),
                    "result",
                )]
            } else {
                step.inputs.clone()
            };

            let mut unique_deps = BTreeSet::new();
            for dependency in &deps {
                if dependency == node {
                    return Err(WorkflowValidationError::SelfDependency {
                        node: node.as_str().into(),
                    });
                }
                if !nodes.contains_key(dependency) {
                    return Err(WorkflowValidationError::UnresolvedDependency {
                        node: node.as_str().into(),
                        dependency: dependency.as_str().into(),
                    });
                }
                if unique_deps.insert(dependency.clone()) {
                    adjacency
                        .entry(dependency.clone())
                        .or_default()
                        .insert(node.clone());
                    *indegree.get_mut(node).expect("node was inserted above") += 1;
                }
            }

            for input in &inputs {
                if input.port.trim().is_empty() {
                    return Err(WorkflowValidationError::UnknownInputContract {
                        node: node.as_str().into(),
                        input: "".into(),
                    });
                }
                match &input.node {
                    Some(dependency) => {
                        if dependency == node {
                            return Err(WorkflowValidationError::SelfDependency {
                                node: node.as_str().into(),
                            });
                        }
                        if !nodes.contains_key(dependency) {
                            return Err(WorkflowValidationError::UnresolvedInputReference {
                                node: node.as_str().into(),
                                dependency: dependency.as_str().into(),
                            });
                        }
                        if !unique_deps.contains(dependency) {
                            return Err(WorkflowValidationError::InputWithoutDependency {
                                node: node.as_str().into(),
                                dependency: dependency.as_str().into(),
                            });
                        }
                        if !legacy_linear
                            && !self.steps[nodes[dependency]].outputs.iter().any(|output| {
                                output.node.as_ref() == Some(dependency)
                                    && output.port == input.port
                            })
                        {
                            return Err(WorkflowValidationError::UnknownInputPort {
                                node: node.as_str().into(),
                                dependency: dependency.as_str().into(),
                                port: input.port.clone(),
                            });
                        }
                    }
                    None => {
                        if !self
                            .input_contracts
                            .iter()
                            .any(|contract| contract.name == input.port)
                        {
                            return Err(WorkflowValidationError::UnknownInputContract {
                                node: node.as_str().into(),
                                input: input.port.clone(),
                            });
                        }
                    }
                }
            }

            dependencies.push(deps);
        }

        let mut ready: BTreeSet<WorkflowNodeId> = indegree
            .iter()
            .filter_map(|(node, degree)| (*degree == 0).then_some(node.clone()))
            .collect();
        let mut order = Vec::with_capacity(node_ids.len());
        while let Some(node) = ready.pop_first() {
            order.push(node.clone());
            if let Some(children) = adjacency.get(&node) {
                for child in children {
                    let degree = indegree.get_mut(child).expect("child was inserted above");
                    *degree -= 1;
                    if *degree == 0 {
                        ready.insert(child.clone());
                    }
                }
            }
        }
        if order.len() != node_ids.len() {
            let cycle_nodes = indegree
                .into_iter()
                .filter_map(|(node, degree)| (degree > 0).then_some(node.as_str().to_string()))
                .collect();
            return Err(WorkflowValidationError::Cycle { nodes: cycle_nodes });
        }

        let roots: Vec<_> = dependencies
            .iter()
            .enumerate()
            .filter_map(|(index, deps)| deps.is_empty().then_some(node_ids[index].clone()))
            .collect();
        let mut reachable = BTreeSet::new();
        let mut queue: VecDeque<_> = roots.iter().cloned().collect();
        while let Some(node) = queue.pop_front() {
            if !reachable.insert(node.clone()) {
                continue;
            }
            if let Some(children) = adjacency.get(&node) {
                queue.extend(children.iter().cloned());
            }
        }
        if reachable.len() != node_ids.len() {
            return Err(WorkflowValidationError::UnreachableNodes {
                nodes: node_ids
                    .iter()
                    .filter(|node| !reachable.contains(*node))
                    .map(|node| node.as_str().to_string())
                    .collect(),
            });
        }

        // A graph may have several roots, but they must converge into one
        // connected workflow rather than silently executing independent graphs.
        let mut components = Vec::new();
        let mut unvisited: BTreeSet<_> = node_ids.iter().cloned().collect();
        while let Some(start) = unvisited.pop_first() {
            let mut component = BTreeSet::new();
            let mut pending = vec![start];
            while let Some(node) = pending.pop() {
                if !component.insert(node.clone()) {
                    continue;
                }
                unvisited.remove(&node);
                for dependency in &dependencies[nodes[&node]] {
                    pending.push(dependency.clone());
                }
                if let Some(children) = adjacency.get(&node) {
                    pending.extend(children.iter().cloned());
                }
            }
            components.push(component);
        }
        if components.len() > 1 {
            return Err(WorkflowValidationError::DisconnectedNodes {
                nodes: components
                    .into_iter()
                    .skip(1)
                    .flatten()
                    .map(|node| node.as_str().to_string())
                    .collect(),
            });
        }

        if !legacy_linear {
            for (index, step) in self.steps.iter().enumerate() {
                let node = &node_ids[index];
                if step.outputs.is_empty() {
                    return Err(WorkflowValidationError::MissingOutputReferences {
                        node: node.as_str().into(),
                    });
                }
                for output in &step.outputs {
                    if output.node.as_ref() != Some(node) || output.port.trim().is_empty() {
                        return Err(WorkflowValidationError::EmptyOutputPort {
                            node: node.as_str().into(),
                        });
                    }
                }
            }
        }
        self.validate_output_refs(&nodes, legacy_linear)?;
        self.validate_output_contracts(&node_ids, legacy_linear)?;
        Ok(order)
    }

    /// Compute a SHA-256 digest of the canonical, execution-independent graph.
    /// Runtime UUIDs, review status, and retrieval timestamps are omitted.
    pub fn stable_digest(&self) -> Result<String, WorkflowValidationError> {
        self.validate()?;
        Ok(self.stable_digest_unchecked())
    }

    /// Alias for callers that use the shorter digest terminology.
    pub fn digest(&self) -> Result<String, WorkflowValidationError> {
        self.stable_digest()
    }

    /// Compute the canonical digest after the caller has already validated the
    /// graph. This method is useful for receipts that need no second check.
    pub fn stable_digest_unchecked(&self) -> String {
        let legacy_linear = self.is_legacy_linear();
        let canonical_steps = self
            .steps
            .iter()
            .enumerate()
            .map(|(index, step)| {
                let node_id = step.node_id();
                let depends_on = if legacy_linear && index > 0 {
                    vec![node_id_for_step(&self.steps[index - 1])]
                } else {
                    step.depends_on.clone()
                };
                let inputs = if legacy_linear && index > 0 {
                    vec![WorkflowDataRef::output(
                        node_id_for_step(&self.steps[index - 1]),
                        "result",
                    )]
                } else {
                    step.inputs.clone()
                };
                let outputs = if legacy_linear {
                    vec![WorkflowDataRef::output(node_id.clone(), "result")]
                } else {
                    step.outputs.clone()
                };
                serde_json::json!({
                    "stable_id": node_id,
                    "operation": step.operation,
                    "parameters": step.parameters,
                    "expected_schema": step.expected_schema,
                    "validation": step.validation,
                    "provenance": step.provenance,
                    "depends_on": depends_on,
                    "inputs": inputs,
                    "outputs": outputs,
                    "output_contracts": step.output_contracts,
                })
            })
            .collect::<Vec<_>>();
        let output_refs = if self.output_refs.is_empty() && legacy_linear {
            self.steps
                .last()
                .map(|step| vec![WorkflowDataRef::output(step.node_id(), "result")])
                .unwrap_or_default()
        } else {
            self.output_refs.clone()
        };
        let mut document = serde_json::json!({
            "goal": self.goal,
            "assumptions": self.assumptions,
            "inputs": self.inputs,
            "input_contracts": self.input_contracts,
            "outputs": self.outputs,
            "output_refs": output_refs,
            "steps": canonical_steps,
            "citations": self.citations.iter().map(|citation| {
                serde_json::json!({
                    "title": citation.title,
                    "url": citation.url,
                    "license": citation.license,
                    // Retrieval is an execution event, not workflow identity.
                })
            }).collect::<Vec<_>>(),
        });
        if let Some(steps) = document
            .get_mut("steps")
            .and_then(serde_json::Value::as_array_mut)
        {
            steps.sort_by(|left, right| {
                left.get("stable_id")
                    .map(canonical_json)
                    .cmp(&right.get("stable_id").map(canonical_json))
            });
            for step in steps {
                for field in ["depends_on", "inputs", "outputs", "output_contracts"] {
                    if let Some(values) = step
                        .get_mut(field)
                        .and_then(serde_json::Value::as_array_mut)
                    {
                        values.sort_by_key(canonical_json);
                    }
                }
            }
        }
        for field in ["input_contracts", "output_refs", "citations"] {
            if let Some(values) = document
                .get_mut(field)
                .and_then(serde_json::Value::as_array_mut)
            {
                values.sort_by_key(canonical_json);
            }
        }
        // Source snapshots can carry an adapter retrieval event. Strip it from
        // the canonical identity recursively while preserving all stable source
        // fields (URI, license, version, expected/observed checksum, status).
        strip_runtime_fields(&mut document);
        let canonical = canonical_json(&document);
        let digest = Sha256::digest(canonical.as_bytes());
        format!("sha256:{}", hex(&digest))
    }

    fn is_legacy_linear(&self) -> bool {
        self.steps.iter().all(|step| {
            step.stable_id.trim().is_empty()
                && step.depends_on.is_empty()
                && step.inputs.is_empty()
                && step.outputs.is_empty()
        })
    }

    fn validate_input_contracts(&self) -> Result<(), WorkflowValidationError> {
        let mut names = BTreeSet::new();
        for contract in &self.input_contracts {
            if !names.insert(contract.name.clone()) {
                return Err(WorkflowValidationError::DuplicateInputContract {
                    name: contract.name.clone(),
                });
            }
            if contract.name.trim().is_empty() {
                return Err(WorkflowValidationError::InvalidInputContract {
                    input: contract.name.clone(),
                    reason: "name is empty".into(),
                });
            }
            if let Some(crs) = &contract.crs {
                let definition = crs.require_known().map_err(|error| {
                    WorkflowValidationError::InvalidInputContract {
                        input: contract.name.clone(),
                        reason: error.to_string(),
                    }
                })?;
                if contract.coordinate_unit != Some(definition.unit) {
                    return Err(WorkflowValidationError::InvalidInputContract {
                        input: contract.name.clone(),
                        reason: format!("coordinate unit must be {} for {}", definition.unit, crs),
                    });
                }
            }
            if contract.coordinate_unit == Some(CoordinateUnit::Unknown) {
                return Err(WorkflowValidationError::InvalidInputContract {
                    input: contract.name.clone(),
                    reason: "coordinate unit is unknown".into(),
                });
            }
            if contract
                .value_unit
                .as_deref()
                .is_some_and(|unit| unit.trim().is_empty())
            {
                return Err(WorkflowValidationError::InvalidInputContract {
                    input: contract.name.clone(),
                    reason: "value unit is empty".into(),
                });
            }
            if let Some(source) = &contract.source_snapshot {
                if source.uri.trim().is_empty() {
                    return Err(WorkflowValidationError::InvalidInputContract {
                        input: contract.name.clone(),
                        reason: "source URI is empty".into(),
                    });
                }
            }
            if let Some(geo_contract) = &contract.geo_contract {
                geo_contract.validate().map_err(|error| {
                    WorkflowValidationError::InvalidInputContract {
                        input: contract.name.clone(),
                        reason: error.to_string(),
                    }
                })?;
                if let (Some(legacy), Some(spatial)) = (&contract.crs, &geo_contract.spatial) {
                    if spatial.crs.as_ref() != Some(legacy) {
                        return Err(WorkflowValidationError::InvalidInputContract {
                            input: contract.name.clone(),
                            reason: "legacy CRS disagrees with GeoContract spatial CRS".into(),
                        });
                    }
                }
                if let (Some(legacy), Some(spatial)) =
                    (contract.coordinate_unit, &geo_contract.spatial)
                {
                    if spatial.coordinate_unit != legacy {
                        return Err(WorkflowValidationError::InvalidInputContract {
                            input: contract.name.clone(),
                            reason:
                                "legacy coordinate unit disagrees with GeoContract spatial unit"
                                    .into(),
                        });
                    }
                }
                if let (Some(legacy), Some(measure)) =
                    (contract.value_unit.as_deref(), &geo_contract.measure)
                {
                    if measure.unit != legacy {
                        return Err(WorkflowValidationError::InvalidInputContract {
                            input: contract.name.clone(),
                            reason: "legacy value unit disagrees with GeoContract measure unit"
                                .into(),
                        });
                    }
                }
                if let (Some(legacy), Some(source)) =
                    (&contract.source_snapshot, &geo_contract.source)
                {
                    if !same_source_contract_identity(legacy, &source.snapshot) {
                        return Err(WorkflowValidationError::InvalidInputContract {
                            input: contract.name.clone(),
                            reason: "legacy source snapshot disagrees with GeoContract source"
                                .into(),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    fn validate_output_contracts(
        &self,
        node_ids: &[WorkflowNodeId],
        legacy_linear: bool,
    ) -> Result<(), WorkflowValidationError> {
        for (index, step) in self.steps.iter().enumerate() {
            let node = &node_ids[index];
            let mut ports = BTreeSet::new();
            for output_contract in &step.output_contracts {
                if output_contract.port.trim().is_empty() {
                    return Err(WorkflowValidationError::InvalidOutputContract {
                        node: node.as_str().into(),
                        port: output_contract.port.clone(),
                        reason: "port is empty".into(),
                    });
                }
                if !ports.insert(output_contract.port.clone()) {
                    return Err(WorkflowValidationError::InvalidOutputContract {
                        node: node.as_str().into(),
                        port: output_contract.port.clone(),
                        reason: "duplicate output contract".into(),
                    });
                }
                if !legacy_linear
                    && !step.outputs.iter().any(|output| {
                        output.node.as_ref() == Some(node) && output.port == output_contract.port
                    })
                {
                    return Err(WorkflowValidationError::InvalidOutputContract {
                        node: node.as_str().into(),
                        port: output_contract.port.clone(),
                        reason: "contract does not name an exported output port".into(),
                    });
                }
                output_contract.contract.validate().map_err(|error| {
                    WorkflowValidationError::InvalidOutputContract {
                        node: node.as_str().into(),
                        port: output_contract.port.clone(),
                        reason: error.to_string(),
                    }
                })?;
            }
        }
        Ok(())
    }

    fn validate_output_refs(
        &self,
        nodes: &BTreeMap<WorkflowNodeId, usize>,
        legacy_linear: bool,
    ) -> Result<(), WorkflowValidationError> {
        if self.output_refs.is_empty() {
            // Old serialized workflows had only semantic output payloads. They
            // remain readable; migrated templates populate output_refs.
            return Ok(());
        }
        for output in &self.output_refs {
            let Some(node) = &output.node else {
                return Err(WorkflowValidationError::UnresolvedOutputReference {
                    node: "graph-input".into(),
                });
            };
            if !nodes.contains_key(node) {
                return Err(WorkflowValidationError::UnresolvedOutputReference {
                    node: node.as_str().into(),
                });
            }
            if legacy_linear {
                continue;
            }
            let step = &self.steps[nodes[node]];
            if !step.outputs.iter().any(|candidate| {
                candidate.node.as_ref() == Some(node) && candidate.port == output.port
            }) {
                return Err(WorkflowValidationError::UnknownOutputPort {
                    node: node.as_str().into(),
                    port: output.port.clone(),
                });
            }
        }
        Ok(())
    }
}

fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            let mut output = String::from("{");
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(&serde_json::to_string(key).expect("JSON key serialization"));
                output.push(':');
                output.push_str(&canonical_json(value));
            }
            output.push('}');
            output
        }
        serde_json::Value::Array(values) => {
            let values = values.iter().map(canonical_json).collect::<Vec<_>>();
            format!("[{}]", values.join(","))
        }
        _ => serde_json::to_string(value).expect("JSON scalar serialization"),
    }
}

fn node_id_for_step(step: &WorkflowStep) -> WorkflowNodeId {
    step.node_id()
}

fn strip_runtime_fields(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            map.remove("retrieved_at");
            for value in map.values_mut() {
                strip_runtime_fields(value);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                strip_runtime_fields(value);
            }
        }
        _ => {}
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn dag_step(
    stable_id: &'static str,
    operation: &'static str,
    parameters: serde_json::Value,
    dependencies: &[&'static str],
) -> WorkflowStep {
    WorkflowStep::named(stable_id, operation, parameters)
        .with_dependencies(dependencies.iter().copied())
        .with_inputs(
            dependencies
                .iter()
                .map(|dependency| WorkflowDataRef::output(*dependency, "result")),
        )
        .with_outputs([WorkflowDataRef::output(stable_id, "result")])
}

fn finish_dag(workflow: &mut GeoWorkflow, stable_id: &'static str) {
    workflow.output_refs = vec![WorkflowDataRef::output(stable_id, "result")];
}

/// Auditable endpoint registry mutation through Command + Workflow Graph.
pub fn stac_endpoint_registry_template(action: &str, endpoint_id: &str) -> GeoWorkflow {
    let mut workflow = GeoWorkflow::new(format!("{action} STAC endpoint {endpoint_id}"));
    workflow.input_contracts.push(
        WorkflowInputContract::new("search")
            .with_crs(Crs::wgs84())
            .with_value_unit("degrees")
            .with_source_snapshot(SourceSnapshot::new(format!("catalog://stac/{endpoint_id}"))),
    );
    workflow.inputs.push(serde_json::json!({
        "endpoint_id": endpoint_id,
        "crs": "EPSG:4326",
        "units": "degrees",
    }));
    let mut validate_stac_endpoint = dag_step(
        "validate-stac-endpoint",
        "ValidateStacEndpoint",
        serde_json::json!({ "endpoint_id": endpoint_id }),
        &[],
    );
    validate_stac_endpoint
        .inputs
        .push(WorkflowDataRef::input("search"));
    workflow.steps = vec![
        validate_stac_endpoint,
        dag_step(
            "apply-catalog-command",
            "ApplyCatalogCommand",
            serde_json::json!({ "action": action }),
            &["validate-stac-endpoint"],
        ),
        dag_step(
            "record-provenance",
            "RecordProvenance",
            serde_json::json!({}),
            &["apply-catalog-command"],
        ),
    ];
    finish_dag(&mut workflow, "record-provenance");
    workflow
}

/// Federated search workflow retaining request CRS, units, and source identity.
pub fn federated_stac_search_template(endpoint_ids: &[String]) -> GeoWorkflow {
    let mut workflow = GeoWorkflow::new("Search federated STAC endpoints");
    workflow.input_contracts.push(
        WorkflowInputContract::new("search")
            .with_crs(Crs::wgs84())
            .with_value_unit("degrees")
            .with_source_snapshot(SourceSnapshot::new("catalog://stac/federated")),
    );
    workflow.inputs.push(serde_json::json!({
        "endpoint_ids": endpoint_ids,
        "crs": "EPSG:4326",
        "units": "degrees",
    }));
    let mut resolve_stac_endpoints = dag_step(
        "resolve-stac-endpoints",
        "ResolveStacEndpoints",
        serde_json::json!({}),
        &[],
    );
    resolve_stac_endpoints
        .inputs
        .push(WorkflowDataRef::input("search"));
    workflow.steps = vec![
        resolve_stac_endpoints,
        dag_step(
            "search-stac-items",
            "SearchStacItems",
            serde_json::json!({ "method": "POST" }),
            &["resolve-stac-endpoints"],
        ),
        dag_step(
            "deduplicate-stac-items",
            "DeduplicateStacItems",
            serde_json::json!({}),
            &["search-stac-items"],
        ),
        dag_step(
            "record-provenance",
            "RecordProvenance",
            serde_json::json!({}),
            &["deduplicate-stac-items"],
        ),
    ];
    finish_dag(&mut workflow, "record-provenance");
    workflow
}

/// Cloud GeoParquet metadata probe or selected row-group execution.
pub fn remote_geoparquet_range_template(uri: &str, row_groups: Option<&[usize]>) -> GeoWorkflow {
    let mut workflow = GeoWorkflow::new("Read remote GeoParquet with HTTP ranges");
    workflow.input_contracts.push(
        WorkflowInputContract::new("asset")
            .with_value_unit("geoparquet")
            .with_source_snapshot(SourceSnapshot::from_uri(uri, None, None)),
    );
    workflow.inputs.push(serde_json::json!({
        "uri": uri,
        "row_groups": row_groups,
        "crs": "from GeoParquet metadata",
        "units": "from declared CRS",
    }));
    let mut probe_geoparquet_metadata = dag_step(
        "probe-geoparquet-metadata",
        "ProbeGeoParquetMetadata",
        serde_json::json!({ "read_mode": "http_range" }),
        &[],
    );
    probe_geoparquet_metadata
        .inputs
        .push(WorkflowDataRef::input("asset"));
    workflow.steps = vec![
        probe_geoparquet_metadata,
        dag_step(
            "select-row-groups",
            "SelectRowGroups",
            serde_json::json!({ "row_groups": row_groups }),
            &["probe-geoparquet-metadata"],
        ),
        dag_step(
            "decode-geoparquet",
            "DecodeGeoParquet",
            serde_json::json!({}),
            &["select-row-groups"],
        ),
        dag_step(
            "verify-geoparquet",
            "VerifyGeoParquet",
            serde_json::json!({ "schema": true, "crs": true, "source_coverage": true }),
            &["decode-geoparquet"],
        ),
        dag_step(
            "record-provenance",
            "RecordProvenance",
            serde_json::json!({}),
            &["verify-geoparquet"],
        ),
    ];
    finish_dag(&mut workflow, "record-provenance");
    workflow
}

/// Federated search through verified asset binding and range-read execution.
pub fn federated_asset_execution_template(
    endpoint_ids: &[String],
    stac_item_key: &str,
    asset_key: &str,
    uri: &str,
) -> GeoWorkflow {
    let mut workflow =
        GeoWorkflow::new("Search, compare, bind, execute, and verify a federated STAC asset");
    workflow.input_contracts = vec![
        WorkflowInputContract::new("search")
            .with_crs(Crs::wgs84())
            .with_value_unit("degrees")
            .with_source_snapshot(SourceSnapshot::new("catalog://stac/federated")),
        WorkflowInputContract::new("asset")
            .with_value_unit("geoparquet")
            .with_source_snapshot(SourceSnapshot::from_uri(uri, None, None)),
    ];
    workflow.inputs.push(serde_json::json!({
        "endpoint_ids": endpoint_ids,
        "stac_item_key": stac_item_key,
        "asset_key": asset_key,
        "uri": uri,
        "search_crs": "EPSG:4326",
        "search_units": "degrees",
    }));
    let mut enforce_remote_access_policy = dag_step(
        "enforce-remote-access-policy",
        "EnforceRemoteAccessPolicy",
        serde_json::json!({
            "allowed_hosts_env": "GENEGIS_REMOTE_ALLOWED_HOSTS",
            "allow_loopback": true,
            "max_response_bytes": 8388608,
            "timeout_ms": 15000,
            "max_redirects": 0,
            "url_credentials": false
        }),
        &[],
    );
    enforce_remote_access_policy.inputs.extend([
        WorkflowDataRef::input("search"),
        WorkflowDataRef::input("asset"),
    ]);
    workflow.steps = vec![
        enforce_remote_access_policy,
        dag_step(
            "search-stac-items",
            "SearchStacItems",
            serde_json::json!({ "method": "POST" }),
            &["enforce-remote-access-policy"],
        ),
        dag_step(
            "compare-asset-candidates",
            "CompareAssetCandidates",
            serde_json::json!({ "policy": "deterministic_verified_score" }),
            &["search-stac-items"],
        ),
        dag_step(
            "verify-asset-metadata",
            "VerifyAssetMetadata",
            serde_json::json!({
                "schema": true,
                "crs": true,
                "units": true,
                "license": true,
                "source_coverage": true
            }),
            &["compare-asset-candidates"],
        ),
        dag_step(
            "bind-stac-asset",
            "BindStacAsset",
            serde_json::json!({ "stac_item_key": stac_item_key, "asset_key": asset_key }),
            &["verify-asset-metadata"],
        ),
        dag_step(
            "read-geoparquet-ranges",
            "ReadGeoParquetRanges",
            serde_json::json!({ "read_mode": "http_range" }),
            &["bind-stac-asset"],
        ),
        dag_step(
            "verify-execution",
            "VerifyExecution",
            serde_json::json!({ "provenance": true }),
            &["read-geoparquet-ranges"],
        ),
        dag_step(
            "record-provenance",
            "RecordProvenance",
            serde_json::json!({}),
            &["verify-execution"],
        ),
    ];
    finish_dag(&mut workflow, "record-provenance");
    workflow
}

/// MVP north-star workflow: Nagoya population density.
pub fn nagoya_population_density_template() -> GeoWorkflow {
    let mut workflow = GeoWorkflow::new("名古屋市の人口密度を表示");
    workflow
        .assumptions
        .push("行政区域は ward または cho 粒度".into());
    let boundary_source = SourceSnapshot::new("catalog://genegis/nagoya-boundaries");
    let population_source = SourceSnapshot::new("catalog://genegis/nagoya-population");
    let mut boundary_source_contract = SourceContract::new(boundary_source.clone());
    boundary_source_contract.authority = Some("MLIT National Land Numerical Information".into());
    let mut population_source_contract = SourceContract::new(population_source.clone());
    population_source_contract.authority = Some("Nagoya City".into());

    let boundary_geo_contract = GeoContract::new("nagoya.boundary.2020")
        .with_spatial(SpatialContract::known(
            GeometryKind::Polygon,
            Crs::wgs84(),
            AxisOrder::LongitudeLatitude,
        ))
        .with_measure(MeasureContract::simple(
            MeasureKind::Geometry,
            "geometry",
            AggregationBasis::None,
        ))
        .with_temporal(TemporalContract {
            reference_period: "2020".into(),
            granularity: TemporalGranularity::Year,
            observed_at: None,
        })
        .with_coverage(nagoya_ward_coverage())
        .with_source(boundary_source_contract);
    let population_geo_contract = GeoContract::new("nagoya.population.2020")
        .with_measure(MeasureContract {
            kind: MeasureKind::Count,
            unit: "persons".into(),
            numerator: None,
            denominator: None,
            aggregation: AggregationBasis::Sum,
            population_universe: Some("2020 census population".into()),
        })
        .with_temporal(TemporalContract {
            reference_period: "2020".into(),
            granularity: TemporalGranularity::Year,
            observed_at: None,
        })
        .with_coverage(nagoya_ward_coverage())
        .with_source(population_source_contract);
    workflow.input_contracts = vec![
        WorkflowInputContract::new("boundary")
            .with_crs(Crs::wgs84())
            .with_value_unit("geometry")
            .with_source_snapshot(boundary_source)
            .with_geo_contract(boundary_geo_contract),
        WorkflowInputContract::new("population")
            .with_value_unit("persons")
            .with_source_snapshot(population_source)
            .with_geo_contract(population_geo_contract),
    ];
    let mut load_boundary = dag_step(
        "load-boundary",
        "LoadBoundary",
        serde_json::json!({}),
        &["find-boundary"],
    );
    load_boundary
        .inputs
        .push(WorkflowDataRef::input("boundary"));
    let mut load_population = dag_step(
        "load-population",
        "LoadPopulation",
        serde_json::json!({}),
        &["find-population"],
    );
    load_population
        .inputs
        .push(WorkflowDataRef::input("population"));
    let density_geo_contract = GeoContract::new("nagoya.population-density.2020")
        .with_spatial(SpatialContract::known(
            GeometryKind::Polygon,
            Crs::wgs84(),
            AxisOrder::LongitudeLatitude,
        ))
        .with_measure(MeasureContract {
            kind: MeasureKind::Density,
            unit: "persons/km2".into(),
            numerator: Some(MeasureTerm {
                kind: MeasureKind::Count,
                unit: "persons".into(),
            }),
            denominator: Some(MeasureTerm {
                kind: MeasureKind::Area,
                unit: "km2".into(),
            }),
            aggregation: AggregationBasis::RatioOfSums,
            population_universe: Some("2020 census population".into()),
        })
        .with_temporal(TemporalContract {
            reference_period: "2020".into(),
            granularity: TemporalGranularity::Year,
            observed_at: None,
        })
        .with_coverage(nagoya_ward_coverage())
        .with_quality(QualityContract {
            uncertainty: None,
            tolerances: vec![QualityTolerance {
                metric: "density_relative_error".into(),
                max_error_ppm: 5_000,
            }],
        });
    let calculate_density = dag_step(
        "calculate-density",
        "CalculateDensity",
        serde_json::json!({ "formula": "population / area_km2" }),
        &["join-population-to-geometry"],
    )
    .with_output_contracts([WorkflowPortContract::new("result", density_geo_contract)]);
    workflow.steps = vec![
        dag_step(
            "resolve-place",
            "ResolvePlace",
            serde_json::json!({ "name": "名古屋市" }),
            &[],
        ),
        dag_step(
            "find-boundary",
            "FindDataset",
            serde_json::json!({ "type": "admin_boundary", "area": "Nagoya" }),
            &["resolve-place"],
        ),
        dag_step(
            "find-population",
            "FindDataset",
            serde_json::json!({ "type": "population", "area": "Nagoya" }),
            &["resolve-place"],
        ),
        load_boundary,
        load_population,
        dag_step(
            "normalize-schema",
            "NormalizeSchema",
            serde_json::json!({}),
            &["load-boundary", "load-population"],
        ),
        dag_step(
            "reproject-for-area",
            "ReprojectForAreaCalculation",
            serde_json::json!({
                "method": "ellipsoidal_wgs84",
                "crs": "EPSG:4326",
                "area_unit": "km²"
            }),
            &["normalize-schema"],
        ),
        dag_step(
            "calculate-area-km2",
            "CalculateAreaKm2",
            serde_json::json!({}),
            &["reproject-for-area"],
        ),
        dag_step(
            "join-population-to-geometry",
            "JoinPopulationToGeometry",
            serde_json::json!({}),
            &["calculate-area-km2", "load-population"],
        ),
        calculate_density,
        dag_step(
            "generate-choropleth",
            "GenerateChoropleth",
            serde_json::json!({}),
            &["calculate-density"],
        ),
        dag_step(
            "verify-units",
            "VerifyUnits",
            serde_json::json!({}),
            &["generate-choropleth"],
        ),
        dag_step(
            "render-map",
            "RenderMap",
            serde_json::json!({}),
            &["verify-units"],
        ),
        dag_step(
            "attach-sources",
            "AttachSources",
            serde_json::json!({}),
            &["render-map"],
        ),
    ];
    finish_dag(&mut workflow, "attach-sources");
    workflow
}

fn nagoya_ward_coverage() -> CoverageContract {
    CoverageContract {
        scope: "JP-23/Nagoya wards".into(),
        expected_feature_count: Some(16),
        join_keys: vec!["ward_code".into()],
        key_uniqueness: KeyUniqueness::Unique,
        null_policy: NullPolicy::Reject,
    }
}

fn same_source_contract_identity(left: &SourceSnapshot, right: &SourceSnapshot) -> bool {
    left.dataset_id == right.dataset_id
        && left.uri == right.uri
        && left.license == right.license
        && left.expected_checksum == right.expected_checksum
        && left.observed_checksum == right.observed_checksum
        && left.source_version == right.source_version
        && left.checksum_status == right.checksum_status
}

/// Remote COG / GeoTIFF metadata probe workflow (catalog + HTTP range-read demo).
pub fn remote_cog_metadata_template() -> GeoWorkflow {
    let mut workflow = GeoWorkflow::new("リモートCOGデモのメタデータを表示");
    workflow
        .assumptions
        .push("Asset is fetched over HTTP range-read when remote".into());
    workflow.steps = vec![
        dag_step(
            "find-dataset",
            "FindDataset",
            serde_json::json!({ "tags": ["cog", "remote", "demo"] }),
            &[],
        ),
        dag_step(
            "probe-raster-metadata",
            "ProbeRasterMetadata",
            serde_json::json!({ "read_mode": "http_range" }),
            &["find-dataset"],
        ),
        dag_step(
            "summarize-cog-info",
            "SummarizeCogInfo",
            serde_json::json!({}),
            &["probe-raster-metadata"],
        ),
        dag_step(
            "attach-sources",
            "AttachSources",
            serde_json::json!({}),
            &["summarize-cog-info"],
        ),
    ];
    finish_dag(&mut workflow, "attach-sources");
    workflow
}

/// Local bundled COG metadata probe workflow (offline fixture).
pub fn local_cog_metadata_template() -> GeoWorkflow {
    let mut workflow = GeoWorkflow::new("ローカルCOGデモのメタデータを表示");
    workflow
        .assumptions
        .push("Asset is read from bundled smoke GeoTIFF fixture".into());
    workflow.steps = vec![
        dag_step(
            "find-dataset",
            "FindDataset",
            serde_json::json!({ "tags": ["cog", "local", "demo"] }),
            &[],
        ),
        dag_step(
            "probe-raster-metadata",
            "ProbeRasterMetadata",
            serde_json::json!({ "read_mode": "local" }),
            &["find-dataset"],
        ),
        dag_step(
            "summarize-cog-info",
            "SummarizeCogInfo",
            serde_json::json!({}),
            &["probe-raster-metadata"],
        ),
        dag_step(
            "attach-sources",
            "AttachSources",
            serde_json::json!({}),
            &["summarize-cog-info"],
        ),
    ];
    finish_dag(&mut workflow, "attach-sources");
    workflow
}

/// Nagoya GeoParquet read + feature-count verification workflow (Phase 9 alpha).
pub fn nagoya_geoparquet_template() -> GeoWorkflow {
    let mut workflow = GeoWorkflow::new("名古屋 wards GeoParquet を検証");
    workflow.input_contracts.push(
        WorkflowInputContract::new("wards")
            .with_crs(Crs::wgs84())
            .with_value_unit("persons")
            .with_source_snapshot(SourceSnapshot::new(
                "catalog://genegis/nagoya-wards-geoparquet",
            )),
    );
    workflow
        .assumptions
        .push("Bundled GeoParquet fixture with 16 Nagoya wards".into());
    let mut load_geoparquet = dag_step(
        "load-geoparquet",
        "LoadGeoParquet",
        serde_json::json!({ "format": "geoparquet" }),
        &["find-dataset"],
    );
    load_geoparquet.inputs.push(WorkflowDataRef::input("wards"));
    workflow.steps = vec![
        dag_step(
            "find-dataset",
            "FindDataset",
            serde_json::json!({ "tags": ["nagoya", "geoparquet", "demo"] }),
            &[],
        ),
        load_geoparquet,
        dag_step(
            "verify-feature-count",
            "VerifyFeatureCount",
            serde_json::json!({ "expected": 16, "field": "ward_name" }),
            &["load-geoparquet"],
        ),
        dag_step(
            "attach-sources",
            "AttachSources",
            serde_json::json!({}),
            &["verify-feature-count"],
        ),
    ];
    finish_dag(&mut workflow, "attach-sources");
    workflow
}

/// Nagoya GeoParquet population density choropleth workflow (Phase 9 beta).
pub fn nagoya_geoparquet_density_template() -> GeoWorkflow {
    let mut workflow = GeoWorkflow::new("名古屋 GeoParquet 人口密度を表示");
    workflow.input_contracts = vec![WorkflowInputContract::new("wards")
        .with_crs(Crs::wgs84())
        .with_value_unit("persons")
        .with_source_snapshot(SourceSnapshot::new(
            "catalog://genegis/nagoya-wards-geoparquet",
        ))];
    workflow
        .assumptions
        .push("Density computed from bundled GeoParquet wards fixture".into());
    let mut load_geoparquet = dag_step(
        "load-geoparquet",
        "LoadGeoParquet",
        serde_json::json!({ "format": "geoparquet" }),
        &["find-dataset"],
    );
    load_geoparquet.inputs.push(WorkflowDataRef::input("wards"));
    workflow.steps = vec![
        dag_step(
            "find-dataset",
            "FindDataset",
            serde_json::json!({ "tags": ["nagoya", "geoparquet", "density"] }),
            &[],
        ),
        load_geoparquet,
        dag_step(
            "calculate-area-km2",
            "CalculateAreaKm2",
            serde_json::json!({}),
            &["load-geoparquet"],
        ),
        dag_step(
            "calculate-density",
            "CalculateDensity",
            serde_json::json!({ "formula": "population / area_km2" }),
            &["calculate-area-km2"],
        ),
        dag_step(
            "generate-choropleth",
            "GenerateChoropleth",
            serde_json::json!({}),
            &["calculate-density"],
        ),
        dag_step(
            "verify-units",
            "VerifyUnits",
            serde_json::json!({}),
            &["generate-choropleth"],
        ),
        dag_step(
            "render-map",
            "RenderMap",
            serde_json::json!({}),
            &["verify-units"],
        ),
        dag_step(
            "attach-sources",
            "AttachSources",
            serde_json::json!({}),
            &["render-map"],
        ),
    ];
    finish_dag(&mut workflow, "attach-sources");
    workflow
}

/// External STAC collection fetch workflow (Phase 9 beta).
pub fn external_stac_fetch_template() -> GeoWorkflow {
    let mut workflow = GeoWorkflow::new("外部 STAC collection を fetch");
    workflow
        .assumptions
        .push("Collection URL is extracted from the user prompt".into());
    workflow.steps = vec![
        dag_step(
            "find-dataset",
            "FindDataset",
            serde_json::json!({ "tags": ["stac", "external", "demo"] }),
            &[],
        ),
        dag_step(
            "fetch-stac-collection",
            "FetchStacCollection",
            serde_json::json!({ "tool": "stac_fetch" }),
            &["find-dataset"],
        ),
        dag_step(
            "summarize-collection",
            "SummarizeCollection",
            serde_json::json!({}),
            &["fetch-stac-collection"],
        ),
        dag_step(
            "attach-sources",
            "AttachSources",
            serde_json::json!({}),
            &["summarize-collection"],
        ),
    ];
    finish_dag(&mut workflow, "attach-sources");
    workflow
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorkflowStep;

    fn all_templates() -> Vec<GeoWorkflow> {
        vec![
            stac_endpoint_registry_template("add", "demo"),
            federated_stac_search_template(&["one".into(), "two".into()]),
            remote_geoparquet_range_template("https://example.test/wards.parquet", Some(&[0, 2])),
            federated_asset_execution_template(
                &["one".into(), "two".into()],
                "item-1",
                "asset",
                "https://example.test/wards.parquet",
            ),
            nagoya_population_density_template(),
            remote_cog_metadata_template(),
            local_cog_metadata_template(),
            nagoya_geoparquet_template(),
            nagoya_geoparquet_density_template(),
            external_stac_fetch_template(),
        ]
    }

    #[test]
    fn migrated_templates_are_valid_dags() {
        for workflow in all_templates() {
            workflow
                .validate()
                .unwrap_or_else(|error| panic!("{}: {error}", workflow.goal));
        }
    }

    #[test]
    fn nagoya_order_and_digest_are_deterministic() {
        let first = nagoya_population_density_template();
        let second = nagoya_population_density_template();
        let order = first
            .topological_order()
            .expect("Nagoya graph has a valid topological order");
        let names: Vec<_> = order.iter().map(WorkflowNodeId::as_str).collect();
        assert_eq!(
            names,
            vec![
                "resolve-place",
                "find-boundary",
                "find-population",
                "load-boundary",
                "load-population",
                "normalize-schema",
                "reproject-for-area",
                "calculate-area-km2",
                "join-population-to-geometry",
                "calculate-density",
                "generate-choropleth",
                "verify-units",
                "render-map",
                "attach-sources",
            ]
        );
        assert_eq!(
            first.stable_digest().expect("digest"),
            second.stable_digest().expect("digest")
        );

        let mut execution_variant = second.clone();
        execution_variant.id = Uuid::nil();
        execution_variant.review_status = ReviewStatus::Approved;
        execution_variant.input_contracts[0]
            .source_snapshot
            .as_mut()
            .expect("source snapshot")
            .retrieved_at = Some("2026-08-22T00:00:00Z".into());
        assert_eq!(
            first.stable_digest().expect("digest"),
            execution_variant.stable_digest().expect("digest")
        );

        execution_variant.steps[0].parameters["name"] = serde_json::json!("愛知県");
        assert_ne!(
            first.stable_digest().expect("digest"),
            execution_variant.stable_digest().expect("digest")
        );
    }

    #[test]
    fn nagoya_template_exposes_versioned_input_and_density_contracts() {
        let workflow = nagoya_population_density_template();
        workflow.validate().expect("valid Nagoya contracts");
        assert_eq!(workflow.input_contracts.len(), 2);
        for input in &workflow.input_contracts {
            let contract = input.geo_contract.as_ref().expect("GeoContract");
            assert_eq!(
                contract.schema_version,
                genegis_contract::GEO_CONTRACT_SCHEMA_VERSION
            );
            assert!(contract.temporal.is_some());
            assert_eq!(
                contract
                    .coverage
                    .as_ref()
                    .and_then(|coverage| coverage.expected_feature_count),
                Some(16)
            );
        }
        let density = workflow
            .steps
            .iter()
            .find(|step| step.stable_id == "calculate-density")
            .expect("density node");
        let contract = &density.output_contracts[0].contract;
        assert_eq!(
            contract.measure.as_ref().map(|measure| measure.kind),
            Some(MeasureKind::Density)
        );
        assert_eq!(
            contract
                .measure
                .as_ref()
                .map(|measure| measure.unit.as_str()),
            Some("persons/km2")
        );
    }

    #[test]
    fn rejects_legacy_and_geo_contract_disagreement() {
        let mut workflow = nagoya_population_density_template();
        workflow.input_contracts[1].value_unit = Some("thousand_persons".into());
        assert!(matches!(
            workflow.validate(),
            Err(WorkflowValidationError::InvalidInputContract { .. })
        ));
    }

    #[test]
    fn rejects_output_contract_for_unexported_port() {
        let mut workflow = nagoya_population_density_template();
        let density = workflow
            .steps
            .iter_mut()
            .find(|step| step.stable_id == "calculate-density")
            .expect("density node");
        density.output_contracts[0].port = "not-exported".into();
        assert!(matches!(
            workflow.validate(),
            Err(WorkflowValidationError::InvalidOutputContract { .. })
        ));
    }

    #[test]
    fn old_linear_json_roundtrips_and_gets_a_deterministic_legacy_graph() {
        let json = serde_json::json!({
            "id": Uuid::nil(),
            "goal": "legacy",
            "assumptions": [],
            "inputs": [],
            "steps": [
                {
                    "id": Uuid::nil(),
                    "operation": "First",
                    "parameters": {},
                    "expected_schema": null,
                    "validation": null,
                    "provenance": null
                },
                {
                    "id": Uuid::nil(),
                    "operation": "Second",
                    "parameters": {},
                    "expected_schema": null,
                    "validation": null,
                    "provenance": null
                }
            ],
            "outputs": [],
            "citations": [],
            "review_status": "draft"
        });
        let workflow: GeoWorkflow = serde_json::from_value(json).expect("legacy JSON");
        workflow.validate().expect("legacy graph validation");
        assert_eq!(workflow.stable_digest(), workflow.stable_digest());
    }

    fn two_nodes(first: WorkflowStep, second: WorkflowStep) -> GeoWorkflow {
        GeoWorkflow {
            id: Uuid::nil(),
            goal: "invalid graph".into(),
            assumptions: vec![],
            inputs: vec![],
            input_contracts: vec![],
            steps: vec![first, second],
            outputs: vec![],
            output_refs: vec![],
            citations: vec![],
            review_status: ReviewStatus::Draft,
        }
    }

    #[test]
    fn rejects_duplicate_ids() {
        let first = WorkflowStep::named("same", "First", serde_json::json!({}));
        let second = WorkflowStep::named("same", "Second", serde_json::json!({}));
        assert!(matches!(
            two_nodes(first, second).validate(),
            Err(WorkflowValidationError::DuplicateNodeId { .. })
        ));
    }

    #[test]
    fn rejects_self_and_unresolved_dependencies() {
        let self_dependency = WorkflowStep::named("self", "Self", serde_json::json!({}))
            .with_dependencies(["self"])
            .with_inputs([WorkflowDataRef::output("self", "result")]);
        let other = WorkflowStep::named("other", "Other", serde_json::json!({}));
        assert!(matches!(
            two_nodes(self_dependency, other).validate(),
            Err(WorkflowValidationError::SelfDependency { .. })
        ));

        let unresolved = WorkflowStep::named("first", "First", serde_json::json!({}))
            .with_dependencies(["missing"])
            .with_inputs([WorkflowDataRef::output("missing", "result")]);
        let other = WorkflowStep::named("other", "Other", serde_json::json!({}));
        assert!(matches!(
            two_nodes(unresolved, other).validate(),
            Err(WorkflowValidationError::UnresolvedDependency { .. })
        ));
    }

    #[test]
    fn rejects_cycles_and_disconnected_nodes() {
        let first = dag_step("first", "First", serde_json::json!({}), &["second"]);
        let second = dag_step("second", "Second", serde_json::json!({}), &["first"]);
        assert!(matches!(
            two_nodes(first, second).validate(),
            Err(WorkflowValidationError::Cycle { .. })
        ));

        let first = dag_step("first", "First", serde_json::json!({}), &[]);
        let second = dag_step("second", "Second", serde_json::json!({}), &[]);
        assert!(matches!(
            two_nodes(first, second).validate(),
            Err(WorkflowValidationError::DisconnectedNodes { .. })
        ));
    }

    #[test]
    fn rejects_missing_input_contract_and_bad_crs_unit_contract() {
        let first = WorkflowStep::named("first", "First", serde_json::json!({}))
            .with_inputs([WorkflowDataRef::input("geometry")])
            .with_outputs([WorkflowDataRef::output("first", "result")]);
        let second = dag_step("second", "Second", serde_json::json!({}), &["first"]);
        assert!(matches!(
            two_nodes(first, second).validate(),
            Err(WorkflowValidationError::UnknownInputContract { .. })
        ));

        let mut workflow = two_nodes(
            dag_step("first", "First", serde_json::json!({}), &[]),
            dag_step("second", "Second", serde_json::json!({}), &["first"]),
        );
        workflow.input_contracts.push(
            WorkflowInputContract::new("geometry")
                .with_crs(Crs::wgs84())
                .with_coordinate_unit(CoordinateUnit::Metres),
        );
        workflow.steps[0]
            .inputs
            .push(WorkflowDataRef::input("geometry"));
        assert!(matches!(
            workflow.validate(),
            Err(WorkflowValidationError::InvalidInputContract { .. })
        ));

        let first = dag_step("first", "First", serde_json::json!({}), &[]);
        let second = WorkflowStep::named("second", "Second", serde_json::json!({}))
            .with_dependencies(["first"])
            .with_inputs([WorkflowDataRef::output("first", "missing")])
            .with_outputs([WorkflowDataRef::output("second", "result")]);
        assert!(matches!(
            two_nodes(first, second).validate(),
            Err(WorkflowValidationError::UnknownInputPort { .. })
        ));
    }
}
