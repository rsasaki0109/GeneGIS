use genegis_contract::GeoContract;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Stable identifier for a node in a [`GeoWorkflow`](crate::GeoWorkflow).
///
/// This identifier is part of the workflow IR and is therefore deliberately
/// independent from the UUID assigned to a particular execution instance.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkflowNodeId(pub String);

impl WorkflowNodeId {
    /// Construct a node identifier without changing its spelling.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Return the identifier as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Return whether this is an empty identifier.
    pub fn is_empty(&self) -> bool {
        self.0.trim().is_empty()
    }
}

impl Default for WorkflowNodeId {
    fn default() -> Self {
        Self(String::new())
    }
}

impl From<&str> for WorkflowNodeId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for WorkflowNodeId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl AsRef<str> for WorkflowNodeId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// A reference to a value produced by another workflow node or supplied as a
/// workflow input. `node: None` denotes a graph input and uses `port` as the
/// input contract name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkflowDataRef {
    /// Producing node. `None` means the reference is a graph input.
    #[serde(default)]
    pub node: Option<WorkflowNodeId>,
    /// Port name on the producing node, or input-contract name for a graph
    /// input.
    pub port: String,
}

/// Semantic contract attached to a named workflow value port.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowPortContract {
    /// Port name exported by or supplied to a node.
    pub port: String,
    /// Versioned geospatial semantic contract for the value.
    pub contract: GeoContract,
}

impl WorkflowPortContract {
    /// Construct a semantic contract for a port.
    pub fn new(port: impl Into<String>, contract: GeoContract) -> Self {
        Self {
            port: port.into(),
            contract,
        }
    }
}

impl WorkflowDataRef {
    /// Create a reference to a graph input contract.
    pub fn input(port: impl Into<String>) -> Self {
        Self {
            node: None,
            port: port.into(),
        }
    }

    /// Create a reference to a node output.
    pub fn output(node: impl Into<WorkflowNodeId>, port: impl Into<String>) -> Self {
        Self {
            node: Some(node.into()),
            port: port.into(),
        }
    }

    /// Return whether this reference names a graph input.
    pub fn is_graph_input(&self) -> bool {
        self.node.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkflowStepId(pub Uuid);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    /// Runtime identity of this step instance. It is intentionally excluded
    /// from the stable workflow digest.
    pub id: WorkflowStepId,
    /// Deterministic identity of this node in the workflow graph.
    #[serde(default)]
    pub stable_id: String,
    pub operation: String,
    pub parameters: serde_json::Value,
    pub expected_schema: Option<serde_json::Value>,
    pub validation: Option<serde_json::Value>,
    pub provenance: Option<String>,
    /// Explicit dependency edges. Data references below must point to the
    /// same producing nodes.
    #[serde(default)]
    pub depends_on: Vec<WorkflowNodeId>,
    /// Explicit input value references.
    #[serde(default)]
    pub inputs: Vec<WorkflowDataRef>,
    /// Explicit output value references exposed by this node.
    #[serde(default)]
    pub outputs: Vec<WorkflowDataRef>,
    /// Semantic contracts for exported output ports.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_contracts: Vec<WorkflowPortContract>,
}

impl WorkflowStep {
    /// Construct a step with a deterministic stable ID derived from the
    /// operation and parameters.
    ///
    /// Call [`WorkflowStep::named`] when the operation occurs more than once
    /// with identical parameters; otherwise those nodes intentionally share a
    /// stable ID and graph validation will reject the duplicate.
    pub fn new(operation: impl Into<String>, parameters: serde_json::Value) -> Self {
        let operation = operation.into();
        let stable_id = generated_stable_id(&operation, &parameters);
        Self {
            id: WorkflowStepId(Uuid::new_v4()),
            stable_id,
            operation,
            parameters,
            expected_schema: None,
            validation: None,
            provenance: None,
            depends_on: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            output_contracts: Vec::new(),
        }
    }

    /// Construct a step with an explicit stable graph-node ID.
    pub fn named(
        stable_id: impl Into<String>,
        operation: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        let mut step = Self::new(operation, parameters);
        step.stable_id = stable_id.into();
        step
    }

    /// Set the explicit dependency edges for this node.
    pub fn with_dependencies<I, T>(mut self, dependencies: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<WorkflowNodeId>,
    {
        self.depends_on = dependencies.into_iter().map(Into::into).collect();
        self
    }

    /// Set explicit input references for this node.
    pub fn with_inputs(mut self, inputs: impl IntoIterator<Item = WorkflowDataRef>) -> Self {
        self.inputs = inputs.into_iter().collect();
        self
    }

    /// Set explicit output references for this node.
    pub fn with_outputs(mut self, outputs: impl IntoIterator<Item = WorkflowDataRef>) -> Self {
        self.outputs = outputs.into_iter().collect();
        self
    }

    /// Attach semantic contracts to exported output ports.
    pub fn with_output_contracts(
        mut self,
        contracts: impl IntoIterator<Item = WorkflowPortContract>,
    ) -> Self {
        self.output_contracts = contracts.into_iter().collect();
        self
    }

    /// Return the stable node identifier.
    pub fn node_id(&self) -> WorkflowNodeId {
        if self.stable_id.trim().is_empty() {
            WorkflowNodeId::new(generated_stable_id(&self.operation, &self.parameters))
        } else {
            WorkflowNodeId::new(self.stable_id.clone())
        }
    }
}

fn generated_stable_id(operation: &str, parameters: &serde_json::Value) -> String {
    let canonical_parameters = canonical_json(parameters);
    let mut digest = Sha256::new();
    digest.update(operation.as_bytes());
    digest.update([0]);
    digest.update(canonical_parameters.as_bytes());
    let digest = digest.finalize();
    let operation = operation
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    format!(
        "{}-{}",
        if operation.is_empty() {
            "node"
        } else {
            &operation
        },
        hex(&digest[..8])
    )
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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
