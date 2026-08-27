//! Partition-aware incremental Workflow Graph scheduling and receipts.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use chrono::DateTime;
use genegis_workflow::{GeoWorkflow, WorkflowStep};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Incremental scheduler receipt schema version.
pub const INCREMENTAL_RECEIPT_SCHEMA_VERSION: &str = "0.1.0";

/// Stable partition identity within a workflow input/node.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PartitionKey(pub String);

/// Exact cursor and event-time window entering one incremental run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IncrementalInputWindow {
    /// Affected partition.
    pub partition: PartitionKey,
    /// Exclusive cursor before the window.
    pub cursor_start: u64,
    /// Inclusive cursor committed by the window.
    pub cursor_end: u64,
    /// Previous committed event-time watermark.
    pub watermark_start: String,
    /// Event-time watermark committed by the window.
    pub watermark_end: String,
}

/// Why an input partition changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum IncrementalChangeKind {
    /// New observations after the committed cursor.
    Append,
    /// Accepted late observations within the lateness policy.
    Late {
        /// Seconds behind the new watermark.
        lateness_seconds: u64,
    },
    /// A provider correction replacing an earlier immutable snapshot.
    Replacement {
        /// Exact snapshot being superseded.
        replaced_snapshot_digest: String,
    },
}

/// One immutable changed input partition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IncrementalChange {
    /// Workflow graph input name.
    pub input_name: String,
    /// Exact input window.
    pub window: IncrementalInputWindow,
    /// New immutable input snapshot identity.
    pub snapshot_digest: String,
    /// Append, late arrival, or replacement semantics.
    pub change: IncrementalChangeKind,
}

/// One node/partition selected for recomputation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IncrementalDecision {
    /// Stable workflow node identity.
    pub node_id: String,
    /// Affected partition.
    pub partition: PartitionKey,
    /// Graph reason for invalidation.
    pub reason: String,
}

/// One append-only scheduler event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum IncrementalEvent {
    /// Exact input window admitted.
    WindowAdmitted { window: IncrementalInputWindow },
    /// Late data was accepted and will invalidate downstream partitions.
    LateDataAccepted {
        partition: PartitionKey,
        lateness_seconds: u64,
    },
    /// An immutable input snapshot superseded another.
    InputReplacement {
        partition: PartitionKey,
        replaced_snapshot_digest: String,
        replacement_snapshot_digest: String,
    },
    /// One node/partition execution attempt started.
    Attempt {
        node_id: String,
        partition: PartitionKey,
        attempt: u32,
    },
    /// A failed attempt will be retried.
    Retry {
        node_id: String,
        partition: PartitionKey,
        failed_attempt: u32,
        error: String,
    },
    /// A node/partition output was committed.
    OutputCommitted {
        node_id: String,
        partition: PartitionKey,
        output_digest: String,
        replaced_output_digest: Option<String>,
    },
    /// Retry budget was exhausted without committing output.
    Failed {
        node_id: String,
        partition: PartitionKey,
        attempts: u32,
        error: String,
    },
}

/// Complete incremental planning/execution receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IncrementalRunReceipt {
    /// Receipt schema version.
    pub schema_version: String,
    /// Exact Workflow Graph digest.
    pub workflow_digest: String,
    /// Exact changed-input identity.
    pub change_digest: String,
    /// Affected node/partition decisions in topological order.
    pub decisions: Vec<IncrementalDecision>,
    /// Append-only window, late/replacement, retry, and output events.
    pub events: Vec<IncrementalEvent>,
    /// Whether every selected node/partition committed successfully.
    pub completed: bool,
    /// Digest of all scheduler partition state after this run.
    pub state_digest: String,
}

/// Fail-closed incremental scheduler error.
#[derive(Debug, Error)]
pub enum IncrementalError {
    /// Workflow or change contract is invalid.
    #[error("invalid incremental workflow change: {0}")]
    Invalid(String),
    /// Stable digest serialization failed.
    #[error("incremental digest failed: {0}")]
    Digest(#[from] serde_json::Error),
}

/// In-memory partition state; durable stores can persist this serialized form.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IncrementalScheduler {
    /// Last committed output digest by node and partition.
    outputs: BTreeMap<String, BTreeMap<PartitionKey, String>>,
    /// Append-only scheduler history.
    history: Vec<IncrementalRunReceipt>,
}

impl IncrementalScheduler {
    /// Return a committed node/partition output identity.
    pub fn output_digest(&self, node_id: &str, partition: &PartitionKey) -> Option<&str> {
        self.outputs
            .get(node_id)?
            .get(partition)
            .map(String::as_str)
    }

    /// Return append-only run history.
    pub fn history(&self) -> &[IncrementalRunReceipt] {
        &self.history
    }

    /// Plan and execute only node partitions affected by one changed input.
    ///
    /// The executor receives the node, partition, and 1-based attempt number.
    /// Returning an error consumes one retry; no downstream node runs after an
    /// exhausted retry budget.
    pub fn execute<F>(
        &mut self,
        workflow: &GeoWorkflow,
        change: IncrementalChange,
        max_retries: u32,
        mut executor: F,
    ) -> Result<IncrementalRunReceipt, IncrementalError>
    where
        F: FnMut(&WorkflowStep, &PartitionKey, u32) -> Result<String, String>,
    {
        workflow
            .validate()
            .map_err(|error| IncrementalError::Invalid(error.to_string()))?;
        validate_change(workflow, &change)?;
        let decisions = affected_decisions(workflow, &change)?;
        let workflow_digest = workflow
            .stable_digest()
            .map_err(|error| IncrementalError::Invalid(error.to_string()))?;
        let change_digest = digest(&change)?;
        let mut events = vec![IncrementalEvent::WindowAdmitted {
            window: change.window.clone(),
        }];
        match &change.change {
            IncrementalChangeKind::Append => {}
            IncrementalChangeKind::Late { lateness_seconds } => {
                events.push(IncrementalEvent::LateDataAccepted {
                    partition: change.window.partition.clone(),
                    lateness_seconds: *lateness_seconds,
                });
            }
            IncrementalChangeKind::Replacement {
                replaced_snapshot_digest,
            } => events.push(IncrementalEvent::InputReplacement {
                partition: change.window.partition.clone(),
                replaced_snapshot_digest: replaced_snapshot_digest.clone(),
                replacement_snapshot_digest: change.snapshot_digest.clone(),
            }),
        }

        let mut completed = true;
        for decision in &decisions {
            let node = workflow
                .steps
                .iter()
                .find(|node| node.stable_id == decision.node_id)
                .expect("planned node exists");
            let mut committed = None;
            let maximum_attempts = max_retries.saturating_add(1);
            for attempt in 1..=maximum_attempts {
                events.push(IncrementalEvent::Attempt {
                    node_id: decision.node_id.clone(),
                    partition: decision.partition.clone(),
                    attempt,
                });
                match executor(node, &decision.partition, attempt) {
                    Ok(output_digest) => {
                        require_digest(&output_digest, "output digest")?;
                        committed = Some(output_digest);
                        break;
                    }
                    Err(error) if attempt < maximum_attempts => {
                        events.push(IncrementalEvent::Retry {
                            node_id: decision.node_id.clone(),
                            partition: decision.partition.clone(),
                            failed_attempt: attempt,
                            error,
                        });
                    }
                    Err(error) => {
                        events.push(IncrementalEvent::Failed {
                            node_id: decision.node_id.clone(),
                            partition: decision.partition.clone(),
                            attempts: attempt,
                            error,
                        });
                    }
                }
            }
            let Some(output_digest) = committed else {
                completed = false;
                break;
            };
            let replaced_output_digest = self
                .outputs
                .entry(decision.node_id.clone())
                .or_default()
                .insert(decision.partition.clone(), output_digest.clone());
            events.push(IncrementalEvent::OutputCommitted {
                node_id: decision.node_id.clone(),
                partition: decision.partition.clone(),
                output_digest,
                replaced_output_digest,
            });
        }
        let state_digest = digest(&self.outputs)?;
        let receipt = IncrementalRunReceipt {
            schema_version: INCREMENTAL_RECEIPT_SCHEMA_VERSION.into(),
            workflow_digest,
            change_digest,
            decisions,
            events,
            completed,
            state_digest,
        };
        self.history.push(receipt.clone());
        Ok(receipt)
    }
}

fn validate_change(
    workflow: &GeoWorkflow,
    change: &IncrementalChange,
) -> Result<(), IncrementalError> {
    if change.input_name.trim().is_empty()
        || change.window.partition.0.trim().is_empty()
        || !workflow
            .input_contracts
            .iter()
            .any(|input| input.name == change.input_name)
        || change.window.cursor_end < change.window.cursor_start
    {
        return Err(IncrementalError::Invalid(
            "input, partition, or cursor window is invalid".into(),
        ));
    }
    require_digest(&change.snapshot_digest, "snapshot digest")?;
    let start = DateTime::parse_from_rfc3339(&change.window.watermark_start)
        .map_err(|_| IncrementalError::Invalid("invalid start watermark".into()))?;
    let end = DateTime::parse_from_rfc3339(&change.window.watermark_end)
        .map_err(|_| IncrementalError::Invalid("invalid end watermark".into()))?;
    if end < start {
        return Err(IncrementalError::Invalid("watermark regressed".into()));
    }
    if let IncrementalChangeKind::Replacement {
        replaced_snapshot_digest,
    } = &change.change
    {
        require_digest(replaced_snapshot_digest, "replaced snapshot digest")?;
        if replaced_snapshot_digest == &change.snapshot_digest {
            return Err(IncrementalError::Invalid(
                "replacement snapshot must differ".into(),
            ));
        }
    }
    Ok(())
}

fn affected_decisions(
    workflow: &GeoWorkflow,
    change: &IncrementalChange,
) -> Result<Vec<IncrementalDecision>, IncrementalError> {
    let direct = workflow
        .steps
        .iter()
        .filter(|step| {
            step.inputs
                .iter()
                .any(|input| input.node.is_none() && input.port == change.input_name)
        })
        .map(|step| step.stable_id.clone())
        .collect::<BTreeSet<_>>();
    if direct.is_empty() {
        return Err(IncrementalError::Invalid(
            "no workflow node consumes changed input".into(),
        ));
    }
    let mut affected = direct.clone();
    let mut queue = direct.into_iter().collect::<VecDeque<_>>();
    while let Some(source) = queue.pop_front() {
        for downstream in workflow.steps.iter().filter(|step| {
            step.depends_on
                .iter()
                .any(|dependency| dependency.as_str() == source)
        }) {
            if affected.insert(downstream.stable_id.clone()) {
                queue.push_back(downstream.stable_id.clone());
            }
        }
    }
    let reason = match change.change {
        IncrementalChangeKind::Append => "input_append",
        IncrementalChangeKind::Late { .. } => "late_data_invalidation",
        IncrementalChangeKind::Replacement { .. } => "replacement_invalidation",
    };
    let mut remaining = affected;
    let mut ordered = Vec::new();
    while !remaining.is_empty() {
        let ready = workflow
            .steps
            .iter()
            .filter(|step| remaining.contains(&step.stable_id))
            .find(|step| {
                step.depends_on.iter().all(|dependency| {
                    !remaining.contains(dependency.as_str())
                        || ordered.iter().any(|decision: &IncrementalDecision| {
                            decision.node_id == dependency.as_str()
                        })
                })
            })
            .map(|step| step.stable_id.clone())
            .ok_or_else(|| IncrementalError::Invalid("affected graph is cyclic".into()))?;
        remaining.remove(&ready);
        ordered.push(IncrementalDecision {
            node_id: ready,
            partition: change.window.partition.clone(),
            reason: reason.into(),
        });
    }
    Ok(ordered)
}

fn require_digest(value: &str, label: &str) -> Result<(), IncrementalError> {
    if value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        Ok(())
    } else {
        Err(IncrementalError::Invalid(format!("invalid {label}")))
    }
}

fn digest(value: &impl Serialize) -> Result<String, serde_json::Error> {
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(value)?)
    ))
}

#[cfg(test)]
mod tests {
    use genegis_crs::SourceSnapshot;
    use genegis_workflow::live_feed_ingest_template;

    use super::*;

    fn hash(value: &str) -> String {
        format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
    }

    fn change(partition: &str, kind: IncrementalChangeKind) -> IncrementalChange {
        IncrementalChange {
            input_name: "live-feed".into(),
            window: IncrementalInputWindow {
                partition: PartitionKey(partition.into()),
                cursor_start: 10,
                cursor_end: 11,
                watermark_start: "2026-08-26T10:00:00Z".into(),
                watermark_end: "2026-08-26T10:05:00Z".into(),
            },
            snapshot_digest: hash(&format!("snapshot-{partition}")),
            change: kind,
        }
    }

    #[test]
    fn recomputes_only_affected_partition_and_records_retry() {
        let workflow = live_feed_ingest_template(
            "sensor",
            "fixture",
            SourceSnapshot::new("fixture://sensor"),
            10,
            "2026-08-26T10:00:00Z",
            10,
        );
        let mut scheduler = IncrementalScheduler::default();
        let first = scheduler
            .execute(
                &workflow,
                change("ward-23101", IncrementalChangeKind::Append),
                1,
                |node, partition, attempt| {
                    if node.stable_id == "validate-live-feed-window" && attempt == 1 {
                        Err("transient verifier unavailable".into())
                    } else {
                        Ok(hash(&format!(
                            "{}:{}:{attempt}",
                            node.stable_id, partition.0
                        )))
                    }
                },
            )
            .expect("run");
        assert!(first.completed);
        assert!(first
            .events
            .iter()
            .any(|event| matches!(event, IncrementalEvent::Retry { .. })));
        assert!(first
            .decisions
            .iter()
            .all(|decision| decision.partition.0 == "ward-23101"));

        let other = scheduler
            .execute(
                &workflow,
                change("ward-23102", IncrementalChangeKind::Append),
                0,
                |node, partition, _| Ok(hash(&format!("{}:{}", node.stable_id, partition.0))),
            )
            .expect("other partition");
        assert!(other.completed);
        let first_output = scheduler
            .output_digest(
                "commit-live-feed-cursor",
                &PartitionKey("ward-23101".into()),
            )
            .expect("first partition")
            .to_string();
        assert_ne!(
            first_output,
            scheduler
                .output_digest(
                    "commit-live-feed-cursor",
                    &PartitionKey("ward-23102".into())
                )
                .expect("second partition")
        );
    }

    #[test]
    fn records_late_and_replacement_invalidation_with_replaced_outputs() {
        let workflow = live_feed_ingest_template(
            "hazard",
            "fixture",
            SourceSnapshot::new("fixture://hazard"),
            10,
            "2026-08-26T10:00:00Z",
            10,
        );
        let mut scheduler = IncrementalScheduler::default();
        scheduler
            .execute(
                &workflow,
                change(
                    "mesh-5235",
                    IncrementalChangeKind::Late {
                        lateness_seconds: 120,
                    },
                ),
                0,
                |node, _, _| Ok(hash(&format!("late:{}", node.stable_id))),
            )
            .expect("late");
        let replacement = scheduler
            .execute(
                &workflow,
                change(
                    "mesh-5235",
                    IncrementalChangeKind::Replacement {
                        replaced_snapshot_digest: hash("old-snapshot"),
                    },
                ),
                0,
                |node, _, _| Ok(hash(&format!("replacement:{}", node.stable_id))),
            )
            .expect("replacement");
        assert!(replacement
            .events
            .iter()
            .any(|event| matches!(event, IncrementalEvent::InputReplacement { .. })));
        assert!(replacement.events.iter().any(|event| matches!(
            event,
            IncrementalEvent::OutputCommitted {
                replaced_output_digest: Some(_),
                ..
            }
        )));
        assert_eq!(scheduler.history().len(), 2);
    }
}
