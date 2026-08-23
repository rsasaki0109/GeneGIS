use crate::{
    IndependenceClass, QualityTolerance, VerificationPolicy, VERIFICATION_POLICY_SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

/// Schema version for [`VerificationGraph`].
pub const VERIFICATION_GRAPH_SCHEMA_VERSION: &str = "0.1.0";

fn default_schema_version() -> String {
    VERIFICATION_GRAPH_SCHEMA_VERSION.to_string()
}

/// Stable identity and independence declaration for a verifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifierIdentity {
    /// Stable verifier implementation identifier.
    pub verifier_id: String,
    /// Engine/package/container that runs the verifier.
    pub engine: String,
    /// Implementation/build digest or version.
    pub implementation: String,
    /// Declared relationship to the executor being checked.
    pub independence: IndependenceClass,
}

/// One check in a verification graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationNode {
    /// Stable check identifier, shared with policy and execution evidence.
    pub check_id: String,
    /// Human-readable claim being tested.
    pub claim: String,
    /// GeoContract identifiers whose meaning this claim relies on.
    #[serde(default)]
    pub subject_contracts: Vec<String>,
    /// Input/artifact/oracle references used by the verifier.
    #[serde(default)]
    pub evidence_inputs: Vec<String>,
    /// Verifier identity and independence declaration.
    pub verifier: VerifierIdentity,
    /// Numeric tolerance required by the check, when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tolerance: Option<QualityTolerance>,
    /// Other verification nodes that must complete first.
    #[serde(default)]
    pub depends_on: Vec<String>,
}

/// Explicit DAG of claims and independent verifiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationGraph {
    /// Verification graph document schema version.
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    /// Stable graph identifier/version.
    pub graph_id: String,
    /// Verification nodes; serialized order is not semantic.
    pub nodes: Vec<VerificationNode>,
}

impl VerificationGraph {
    /// Construct an empty graph for the current schema version.
    pub fn new(graph_id: impl Into<String>) -> Self {
        Self {
            schema_version: default_schema_version(),
            graph_id: graph_id.into(),
            nodes: Vec::new(),
        }
    }

    /// Validate identifiers, references, contracts, and acyclicity.
    pub fn validate(&self) -> Result<(), VerificationGraphError> {
        validate_graph(self)
    }

    /// Ensure every policy check is represented with compatible metadata.
    pub fn validate_against_policy(
        &self,
        policy: &VerificationPolicy,
    ) -> Result<(), VerificationGraphError> {
        self.validate()?;
        if policy.schema_version != VERIFICATION_POLICY_SCHEMA_VERSION {
            return Err(VerificationGraphError::PolicyMismatch(
                "unsupported policy schema".into(),
            ));
        }
        for requirement in &policy.required_checks {
            let node = self
                .nodes
                .iter()
                .find(|node| node.check_id == requirement.check_id)
                .ok_or_else(|| {
                    VerificationGraphError::PolicyMismatch(format!(
                        "required check {} is missing from graph",
                        requirement.check_id
                    ))
                })?;
            if !requirement
                .accepted_independence
                .contains(&node.verifier.independence)
            {
                return Err(VerificationGraphError::PolicyMismatch(format!(
                    "check {} independence is not accepted",
                    requirement.check_id
                )));
            }
            if let Some(maximum) = requirement.max_error_ppm {
                let graph_maximum = node
                    .tolerance
                    .as_ref()
                    .map(|tolerance| tolerance.max_error_ppm)
                    .ok_or_else(|| {
                        VerificationGraphError::PolicyMismatch(format!(
                            "check {} has no graph tolerance",
                            requirement.check_id
                        ))
                    })?;
                if graph_maximum > maximum {
                    return Err(VerificationGraphError::PolicyMismatch(format!(
                        "check {} graph tolerance is weaker than policy",
                        requirement.check_id
                    )));
                }
            }
        }
        Ok(())
    }

    /// Return node identifiers in deterministic topological order.
    pub fn topological_order(&self) -> Result<Vec<String>, VerificationGraphError> {
        topological_order(self)
    }

    /// Compute canonical SHA-256 identity excluding serialized node order.
    pub fn stable_digest(&self) -> Result<String, VerificationGraphError> {
        self.validate()?;
        let mut nodes = self.nodes.clone();
        nodes.sort_by(|left, right| left.check_id.cmp(&right.check_id));
        for node in &mut nodes {
            node.subject_contracts.sort();
            node.evidence_inputs.sort();
            node.depends_on.sort();
        }
        let value = serde_json::json!({
            "schema_version": self.schema_version,
            "graph_id": self.graph_id,
            "nodes": nodes,
        });
        let canonical = canonical_json(&value);
        Ok(format!("sha256:{:x}", Sha256::digest(canonical.as_bytes())))
    }
}

/// Failure raised while validating verification structure or policy binding.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VerificationGraphError {
    /// Graph schema version is unsupported.
    #[error("unsupported verification graph schema: {0}")]
    UnsupportedSchema(String),
    /// Graph or node field is empty.
    #[error("empty verification graph field: {0}")]
    EmptyField(&'static str),
    /// Check identifier occurs more than once.
    #[error("duplicate verification check: {0}")]
    DuplicateCheck(String),
    /// Dependency references an unknown check.
    #[error("verification check {check} references unknown dependency {dependency}")]
    UnknownDependency {
        /// Check with the invalid edge.
        check: String,
        /// Missing dependency.
        dependency: String,
    },
    /// A check depends on itself.
    #[error("verification check depends on itself: {0}")]
    SelfDependency(String),
    /// Dependency graph contains a cycle.
    #[error("verification graph contains a cycle: {0:?}")]
    Cycle(Vec<String>),
    /// A node repeats a contract/evidence/dependency reference.
    #[error("duplicate {field} value on {check}: {value}")]
    DuplicateReference {
        /// Check containing the duplicate.
        check: String,
        /// Reference collection.
        field: &'static str,
        /// Duplicate value.
        value: String,
    },
    /// Graph and release policy disagree.
    #[error("verification graph policy mismatch: {0}")]
    PolicyMismatch(String),
}

fn validate_graph(graph: &VerificationGraph) -> Result<(), VerificationGraphError> {
    if graph.schema_version != VERIFICATION_GRAPH_SCHEMA_VERSION {
        return Err(VerificationGraphError::UnsupportedSchema(
            graph.schema_version.clone(),
        ));
    }
    if graph.graph_id.trim().is_empty() {
        return Err(VerificationGraphError::EmptyField("graph_id"));
    }
    if graph.nodes.is_empty() {
        return Err(VerificationGraphError::EmptyField("nodes"));
    }
    let mut ids = BTreeSet::new();
    for node in &graph.nodes {
        if node.check_id.trim().is_empty() {
            return Err(VerificationGraphError::EmptyField("nodes.check_id"));
        }
        if !ids.insert(&node.check_id) {
            return Err(VerificationGraphError::DuplicateCheck(
                node.check_id.clone(),
            ));
        }
        for (field, value) in [
            ("nodes.claim", node.claim.as_str()),
            (
                "nodes.verifier.verifier_id",
                node.verifier.verifier_id.as_str(),
            ),
            ("nodes.verifier.engine", node.verifier.engine.as_str()),
            (
                "nodes.verifier.implementation",
                node.verifier.implementation.as_str(),
            ),
        ] {
            if value.trim().is_empty() {
                return Err(VerificationGraphError::EmptyField(field));
            }
        }
        unique_references(node, "subject_contracts", &node.subject_contracts)?;
        unique_references(node, "evidence_inputs", &node.evidence_inputs)?;
        unique_references(node, "depends_on", &node.depends_on)?;
        if let Some(tolerance) = &node.tolerance {
            if tolerance.metric.trim().is_empty() {
                return Err(VerificationGraphError::EmptyField("nodes.tolerance.metric"));
            }
        }
    }
    topological_order(graph).map(|_| ())
}

fn unique_references(
    node: &VerificationNode,
    field: &'static str,
    values: &[String],
) -> Result<(), VerificationGraphError> {
    let mut unique = BTreeSet::new();
    for value in values {
        if value.trim().is_empty() {
            return Err(VerificationGraphError::EmptyField("nodes.reference"));
        }
        if !unique.insert(value) {
            return Err(VerificationGraphError::DuplicateReference {
                check: node.check_id.clone(),
                field,
                value: value.clone(),
            });
        }
    }
    Ok(())
}

fn topological_order(graph: &VerificationGraph) -> Result<Vec<String>, VerificationGraphError> {
    let ids: BTreeSet<_> = graph
        .nodes
        .iter()
        .map(|node| node.check_id.clone())
        .collect();
    let mut indegree: BTreeMap<String, usize> = ids.iter().cloned().map(|id| (id, 0)).collect();
    let mut children: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for node in &graph.nodes {
        for dependency in &node.depends_on {
            if dependency == &node.check_id {
                return Err(VerificationGraphError::SelfDependency(
                    node.check_id.clone(),
                ));
            }
            if !ids.contains(dependency) {
                return Err(VerificationGraphError::UnknownDependency {
                    check: node.check_id.clone(),
                    dependency: dependency.clone(),
                });
            }
            if children
                .entry(dependency.clone())
                .or_default()
                .insert(node.check_id.clone())
            {
                *indegree.get_mut(&node.check_id).expect("known check") += 1;
            }
        }
    }
    let mut ready: BTreeSet<_> = indegree
        .iter()
        .filter_map(|(id, degree)| (*degree == 0).then_some(id.clone()))
        .collect();
    let mut order = Vec::with_capacity(ids.len());
    while let Some(id) = ready.pop_first() {
        order.push(id.clone());
        if let Some(next) = children.get(&id) {
            for child in next {
                let degree = indegree.get_mut(child).expect("known child");
                *degree -= 1;
                if *degree == 0 {
                    ready.insert(child.clone());
                }
            }
        }
    }
    if order.len() != ids.len() {
        return Err(VerificationGraphError::Cycle(
            indegree
                .into_iter()
                .filter_map(|(id, degree)| (degree > 0).then_some(id))
                .collect(),
        ));
    }
    Ok(order)
}

fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            let body = entries
                .into_iter()
                .map(|(key, value)| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(key).expect("serialize key"),
                        canonical_json(value)
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{body}}}")
        }
        serde_json::Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        _ => serde_json::to_string(value).expect("serialize scalar"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CheckRequirement, IndependenceClass};

    fn node(check_id: &str, dependencies: &[&str]) -> VerificationNode {
        VerificationNode {
            check_id: check_id.into(),
            claim: format!("claim {check_id}"),
            subject_contracts: vec!["nagoya-density".into()],
            evidence_inputs: vec!["oracle://nagoya-2020".into()],
            verifier: VerifierIdentity {
                verifier_id: "nagoya-oracle-v1".into(),
                engine: "genegis-analysis".into(),
                implementation:
                    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                independence: IndependenceClass::AuthoritativeExternalOracle,
            },
            tolerance: Some(QualityTolerance {
                metric: "relative_error".into(),
                max_error_ppm: 5_000,
            }),
            depends_on: dependencies.iter().map(|value| (*value).into()).collect(),
        }
    }

    #[test]
    fn validates_orders_and_hashes_independent_of_node_order() {
        let graph = VerificationGraph {
            schema_version: VERIFICATION_GRAPH_SCHEMA_VERSION.into(),
            graph_id: "nagoya-v1".into(),
            nodes: vec![node("density", &["area"]), node("area", &[])],
        };
        assert_eq!(
            graph.topological_order().expect("order"),
            ["area", "density"]
        );
        let mut reordered = graph.clone();
        reordered.nodes.reverse();
        assert_eq!(
            graph.stable_digest().expect("digest"),
            reordered.stable_digest().expect("digest")
        );
    }

    #[test]
    fn rejects_duplicate_unknown_self_and_cycle_edges() {
        let mut duplicate = VerificationGraph::new("duplicate");
        duplicate.nodes = vec![node("area", &[]), node("area", &[])];
        assert!(matches!(
            duplicate.validate(),
            Err(VerificationGraphError::DuplicateCheck(_))
        ));

        let mut unknown = VerificationGraph::new("unknown");
        unknown.nodes = vec![node("area", &["missing"])];
        assert!(matches!(
            unknown.validate(),
            Err(VerificationGraphError::UnknownDependency { .. })
        ));

        let mut self_edge = VerificationGraph::new("self");
        self_edge.nodes = vec![node("area", &["area"])];
        assert!(matches!(
            self_edge.validate(),
            Err(VerificationGraphError::SelfDependency(_))
        ));

        let mut cycle = VerificationGraph::new("cycle");
        cycle.nodes = vec![node("area", &["density"]), node("density", &["area"])];
        assert!(matches!(
            cycle.validate(),
            Err(VerificationGraphError::Cycle(_))
        ));
    }

    #[test]
    fn binds_check_independence_and_tolerance_to_policy() {
        let mut graph = VerificationGraph::new("nagoya-v1");
        graph.nodes = vec![node("area", &[])];
        let mut policy = VerificationPolicy::new("release-v1");
        policy.required_checks = vec![CheckRequirement {
            check_id: "area".into(),
            accepted_independence: BTreeSet::from([IndependenceClass::AuthoritativeExternalOracle]),
            max_error_ppm: Some(5_000),
        }];
        graph
            .validate_against_policy(&policy)
            .expect("matching graph and policy");

        graph.nodes[0].verifier.independence = IndependenceClass::SameImplementation;
        assert!(matches!(
            graph.validate_against_policy(&policy),
            Err(VerificationGraphError::PolicyMismatch(_))
        ));
    }
}
