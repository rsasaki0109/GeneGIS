use chrono::{DateTime, Utc};
use genegis_crs::{CoordinateUnit, Crs, SourceSnapshot};
use genegis_style::EvidenceMapStyle;
use genegis_workflow::GeoWorkflow;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    EditableGeometryKind, EditableLayer, FeatureEdit, FeatureSchema, Layer, LayerId,
    MutationWorkflowBinding, Project,
};

/// Who initiated a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandOrigin {
    Ui,
    Ai,
    Cli,
    Plugin,
    System,
}

/// Stable SHA-256 identity of the workflow graph executed by a command.
///
/// The transparent representation keeps the JSON contract compatible with
/// clients that previously represented digests as strings while preventing
/// accidental mixing with arbitrary identifiers in typed Rust APIs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkflowDigest(String);

impl WorkflowDigest {
    /// Construct a digest identity from its canonical string spelling.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into().trim().to_string())
    }

    /// Return the digest string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Return whether this digest is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<String> for WorkflowDigest {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for WorkflowDigest {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl AsRef<str> for WorkflowDigest {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for WorkflowDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Immutable identity of one workflow input at command execution time.
///
/// This is deliberately separate from a workflow's input contract: the
/// contract states what an operation requires, while this snapshot states
/// which bytes/catalog revision were actually authorized for this run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputSnapshot {
    /// Name of the workflow input contract.
    pub name: String,
    /// Source identity, checksum state, license, and provider revision.
    pub source: SourceSnapshot,
    /// CRS observed for this input, when spatial.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crs: Option<Crs>,
    /// Coordinate axis unit derived from `crs`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinate_unit: Option<CoordinateUnit>,
    /// Unit of a non-coordinate value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_unit: Option<String>,
}

impl InputSnapshot {
    /// Construct a source-only input snapshot.
    pub fn new(name: impl Into<String>, source: SourceSnapshot) -> Self {
        Self {
            name: name.into(),
            source,
            crs: None,
            coordinate_unit: None,
            value_unit: None,
        }
    }

    /// Attach a CRS and derive its coordinate unit.
    pub fn with_crs(mut self, crs: Crs) -> Self {
        self.coordinate_unit = Some(crs.coordinate_unit());
        self.crs = Some(crs);
        self
    }

    /// Attach a non-coordinate value unit.
    pub fn with_value_unit(mut self, unit: impl Into<String>) -> Self {
        self.value_unit = Some(unit.into());
        self
    }
}

/// Envelope for every mutating operation in GeneGIS.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandEnvelope {
    pub id: Uuid,
    pub origin: CommandOrigin,
    pub timestamp: DateTime<Utc>,
    /// Digest of the graph that authorized this command. Missing values are
    /// accepted only for old envelopes; `RunWorkflow` execution rejects them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_digest: Option<WorkflowDigest>,
    /// Source snapshots attached to the command-level receipt.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_snapshots: Vec<SourceSnapshot>,
    /// Named input snapshots used to verify the graph's source contracts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_snapshots: Vec<InputSnapshot>,
    pub command: Command,
}

/// Context passed to a workflow executor after the command boundary has
/// validated the graph, digest, and input snapshots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowExecutionContext {
    /// Digest of the validated graph.
    pub workflow_digest: WorkflowDigest,
    /// Command identity for provenance and deterministic local observations.
    pub command_id: Uuid,
    /// Command event time.
    pub command_timestamp: DateTime<Utc>,
    /// Stable source identities authorized for the run.
    #[serde(default)]
    pub source_snapshots: Vec<SourceSnapshot>,
    /// Named input identities authorized for the run.
    #[serde(default)]
    pub input_snapshots: Vec<InputSnapshot>,
}

/// A retrieval/observation event emitted by a workflow executor.
///
/// Event time is intentionally separate from the stable source snapshot. It
/// is retained in receipts and audit records, but must not enter a stable
/// workflow/result digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowExecutionEvent {
    /// Event kind, for example `source_read` or `adapter_observation`.
    pub kind: String,
    /// Source URI observed by the adapter, when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_uri: Option<String>,
    /// Time at which the adapter observed/read the source.
    pub observed_at: DateTime<Utc>,
    /// Adapter-specific evidence that is not part of stable source identity.
    #[serde(default)]
    pub details: serde_json::Value,
}

/// Successful output of a workflow executor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowExecution {
    /// Canonical digest of the actual output and verification evidence.
    pub result_digest: String,
    /// Typed-engine output serialized at the core boundary.
    pub output: serde_json::Value,
    /// Verification/lineage evidence used to build the receipt.
    #[serde(default)]
    pub evidence: serde_json::Value,
    /// Retrieval/observation events emitted during execution.
    #[serde(default)]
    pub events: Vec<WorkflowExecutionEvent>,
}

/// Persisted workflow output associated with one command envelope. The
/// record lets a loaded log replay the same verified output without inventing
/// a new retrieval event; callers may also provide a live executor to
/// recompute the result during replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowExecutionRecord {
    pub command_id: Uuid,
    pub workflow_digest: WorkflowDigest,
    pub result_digest: String,
    pub output: serde_json::Value,
    #[serde(default)]
    pub evidence: serde_json::Value,
    #[serde(default)]
    pub events: Vec<WorkflowExecutionEvent>,
}

/// Error returned by a workflow executor before command state is committed.
#[derive(Debug, Error)]
pub enum WorkflowExecutionError {
    /// The executor rejected or could not complete the operation.
    #[error("workflow executor failed: {0}")]
    Failed(String),
}

/// Type-safe execution boundary used by [`CommandBus`] for RunWorkflow.
pub trait WorkflowExecutor {
    /// Execute a previously validated graph and return actual output/evidence.
    fn execute(
        &self,
        workflow: &GeoWorkflow,
        context: &WorkflowExecutionContext,
    ) -> Result<WorkflowExecution, WorkflowExecutionError>;
}

/// Command variants — all UX and AI paths converge here.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    RegisterStacEndpoint {
        endpoint_id: String,
        title: String,
        url: String,
        auth_kind: String,
        auth_env: Option<String>,
        auth_header: Option<String>,
    },
    RemoveStacEndpoint {
        endpoint_id: String,
    },
    SearchFederatedStac {
        endpoint_ids: Vec<String>,
        bbox: Option<[f64; 4]>,
        datetime: Option<String>,
        collections: Vec<String>,
        limit: Option<usize>,
    },
    BindStacAsset {
        stac_item_key: String,
        asset_key: String,
        source_endpoints: Vec<String>,
        href: String,
        media_type: String,
        crs: String,
        units: String,
        license: String,
    },
    ReadRemoteGeoParquet {
        uri: String,
        row_groups: Option<Vec<usize>>,
    },
    AddLayer {
        name: String,
        source_id: Uuid,
    },
    RemoveLayer {
        layer_id: Uuid,
    },
    SetLayerVisibility {
        layer_id: Uuid,
        visible: bool,
    },
    SetViewCamera {
        view_id: Uuid,
        center: [f64; 2],
        zoom: f64,
    },
    /// Attach an empty, revisioned feature store to an existing vector layer.
    InitializeEditableLayer {
        layer_id: Uuid,
        crs: Crs,
        geometry_kind: EditableGeometryKind,
        schema: FeatureSchema,
        workflow: MutationWorkflowBinding,
    },
    /// Apply one typed vector edit with optimistic concurrency.
    EditFeatures {
        layer_id: Uuid,
        expected_layer_revision: u64,
        edit: FeatureEdit,
        workflow: MutationWorkflowBinding,
    },
    /// Install or replace an evidence-carrying style for one layer.
    SetEvidenceMapStyle {
        style: EvidenceMapStyle,
        workflow: MutationWorkflowBinding,
    },
    RunWorkflow {
        workflow_id: Uuid,
    },
    Undo,
    Redo,
}

impl CommandEnvelope {
    /// Create a new command envelope with a fresh execution identity.
    pub fn new(origin: CommandOrigin, command: Command) -> Self {
        Self {
            id: Uuid::new_v4(),
            origin,
            timestamp: Utc::now(),
            workflow_digest: None,
            source_snapshots: Vec::new(),
            input_snapshots: Vec::new(),
            command,
        }
    }

    /// Attach the graph digest that authorized this command.
    pub fn with_workflow_digest(mut self, digest: impl Into<WorkflowDigest>) -> Self {
        self.workflow_digest = Some(digest.into());
        self
    }

    /// Attach a source snapshot to the command receipt.
    pub fn with_source_snapshot(mut self, source: SourceSnapshot) -> Self {
        self.source_snapshots.push(source);
        self
    }

    /// Attach a named input snapshot to the command receipt.
    pub fn with_input_snapshot(mut self, snapshot: InputSnapshot) -> Self {
        self.input_snapshots.push(snapshot);
        self
    }
}

/// Result of a successfully dispatched command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandExecution {
    /// Envelope identity that was applied.
    pub command_id: Uuid,
    /// Semantic project state after the operation.
    pub state_digest: String,
    /// Workflow identity, when the operation executed a workflow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_digest: Option<WorkflowDigest>,
    /// Actual workflow output digest, when a workflow was executed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_digest: Option<String>,
    /// Actual executor output, retained for typed adapters at the boundary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<serde_json::Value>,
    /// Verification/lineage evidence returned by the executor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<serde_json::Value>,
    /// Retrieval/observation events emitted by the executor.
    #[serde(default)]
    pub events: Vec<WorkflowExecutionEvent>,
    /// Active command-history cursor after the operation.
    pub cursor: usize,
}

/// Errors raised by the stateful command dispatcher and command log.
#[derive(Debug, Error)]
pub enum CommandError {
    /// A command needs an existing entity but could not find it.
    #[error("command target not found: {target}")]
    TargetNotFound { target: String },
    /// The command is not a Project/Workspace mutation handled by this core
    /// dispatcher. Catalog adapters own their specialized state.
    #[error("command is not supported by the project dispatcher: {command}")]
    UnsupportedCommand { command: String },
    /// A workflow ID was not registered with the dispatcher.
    #[error("workflow not registered: {workflow_id}")]
    WorkflowNotRegistered { workflow_id: Uuid },
    /// Workflow graph validation failed before any project state was changed.
    #[error("workflow validation failed: {reason}")]
    WorkflowInvalid { reason: String },
    /// A RunWorkflow command was dispatched without an executor.
    #[error("RunWorkflow requires a WorkflowExecutor")]
    ExecutorRequired,
    /// The executor failed; no project state or command history was committed.
    #[error("workflow execution failed: {reason}")]
    WorkflowExecutionFailed { reason: String },
    /// The executor returned an empty result identity.
    #[error("workflow executor returned an empty result digest")]
    EmptyResultDigest,
    /// An envelope omitted the digest required for workflow execution.
    #[error("RunWorkflow requires workflow_digest")]
    MissingWorkflowDigest,
    /// The envelope digest differs from the registered graph.
    #[error("workflow digest mismatch: expected {expected}, registered {actual}")]
    WorkflowDigestMismatch { expected: String, actual: String },
    /// A mutation did not bind to the matching reviewed workflow node.
    #[error("mutation workflow binding failed: {reason}")]
    MutationWorkflowBinding { reason: String },
    /// A vector edit failed validation without changing project state.
    #[error("feature edit rejected: {reason}")]
    FeatureEditRejected { reason: String },
    /// A map style failed validation without changing project state.
    #[error("map style rejected: {reason}")]
    MapStyleRejected { reason: String },
    /// A workflow source contract was not represented by the command snapshot.
    #[error("missing input snapshot for workflow contract {input}")]
    MissingInputSnapshot { input: String },
    /// A command snapshot differs from the graph's authorized source identity.
    #[error("input snapshot mismatch for workflow contract {input}")]
    InputSnapshotMismatch { input: String },
    /// No prior command can be undone.
    #[error("no command to undo")]
    NothingToUndo,
    /// No undone command can be redone.
    #[error("no command to redo")]
    NothingToRedo,
    /// A bus loaded from a legacy/manual history has no state snapshots.
    #[error("command history has no state snapshots; replay it before undo/redo")]
    MissingStateSnapshot,
    /// The command log schema is not understood.
    #[error("unsupported command log schema version {version}")]
    UnsupportedLogSchema { version: u32 },
    /// A persisted command log did not contain an integrity digest.
    #[error("command log is missing log_digest")]
    MissingLogDigest,
    /// A persisted command log was modified after it was written.
    #[error("command log digest mismatch: expected {expected}, observed {observed}")]
    LogDigestMismatch { expected: String, observed: String },
    /// Replay produced a state different from the persisted receipt.
    #[error("replay state digest mismatch: expected {expected}, observed {observed}")]
    ReplayDigestMismatch { expected: String, observed: String },
    /// Filesystem or serialization failure while persisting the command log.
    #[error("command log I/O error: {0}")]
    Io(String),
    /// JSON encoding/decoding failure while persisting the command log.
    #[error("command log JSON error: {0}")]
    Json(String),
}

/// Versioned, tamper-evident command log persisted by [`CommandBus`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandLog {
    /// Persistence schema version.
    pub schema_version: u32,
    /// State from which the event stream is replayed.
    pub initial_state: Project,
    /// Active branch of mutating commands.
    pub commands: Vec<CommandEnvelope>,
    /// Full event stream, including undo/redo control events and abandoned
    /// branches, used for faithful replay and audit.
    #[serde(default)]
    pub audit_log: Vec<CommandEnvelope>,
    /// Active history cursor after the last persisted operation.
    pub cursor: usize,
    /// Workflows required to validate RunWorkflow events during replay.
    #[serde(default)]
    pub workflows: Vec<GeoWorkflow>,
    /// Successful workflow outputs captured at command time. This is
    /// intentionally separate from source identity; event timestamps remain
    /// in `events` and are never used for stable result digests.
    #[serde(default)]
    pub workflow_executions: Vec<WorkflowExecutionRecord>,
    /// Canonical semantic state digest captured at persistence time.
    pub state_digest: String,
    /// SHA-256 digest over this document with `log_digest` omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_digest: Option<String>,
}

impl CommandLog {
    /// Current command log schema version.
    pub const CURRENT_SCHEMA_VERSION: u32 = 1;
}

/// Stateful Command + Workflow Graph dispatcher.
///
/// The history cursor is only an index into the event stream. Actual state is
/// changed through snapshots captured around each successful command, and a
/// persisted log can reconstruct those snapshots from the initial Project.
#[derive(Debug, Clone, Default)]
pub struct CommandBus {
    history: Vec<CommandEnvelope>,
    cursor: usize,
    before_states: Vec<Project>,
    after_states: Vec<Project>,
    initial_state: Option<Project>,
    current_state: Option<Project>,
    audit_log: Vec<CommandEnvelope>,
    workflows: BTreeMap<Uuid, GeoWorkflow>,
    workflow_executions: BTreeMap<Uuid, WorkflowExecutionRecord>,
    expected_replay_digest: Option<String>,
}

struct RecordedWorkflowExecutor<'a> {
    records: &'a BTreeMap<Uuid, WorkflowExecutionRecord>,
}

impl WorkflowExecutor for RecordedWorkflowExecutor<'_> {
    fn execute(
        &self,
        _workflow: &GeoWorkflow,
        context: &WorkflowExecutionContext,
    ) -> Result<WorkflowExecution, WorkflowExecutionError> {
        let record = self.records.get(&context.command_id).ok_or_else(|| {
            WorkflowExecutionError::Failed(format!(
                "persisted workflow execution missing for command {}",
                context.command_id
            ))
        })?;
        if record.workflow_digest != context.workflow_digest {
            return Err(WorkflowExecutionError::Failed(
                "persisted workflow execution digest differs from command".into(),
            ));
        }
        Ok(WorkflowExecution {
            result_digest: record.result_digest.clone(),
            output: record.output.clone(),
            evidence: record.evidence.clone(),
            events: record.events.clone(),
        })
    }
}

impl CommandBus {
    /// Create an empty dispatcher rooted at a known initial project state.
    pub fn new(initial_state: Project) -> Self {
        Self {
            initial_state: Some(initial_state.clone()),
            current_state: Some(initial_state),
            ..Self::default()
        }
    }

    /// Register a workflow graph by its execution ID.
    pub fn register_workflow(
        &mut self,
        workflow: GeoWorkflow,
    ) -> Result<WorkflowDigest, CommandError> {
        workflow
            .validate()
            .map_err(|error| CommandError::WorkflowInvalid {
                reason: error.to_string(),
            })?;
        let digest = WorkflowDigest::new(workflow.stable_digest().map_err(|error| {
            CommandError::WorkflowInvalid {
                reason: error.to_string(),
            }
        })?);
        self.workflows.insert(workflow.id, workflow);
        Ok(digest)
    }

    /// Return the registered workflow, if present.
    pub fn workflow(&self, id: Uuid) -> Option<&GeoWorkflow> {
        self.workflows.get(&id)
    }

    /// Record a legacy envelope without applying it.
    ///
    /// This method remains for catalog adapters that maintain their own state
    /// (for example the STAC endpoint registry). Project mutations should use
    /// [`CommandBus::apply`].
    pub fn push(&mut self, envelope: CommandEnvelope) {
        self.history.truncate(self.cursor);
        self.before_states.truncate(self.cursor);
        self.after_states.truncate(self.cursor);
        self.history.push(envelope.clone());
        self.cursor = self.history.len();
        self.audit_log.push(envelope);
    }

    /// Apply a command to the supplied Project and return its state receipt.
    pub fn apply(
        &mut self,
        project: &mut Project,
        envelope: CommandEnvelope,
    ) -> Result<CommandExecution, CommandError> {
        self.dispatch(project, envelope, None)
    }

    /// Apply a command through a typed workflow executor.
    ///
    /// `RunWorkflow` is validated first. The executor is then called before
    /// the Project, history, audit log, or receipt state is changed. Any
    /// executor error therefore leaves all command state untouched.
    pub fn apply_with_executor(
        &mut self,
        project: &mut Project,
        envelope: CommandEnvelope,
        executor: &dyn WorkflowExecutor,
    ) -> Result<CommandExecution, CommandError> {
        self.dispatch(project, envelope, Some(executor))
    }

    fn dispatch(
        &mut self,
        project: &mut Project,
        envelope: CommandEnvelope,
        executor: Option<&dyn WorkflowExecutor>,
    ) -> Result<CommandExecution, CommandError> {
        match envelope.command {
            Command::Undo => {
                let command_id = envelope.id;
                self.undo(project)?;
                self.audit_log.push(envelope);
                return Ok(self.execution(command_id, None, None, project));
            }
            Command::Redo => {
                let command_id = envelope.id;
                self.redo(project)?;
                self.audit_log.push(envelope);
                return Ok(self.execution(command_id, None, None, project));
            }
            _ => {}
        }

        self.validate_mutation_workflow(&envelope)?;

        // Validate a RunWorkflow before cloning or mutating project state. The
        // registered graph and its digest are the fail-closed authorization
        // boundary for all workflow-backed commands.
        let workflow_digest = self.validate_workflow_command(&envelope)?;
        let workflow_execution = if workflow_digest.is_some() {
            let executor = executor.ok_or(CommandError::ExecutorRequired)?;
            let Command::RunWorkflow { workflow_id } = &envelope.command else {
                unreachable!("workflow digest only exists for RunWorkflow")
            };
            let workflow = self
                .workflows
                .get(workflow_id)
                .expect("RunWorkflow was validated");
            let context = WorkflowExecutionContext {
                workflow_digest: workflow_digest
                    .clone()
                    .expect("workflow digest was validated"),
                command_id: envelope.id,
                command_timestamp: envelope.timestamp,
                source_snapshots: envelope.source_snapshots.clone(),
                input_snapshots: envelope.input_snapshots.clone(),
            };
            let execution = executor.execute(workflow, &context).map_err(|error| {
                CommandError::WorkflowExecutionFailed {
                    reason: error.to_string(),
                }
            })?;
            if execution.result_digest.trim().is_empty() {
                return Err(CommandError::EmptyResultDigest);
            }
            Some(execution)
        } else {
            None
        };
        let before = project.clone();
        self.apply_project_mutation(
            project,
            &envelope,
            workflow_digest.as_ref(),
            workflow_execution.as_ref(),
        )?;

        if self.initial_state.is_none() {
            self.initial_state = Some(before.clone());
        }
        self.history.truncate(self.cursor);
        self.before_states.truncate(self.cursor);
        self.after_states.truncate(self.cursor);
        self.history.push(envelope.clone());
        self.before_states.push(before);
        self.after_states.push(project.clone());
        self.cursor = self.history.len();
        self.current_state = Some(project.clone());
        self.audit_log.push(envelope.clone());
        if let (Some(workflow_digest), Some(execution)) =
            (workflow_digest.as_ref(), workflow_execution.as_ref())
        {
            self.workflow_executions.insert(
                envelope.id,
                WorkflowExecutionRecord {
                    command_id: envelope.id,
                    workflow_digest: workflow_digest.clone(),
                    result_digest: execution.result_digest.clone(),
                    output: execution.output.clone(),
                    evidence: execution.evidence.clone(),
                    events: execution.events.clone(),
                },
            );
        }
        Ok(self.execution(
            envelope.id,
            workflow_digest,
            workflow_execution.as_ref(),
            project,
        ))
    }

    /// Register a graph and apply its command through the same dispatcher.
    pub fn apply_with_workflow(
        &mut self,
        project: &mut Project,
        envelope: CommandEnvelope,
        workflow: GeoWorkflow,
    ) -> Result<CommandExecution, CommandError> {
        self.register_workflow(workflow)?;
        self.apply(project, envelope)
    }

    /// Register a graph and execute it through the supplied typed executor.
    pub fn apply_with_workflow_executor(
        &mut self,
        project: &mut Project,
        envelope: CommandEnvelope,
        workflow: GeoWorkflow,
        executor: &dyn WorkflowExecutor,
    ) -> Result<CommandExecution, CommandError> {
        self.register_workflow(workflow)?;
        self.apply_with_executor(project, envelope, executor)
    }

    /// Restore the actual Project state before the current command.
    pub fn undo(&mut self, project: &mut Project) -> Result<(), CommandError> {
        if self.cursor == 0 {
            return Err(CommandError::NothingToUndo);
        }
        let state = self
            .before_states
            .get(self.cursor - 1)
            .cloned()
            .ok_or(CommandError::MissingStateSnapshot)?;
        *project = state.clone();
        self.cursor -= 1;
        self.current_state = Some(state);
        Ok(())
    }

    /// Restore the actual Project state after the next undone command.
    pub fn redo(&mut self, project: &mut Project) -> Result<(), CommandError> {
        if self.cursor >= self.history.len() {
            return Err(CommandError::NothingToRedo);
        }
        let state = self
            .after_states
            .get(self.cursor)
            .cloned()
            .ok_or(CommandError::MissingStateSnapshot)?;
        *project = state.clone();
        self.cursor += 1;
        self.current_state = Some(state);
        Ok(())
    }

    /// Return the active mutating command history.
    pub fn history(&self) -> &[CommandEnvelope] {
        &self.history
    }

    /// Return a persisted workflow output record for a command, when the
    /// command completed through a workflow executor.
    pub fn workflow_execution(&self, command_id: Uuid) -> Option<&WorkflowExecutionRecord> {
        self.workflow_executions.get(&command_id)
    }

    /// Return the complete audit event stream, including undo/redo events.
    pub fn audit_log(&self) -> &[CommandEnvelope] {
        &self.audit_log
    }

    /// Return the active history cursor.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Return whether a real state undo is available.
    pub fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    /// Return whether a real state redo is available.
    pub fn can_redo(&self) -> bool {
        self.cursor < self.history.len()
    }

    /// Persist the initial state and tamper-evident command/event log.
    pub fn persist(&self, path: impl AsRef<Path>) -> Result<(), CommandError> {
        let mut copy = self.clone();
        let state = copy.current_project_for_persistence()?;
        let mut log = CommandLog {
            schema_version: CommandLog::CURRENT_SCHEMA_VERSION,
            initial_state: self
                .initial_state
                .clone()
                .ok_or(CommandError::MissingStateSnapshot)?,
            commands: self.history.clone(),
            audit_log: self.audit_log.clone(),
            cursor: self.cursor,
            workflows: self.workflows.values().cloned().collect(),
            workflow_executions: self.workflow_executions.values().cloned().collect(),
            state_digest: state.state_digest(),
            log_digest: None,
        };
        let mut value =
            serde_json::to_value(&log).map_err(|error| CommandError::Json(error.to_string()))?;
        let digest = digest_json(&value);
        log.log_digest = Some(digest);
        value =
            serde_json::to_value(&log).map_err(|error| CommandError::Json(error.to_string()))?;
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| CommandError::Io(error.to_string()))?;
            }
        }
        let json = serde_json::to_string_pretty(&value)
            .map_err(|error| CommandError::Json(error.to_string()))?;
        std::fs::write(path, json).map_err(|error| CommandError::Io(error.to_string()))
    }

    /// Alias for [`CommandBus::persist`].
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), CommandError> {
        self.persist(path)
    }

    /// Load and integrity-check a persisted command log.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, CommandError> {
        let json =
            std::fs::read_to_string(path).map_err(|error| CommandError::Io(error.to_string()))?;
        let log: CommandLog =
            serde_json::from_str(&json).map_err(|error| CommandError::Json(error.to_string()))?;
        if log.schema_version != CommandLog::CURRENT_SCHEMA_VERSION {
            return Err(CommandError::UnsupportedLogSchema {
                version: log.schema_version,
            });
        }
        let expected = log
            .log_digest
            .clone()
            .ok_or(CommandError::MissingLogDigest)?;
        let mut without_digest = log.clone();
        without_digest.log_digest = None;
        let value = serde_json::to_value(&without_digest)
            .map_err(|error| CommandError::Json(error.to_string()))?;
        let observed = digest_json(&value);
        if expected != observed {
            return Err(CommandError::LogDigestMismatch { expected, observed });
        }
        if log.cursor > log.commands.len() {
            return Err(CommandError::Json(
                "command cursor exceeds command history".into(),
            ));
        }
        let workflows = log
            .workflows
            .iter()
            .cloned()
            .map(|workflow| (workflow.id, workflow))
            .collect();
        let workflow_executions = log
            .workflow_executions
            .iter()
            .cloned()
            .map(|execution| (execution.command_id, execution))
            .collect();
        Ok(Self {
            history: log.commands,
            cursor: log.cursor,
            before_states: Vec::new(),
            after_states: Vec::new(),
            initial_state: Some(log.initial_state),
            current_state: None,
            audit_log: log.audit_log,
            workflows,
            workflow_executions,
            expected_replay_digest: Some(log.state_digest),
        })
    }

    /// Replay the persisted audit stream from the initial state.
    ///
    /// The returned Project is the state at the replayed cursor, and the bus
    /// receives fresh before/after snapshots so undo/redo remains real after
    /// loading. A stored result digest is checked before returning.
    pub fn replay(&mut self) -> Result<Project, CommandError> {
        self.replay_inner(None)
    }

    /// Replay the persisted stream while recomputing workflow outputs through
    /// a live executor. This is useful for deterministic verification of a
    /// saved log; [`CommandBus::replay`] uses the captured execution records so
    /// a local read does not create a new observation event.
    pub fn replay_with_executor(
        &mut self,
        executor: &dyn WorkflowExecutor,
    ) -> Result<Project, CommandError> {
        self.replay_inner(Some(executor))
    }

    fn replay_inner(
        &mut self,
        executor: Option<&dyn WorkflowExecutor>,
    ) -> Result<Project, CommandError> {
        let initial = self
            .initial_state
            .clone()
            .ok_or(CommandError::MissingStateSnapshot)?;
        let events = if self.audit_log.is_empty() {
            self.history.clone()
        } else {
            self.audit_log.clone()
        };
        let workflows: Vec<_> = self.workflows.values().cloned().collect();
        let mut replayed = CommandBus::new(initial.clone());
        for workflow in workflows {
            replayed.register_workflow(workflow)?;
        }
        let recorded_executor = RecordedWorkflowExecutor {
            records: &self.workflow_executions,
        };
        let replay_executor = match executor {
            Some(executor) => Some(executor),
            None if !self.workflow_executions.is_empty() => {
                Some(&recorded_executor as &dyn WorkflowExecutor)
            }
            None => None,
        };
        let mut state = initial;
        for event in events {
            if let Some(executor) = replay_executor {
                replayed.apply_with_executor(&mut state, event, executor)?;
            } else {
                replayed.apply(&mut state, event)?;
            }
        }
        let expected_cursor = self.cursor;
        let result = if replayed.cursor == expected_cursor {
            state
        } else {
            // Persisted command histories remain backward compatible with
            // logs that only had the active commands and cursor.
            let mut active = CommandBus::new(
                replayed
                    .initial_state
                    .clone()
                    .ok_or(CommandError::MissingStateSnapshot)?,
            );
            for workflow in replayed.workflows.values().cloned() {
                active.register_workflow(workflow)?;
            }
            let mut active_state = active
                .initial_state
                .clone()
                .ok_or(CommandError::MissingStateSnapshot)?;
            for event in self.history.iter().take(expected_cursor).cloned() {
                if let Some(executor) = replay_executor {
                    active.apply_with_executor(&mut active_state, event, executor)?;
                } else {
                    active.apply(&mut active_state, event)?;
                }
            }
            active_state
        };
        if let Some(expected) = &self.expected_replay_digest {
            let observed = result.state_digest();
            if expected != &observed {
                return Err(CommandError::ReplayDigestMismatch {
                    expected: expected.clone(),
                    observed,
                });
            }
        }

        // Rebuild snapshot arrays from the complete event stream while
        // preserving the state returned at the persisted cursor.
        self.history = replayed.history;
        self.before_states = replayed.before_states;
        self.after_states = replayed.after_states;
        self.cursor = replayed.cursor;
        self.initial_state = replayed.initial_state;
        self.workflow_executions = replayed.workflow_executions;
        self.current_state = Some(result.clone());
        Ok(result)
    }

    /// Alias for [`CommandBus::replay`].
    pub fn replay_state(&mut self) -> Result<Project, CommandError> {
        self.replay()
    }

    /// Alias for [`CommandBus::replay_with_executor`].
    pub fn replay_state_with_executor(
        &mut self,
        executor: &dyn WorkflowExecutor,
    ) -> Result<Project, CommandError> {
        self.replay_with_executor(executor)
    }

    /// Return the canonical state digest after replaying this bus.
    pub fn replay_digest(&mut self) -> Result<String, CommandError> {
        Ok(self.replay()?.state_digest())
    }

    /// Return the canonical state digest after replaying with a live executor.
    pub fn replay_digest_with_executor(
        &mut self,
        executor: &dyn WorkflowExecutor,
    ) -> Result<String, CommandError> {
        Ok(self.replay_with_executor(executor)?.state_digest())
    }

    fn current_project_for_persistence(&mut self) -> Result<Project, CommandError> {
        if let Some(state) = &self.current_state {
            return Ok(state.clone());
        }
        self.replay()
    }

    fn execution(
        &self,
        command_id: Uuid,
        workflow_digest: Option<WorkflowDigest>,
        workflow_execution: Option<&WorkflowExecution>,
        project: &Project,
    ) -> CommandExecution {
        CommandExecution {
            command_id,
            state_digest: project.state_digest(),
            workflow_digest,
            result_digest: workflow_execution.map(|execution| execution.result_digest.clone()),
            output: workflow_execution.map(|execution| execution.output.clone()),
            evidence: workflow_execution.map(|execution| execution.evidence.clone()),
            events: workflow_execution
                .map(|execution| execution.events.clone())
                .unwrap_or_default(),
            cursor: self.cursor,
        }
    }

    fn validate_workflow_command(
        &self,
        envelope: &CommandEnvelope,
    ) -> Result<Option<WorkflowDigest>, CommandError> {
        let Command::RunWorkflow { workflow_id } = &envelope.command else {
            return Ok(None);
        };
        let workflow =
            self.workflows
                .get(workflow_id)
                .ok_or(CommandError::WorkflowNotRegistered {
                    workflow_id: *workflow_id,
                })?;
        workflow
            .validate()
            .map_err(|error| CommandError::WorkflowInvalid {
                reason: error.to_string(),
            })?;
        let actual = WorkflowDigest::new(workflow.stable_digest().map_err(|error| {
            CommandError::WorkflowInvalid {
                reason: error.to_string(),
            }
        })?);
        let expected = envelope
            .workflow_digest
            .clone()
            .ok_or(CommandError::MissingWorkflowDigest)?;
        if expected != actual {
            return Err(CommandError::WorkflowDigestMismatch {
                expected: expected.to_string(),
                actual: actual.to_string(),
            });
        }
        validate_input_snapshots(workflow, envelope)?;
        Ok(Some(actual))
    }

    fn validate_mutation_workflow(&self, envelope: &CommandEnvelope) -> Result<(), CommandError> {
        let (binding, expected_operation) = match &envelope.command {
            Command::InitializeEditableLayer { workflow, .. } => {
                (workflow, "InitializeEditableLayer")
            }
            Command::EditFeatures { edit, workflow, .. } => (workflow, edit.workflow_operation()),
            Command::SetEvidenceMapStyle { workflow, .. } => (workflow, "SetEvidenceMapStyle"),
            _ => return Ok(()),
        };
        let workflow = self.workflows.get(&binding.workflow_id).ok_or_else(|| {
            CommandError::MutationWorkflowBinding {
                reason: format!("workflow {} is not registered", binding.workflow_id),
            }
        })?;
        workflow
            .validate()
            .map_err(|error| CommandError::MutationWorkflowBinding {
                reason: error.to_string(),
            })?;
        let actual =
            workflow
                .stable_digest()
                .map_err(|error| CommandError::MutationWorkflowBinding {
                    reason: error.to_string(),
                })?;
        let expected = envelope.workflow_digest.as_ref().ok_or_else(|| {
            CommandError::MutationWorkflowBinding {
                reason: "command envelope has no workflow digest".into(),
            }
        })?;
        if expected.as_str() != actual {
            return Err(CommandError::WorkflowDigestMismatch {
                expected: expected.to_string(),
                actual,
            });
        }
        let node = workflow
            .steps
            .iter()
            .find(|step| step.node_id().as_str() == binding.node_id)
            .ok_or_else(|| CommandError::MutationWorkflowBinding {
                reason: format!("node {} does not exist", binding.node_id),
            })?;
        if node.operation != expected_operation {
            return Err(CommandError::MutationWorkflowBinding {
                reason: format!(
                    "node {} operation is {:?}, expected {:?}",
                    binding.node_id, node.operation, expected_operation
                ),
            });
        }
        Ok(())
    }

    fn apply_project_mutation(
        &self,
        project: &mut Project,
        envelope: &CommandEnvelope,
        workflow_digest: Option<&WorkflowDigest>,
        workflow_execution: Option<&WorkflowExecution>,
    ) -> Result<(), CommandError> {
        match &envelope.command {
            Command::AddLayer { name, source_id } => {
                // The envelope ID is stable in a persisted log, so generated
                // layer identity remains identical during replay.
                let layer = Layer {
                    id: LayerId(envelope.id),
                    name: name.clone(),
                    kind: crate::LayerKind::Vector,
                    source_id: *source_id,
                    crs: None,
                    extent: None,
                    time_extent: None,
                    statistics: crate::LayerStatistics::default(),
                    style_id: None,
                    visible: true,
                    opacity: 1.0,
                };
                let workspace = project.workspace_mut();
                if workspace
                    .layers
                    .iter()
                    .any(|candidate| candidate.id == layer.id)
                {
                    return Err(CommandError::UnsupportedCommand {
                        command: "AddLayer would duplicate envelope identity".into(),
                    });
                }
                workspace.add_layer(layer);
                record_command_provenance(
                    project,
                    envelope,
                    "add_layer",
                    envelope.id.to_string(),
                    serde_json::json!({ "name": name, "source_id": source_id }),
                );
            }
            Command::RemoveLayer { layer_id } => {
                let workspace = project.workspace_mut();
                let index = workspace
                    .layers
                    .iter()
                    .position(|layer| layer.id.0 == *layer_id)
                    .ok_or_else(|| CommandError::TargetNotFound {
                        target: format!("layer {layer_id}"),
                    })?;
                workspace.layers.remove(index);
                workspace.updated_at = envelope.timestamp;
                record_command_provenance(
                    project,
                    envelope,
                    "remove_layer",
                    layer_id.to_string(),
                    serde_json::json!({}),
                );
            }
            Command::SetLayerVisibility { layer_id, visible } => {
                let workspace = project.workspace_mut();
                let layer = workspace
                    .layers
                    .iter_mut()
                    .find(|layer| layer.id.0 == *layer_id)
                    .ok_or_else(|| CommandError::TargetNotFound {
                        target: format!("layer {layer_id}"),
                    })?;
                layer.visible = *visible;
                workspace.updated_at = envelope.timestamp;
                record_command_provenance(
                    project,
                    envelope,
                    "set_layer_visibility",
                    layer_id.to_string(),
                    serde_json::json!({ "visible": visible }),
                );
            }
            Command::SetViewCamera {
                view_id,
                center,
                zoom,
            } => {
                let workspace = project.workspace_mut();
                let view = workspace
                    .views
                    .iter_mut()
                    .find(|view| view.id.0 == *view_id)
                    .ok_or_else(|| CommandError::TargetNotFound {
                        target: format!("view {view_id}"),
                    })?;
                view.center = Some(*center);
                view.zoom = Some(*zoom);
                workspace.updated_at = envelope.timestamp;
                record_command_provenance(
                    project,
                    envelope,
                    "set_view_camera",
                    view_id.to_string(),
                    serde_json::json!({ "center": center, "zoom": zoom }),
                );
            }
            Command::InitializeEditableLayer {
                layer_id,
                crs,
                geometry_kind,
                schema,
                workflow,
            } => {
                let workspace = project.workspace_mut();
                let layer = workspace
                    .layers
                    .iter()
                    .find(|layer| layer.id.0 == *layer_id)
                    .ok_or_else(|| CommandError::TargetNotFound {
                        target: format!("layer {layer_id}"),
                    })?;
                if layer.kind != crate::LayerKind::Vector {
                    return Err(CommandError::FeatureEditRejected {
                        reason: "only vector layers can become editable".into(),
                    });
                }
                if workspace
                    .editable_layers
                    .iter()
                    .any(|editable| editable.layer_id == *layer_id)
                {
                    return Err(CommandError::FeatureEditRejected {
                        reason: "editable layer already initialized".into(),
                    });
                }
                let editable =
                    EditableLayer::new(*layer_id, crs.clone(), *geometry_kind, schema.clone())
                        .map_err(|error| CommandError::FeatureEditRejected {
                            reason: error.to_string(),
                        })?;
                let coordinate_unit = editable.coordinate_unit;
                workspace.editable_layers.push(editable);
                workspace.updated_at = envelope.timestamp;
                record_command_provenance(
                    project,
                    envelope,
                    "initialize_editable_layer",
                    layer_id.to_string(),
                    serde_json::json!({
                        "crs": crs,
                        "coordinate_unit": coordinate_unit,
                        "geometry_kind": geometry_kind,
                        "schema": schema,
                        "workflow_id": workflow.workflow_id,
                        "workflow_node": workflow.node_id,
                        "workflow_digest": envelope.workflow_digest,
                    }),
                );
            }
            Command::EditFeatures {
                layer_id,
                expected_layer_revision,
                edit,
                workflow,
            } => {
                let receipt = project
                    .workspace_mut()
                    .editable_layers
                    .iter_mut()
                    .find(|editable| editable.layer_id == *layer_id)
                    .ok_or_else(|| CommandError::TargetNotFound {
                        target: format!("editable layer {layer_id}"),
                    })?
                    .apply(*expected_layer_revision, edit.clone())
                    .map_err(|error| CommandError::FeatureEditRejected {
                        reason: error.to_string(),
                    })?;
                project.workspace_mut().updated_at = envelope.timestamp;
                record_command_provenance(
                    project,
                    envelope,
                    edit.workflow_operation(),
                    layer_id.to_string(),
                    serde_json::json!({
                        "receipt": receipt,
                        "crs": project.workspace().editable_layers.iter().find(|editable| editable.layer_id == *layer_id).map(|editable| &editable.crs),
                        "coordinate_unit": project.workspace().editable_layers.iter().find(|editable| editable.layer_id == *layer_id).map(|editable| editable.coordinate_unit),
                        "workflow_id": workflow.workflow_id,
                        "workflow_node": workflow.node_id,
                        "workflow_digest": envelope.workflow_digest,
                    }),
                );
            }
            Command::SetEvidenceMapStyle { style, workflow } => {
                style
                    .validate()
                    .map_err(|error| CommandError::MapStyleRejected {
                        reason: error.to_string(),
                    })?;
                let workspace = project.workspace_mut();
                let layer = workspace
                    .layers
                    .iter_mut()
                    .find(|layer| layer.id.0 == style.layer_id)
                    .ok_or_else(|| CommandError::TargetNotFound {
                        target: format!("layer {}", style.layer_id),
                    })?;
                layer.style_id = Some(style.id);
                if let Some(existing) = workspace
                    .map_styles
                    .iter_mut()
                    .find(|candidate| candidate.id == style.id)
                {
                    *existing = style.clone();
                } else {
                    workspace.map_styles.push(style.clone());
                    workspace.map_styles.sort_by_key(|candidate| candidate.id);
                }
                workspace.updated_at = envelope.timestamp;
                record_command_provenance(
                    project,
                    envelope,
                    "set_evidence_map_style",
                    style.layer_id.to_string(),
                    serde_json::json!({
                        "style": style,
                        "workflow_id": workflow.workflow_id,
                        "workflow_node": workflow.node_id,
                        "workflow_digest": envelope.workflow_digest,
                    }),
                );
            }
            Command::RunWorkflow { workflow_id } => {
                let digest = workflow_digest.expect("RunWorkflow was validated");
                let execution = workflow_execution.expect("RunWorkflow was executed");
                record_command_provenance(
                    project,
                    envelope,
                    "run_workflow",
                    workflow_id.to_string(),
                    serde_json::json!({
                        "workflow_digest": digest,
                        "workflow_id": workflow_id,
                        "result_digest": execution.result_digest,
                        "evidence": execution.evidence,
                        "events": execution.events,
                        "source_snapshots": envelope.source_snapshots,
                        "input_snapshots": envelope.input_snapshots,
                    }),
                );
            }
            Command::RegisterStacEndpoint { .. }
            | Command::RemoveStacEndpoint { .. }
            | Command::SearchFederatedStac { .. }
            | Command::BindStacAsset { .. }
            | Command::ReadRemoteGeoParquet { .. }
            | Command::Undo
            | Command::Redo => {
                return Err(CommandError::UnsupportedCommand {
                    command: format!("{:?}", envelope.command),
                });
            }
        }
        Ok(())
    }
}

fn validate_input_snapshots(
    workflow: &GeoWorkflow,
    envelope: &CommandEnvelope,
) -> Result<(), CommandError> {
    // Every explicit source snapshot is part of the authorization envelope,
    // not merely informational receipt data. Reject an unknown/tampered
    // source even when another duplicate snapshot happens to match a
    // contract; otherwise a caller could smuggle an unverified source into
    // the executor context.
    for snapshot in &envelope.source_snapshots {
        let known = workflow
            .input_contracts
            .iter()
            .filter_map(|contract| contract.source_snapshot.as_ref())
            .any(|expected| same_source_identity(snapshot, expected));
        let belongs_to_contract = workflow.input_contracts.iter().any(|contract| {
            contract
                .source_snapshot
                .as_ref()
                .and_then(|expected| expected.dataset_id.as_ref())
                .zip(snapshot.dataset_id.as_ref())
                .is_some_and(|(expected, observed)| expected == observed)
        });
        // Catalog-selected assets may carry a concrete dataset identity in
        // addition to a provider/catalog input contract. Such an observation
        // is valid even when its URI is not the contract URI. If the same
        // contract dataset identity is present, however, its URI/checksum
        // must still match so a tampered source cannot be smuggled through a
        // duplicate input snapshot.
        let supplemental_catalog_source = snapshot.dataset_id.is_some() && !belongs_to_contract;
        if !known && !supplemental_catalog_source && !workflow.input_contracts.is_empty() {
            return Err(CommandError::InputSnapshotMismatch {
                input: snapshot.uri.clone(),
            });
        }
    }

    // When input snapshots are supplied, validate every named entry as well
    // as every required contract. Empty input lists remain readable for old
    // envelopes that carried only a matching source snapshot.
    for snapshot in &envelope.input_snapshots {
        let Some(contract) = workflow
            .input_contracts
            .iter()
            .find(|contract| contract.name == snapshot.name)
        else {
            return Err(CommandError::InputSnapshotMismatch {
                input: snapshot.name.clone(),
            });
        };
        let matches_contract = contract
            .source_snapshot
            .as_ref()
            .map(|expected| same_source_identity(&snapshot.source, expected))
            .unwrap_or(true);
        if !matches_contract {
            return Err(CommandError::InputSnapshotMismatch {
                input: snapshot.name.clone(),
            });
        }
    }

    for contract in &workflow.input_contracts {
        let Some(expected) = &contract.source_snapshot else {
            continue;
        };
        let input_match = envelope
            .input_snapshots
            .iter()
            .find(|snapshot| snapshot.name == contract.name)
            .map(|snapshot| same_source_identity(&snapshot.source, expected))
            .unwrap_or(false);
        let source_match = envelope
            .source_snapshots
            .iter()
            .any(|snapshot| same_source_identity(snapshot, expected));
        if !input_match && !source_match {
            return if envelope.input_snapshots.is_empty() && envelope.source_snapshots.is_empty() {
                Err(CommandError::MissingInputSnapshot {
                    input: contract.name.clone(),
                })
            } else {
                Err(CommandError::InputSnapshotMismatch {
                    input: contract.name.clone(),
                })
            };
        }
    }
    Ok(())
}

fn same_source_identity(left: &SourceSnapshot, right: &SourceSnapshot) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    // Retrieval time describes this fetch, not the immutable source identity.
    left.retrieved_at = None;
    right.retrieved_at = None;
    left == right
}

fn record_command_provenance(
    project: &mut Project,
    envelope: &CommandEnvelope,
    action: &str,
    target: String,
    details: serde_json::Value,
) {
    let workflow_id = match envelope.command {
        Command::RunWorkflow { workflow_id } => Some(workflow_id.to_string()),
        Command::InitializeEditableLayer { ref workflow, .. }
        | Command::EditFeatures { ref workflow, .. }
        | Command::SetEvidenceMapStyle { ref workflow, .. } => {
            Some(workflow.workflow_id.to_string())
        }
        _ => None,
    };
    project
        .workspace_mut()
        .provenance
        .entries
        .push(crate::ProvenanceEntry {
            // A command can be replayed, so provenance identity is derived from
            // the persisted envelope rather than from a fresh random UUID.
            id: envelope.id,
            timestamp: envelope.timestamp,
            actor: format!("{:?}", envelope.origin).to_lowercase(),
            action: action.to_string(),
            target,
            details,
            agent_run_id: None,
            workflow_id,
        });
}

fn digest_json(value: &serde_json::Value) -> String {
    let canonical = canonical_json(value);
    format!("sha256:{:x}", Sha256::digest(canonical.as_bytes()))
}

fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by_key(|(key, _)| *key);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LayerKind, Project, View, ViewKind};
    use genegis_crs::Crs;
    use genegis_workflow::nagoya_population_density_template;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    fn layer_command(name: &str) -> CommandEnvelope {
        CommandEnvelope::new(
            CommandOrigin::Cli,
            Command::AddLayer {
                name: name.into(),
                source_id: Uuid::nil(),
            },
        )
    }

    struct EchoExecutor {
        result_digest: String,
        fail: bool,
    }

    impl WorkflowExecutor for EchoExecutor {
        fn execute(
            &self,
            _workflow: &GeoWorkflow,
            _context: &WorkflowExecutionContext,
        ) -> Result<WorkflowExecution, WorkflowExecutionError> {
            if self.fail {
                return Err(WorkflowExecutionError::Failed("intentional failure".into()));
            }
            Ok(WorkflowExecution {
                result_digest: self.result_digest.clone(),
                output: serde_json::json!({"ok": true}),
                evidence: serde_json::json!({"verified": true}),
                events: Vec::new(),
            })
        }
    }

    struct CountingExecutor {
        calls: Arc<AtomicUsize>,
    }

    impl WorkflowExecutor for CountingExecutor {
        fn execute(
            &self,
            _workflow: &GeoWorkflow,
            _context: &WorkflowExecutionContext,
        ) -> Result<WorkflowExecution, WorkflowExecutionError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(WorkflowExecution {
                result_digest: "sha256:counted".into(),
                output: serde_json::json!({"counted": true}),
                evidence: serde_json::json!({"verified": true}),
                events: Vec::new(),
            })
        }
    }

    #[test]
    fn envelope_old_json_deserializes_with_empty_runtime_identity_fields() {
        let envelope = serde_json::from_value::<CommandEnvelope>(serde_json::json!({
            "id": Uuid::nil(),
            "origin": "cli",
            "timestamp": "2026-08-22T00:00:00Z",
            "command": { "type": "undo" }
        }))
        .expect("legacy envelope");
        assert!(envelope.workflow_digest.is_none());
        assert!(envelope.source_snapshots.is_empty());
        assert!(envelope.input_snapshots.is_empty());
    }

    #[test]
    fn apply_undo_redo_changes_and_restores_project_state() {
        let mut project = Project::new("runtime");
        let mut bus = CommandBus::new(project.clone());
        let command = layer_command("wards");
        let layer_id = command.id;
        bus.apply(&mut project, command).expect("apply");
        assert_eq!(project.workspace().layers.len(), 1);
        let applied_digest = project.state_digest();
        bus.undo(&mut project).expect("undo");
        assert!(project.workspace().layers.is_empty());
        bus.redo(&mut project).expect("redo");
        assert_eq!(project.workspace().layers[0].id.0, layer_id);
        assert_eq!(project.state_digest(), applied_digest);
    }

    #[test]
    fn workflow_run_validates_digest_and_input_snapshot_before_mutation() {
        let workflow = nagoya_population_density_template();
        let digest = WorkflowDigest::new(workflow.stable_digest().expect("digest"));
        let mut envelope = CommandEnvelope::new(
            CommandOrigin::Ai,
            Command::RunWorkflow {
                workflow_id: workflow.id,
            },
        )
        .with_workflow_digest(digest);
        let boundary = workflow.input_contracts[0]
            .source_snapshot
            .clone()
            .expect("source");
        let population = workflow.input_contracts[1]
            .source_snapshot
            .clone()
            .expect("source");
        envelope = envelope
            .with_input_snapshot(InputSnapshot::new("boundary", boundary).with_crs(Crs::wgs84()))
            .with_input_snapshot(InputSnapshot::new("population", population));

        let mut project = Project::new("runtime");
        let mut bus = CommandBus::new(project.clone());
        bus.register_workflow(workflow.clone()).expect("register");
        let executor = EchoExecutor {
            result_digest: "sha256:echo".into(),
            fail: false,
        };
        let execution = bus
            .apply_with_executor(&mut project, envelope.clone(), &executor)
            .expect("run");
        assert_eq!(project.workspace().provenance.entries.len(), 1);
        assert_eq!(execution.result_digest.as_deref(), Some("sha256:echo"));

        let mut tampered = envelope;
        tampered.workflow_digest = Some(WorkflowDigest::from("sha256:tampered"));
        let before = project.state_digest();
        assert!(matches!(
            bus.apply(&mut project, tampered),
            Err(CommandError::WorkflowDigestMismatch { .. })
        ));
        assert_eq!(project.state_digest(), before);
    }

    #[test]
    fn failed_executor_does_not_change_project_or_history() {
        let workflow = nagoya_population_density_template();
        let digest = WorkflowDigest::new(workflow.stable_digest().expect("digest"));
        let mut envelope = CommandEnvelope::new(
            CommandOrigin::Ai,
            Command::RunWorkflow {
                workflow_id: workflow.id,
            },
        )
        .with_workflow_digest(digest);
        for contract in &workflow.input_contracts {
            if let Some(source) = &contract.source_snapshot {
                envelope = envelope
                    .with_source_snapshot(source.clone())
                    .with_input_snapshot(InputSnapshot::new(contract.name.clone(), source.clone()));
            }
        }
        let mut project = Project::new("runtime");
        let before = project.state_digest();
        let mut bus = CommandBus::new(project.clone());
        bus.register_workflow(workflow).expect("register");
        let executor = EchoExecutor {
            result_digest: "sha256:never".into(),
            fail: true,
        };
        assert!(matches!(
            bus.apply_with_executor(&mut project, envelope, &executor),
            Err(CommandError::WorkflowExecutionFailed { .. })
        ));
        assert_eq!(project.state_digest(), before);
        assert!(bus.history().is_empty());
        assert!(bus.audit_log().is_empty());
    }

    #[test]
    fn tampered_workflow_or_source_is_rejected_before_executor_call() {
        let calls = Arc::new(AtomicUsize::new(0));
        let executor = CountingExecutor {
            calls: calls.clone(),
        };
        let workflow = nagoya_population_density_template();
        let digest = WorkflowDigest::new(workflow.stable_digest().expect("digest"));
        let mut envelope = CommandEnvelope::new(
            CommandOrigin::Ai,
            Command::RunWorkflow {
                workflow_id: workflow.id,
            },
        )
        .with_workflow_digest(digest);
        for contract in &workflow.input_contracts {
            if let Some(source) = &contract.source_snapshot {
                envelope = envelope
                    .with_source_snapshot(source.clone())
                    .with_input_snapshot(InputSnapshot::new(contract.name.clone(), source.clone()));
            }
        }

        let mut project = Project::new("tamper-workflow");
        let mut bus = CommandBus::new(project.clone());
        bus.register_workflow(workflow.clone()).expect("register");
        bus.workflows
            .get_mut(&workflow.id)
            .expect("registered workflow")
            .steps[0]
            .parameters = serde_json::json!({"tampered": true});
        assert!(matches!(
            bus.apply_with_executor(&mut project, envelope.clone(), &executor),
            Err(CommandError::WorkflowDigestMismatch { .. })
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        let mut source_tampered = envelope.clone();
        source_tampered.input_snapshots[0].source.uri = "tampered://source".into();
        let mut source_project = Project::new("tamper-source");
        let mut source_bus = CommandBus::new(source_project.clone());
        source_bus
            .register_workflow(workflow.clone())
            .expect("register");
        assert!(matches!(
            source_bus.apply_with_executor(&mut source_project, source_tampered, &executor),
            Err(CommandError::InputSnapshotMismatch { .. })
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        let mut source_identity_tampered = envelope;
        source_identity_tampered.source_snapshots[0].uri = "tampered://source-only".into();
        let mut source_identity_project = Project::new("tamper-source-identity");
        let mut source_identity_bus = CommandBus::new(source_identity_project.clone());
        source_identity_bus
            .register_workflow(workflow)
            .expect("register");
        assert!(matches!(
            source_identity_bus.apply_with_executor(
                &mut source_identity_project,
                source_identity_tampered,
                &executor
            ),
            Err(CommandError::InputSnapshotMismatch { .. })
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn persist_load_replay_detects_tamper_and_preserves_state_digest() {
        let path = std::env::temp_dir().join(format!("genegis-command-{}.json", Uuid::new_v4()));
        let mut project = Project::new("replay");
        let mut bus = CommandBus::new(project.clone());
        bus.apply(&mut project, layer_command("wards"))
            .expect("apply");
        bus.persist(&path).expect("persist");
        let mut loaded = CommandBus::load(&path).expect("load");
        let replayed = loaded.replay().expect("replay");
        assert_eq!(replayed.state_digest(), project.state_digest());

        let mut json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("read log")).expect("json");
        json["commands"][0]["command"]["name"] = serde_json::json!("tampered");
        std::fs::write(&path, serde_json::to_string(&json).expect("json")).expect("tamper");
        assert!(matches!(
            CommandBus::load(&path),
            Err(CommandError::LogDigestMismatch { .. })
        ));
        std::fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn persisted_workflow_replay_preserves_result_digest() {
        let path =
            std::env::temp_dir().join(format!("genegis-workflow-command-{}.json", Uuid::new_v4()));
        let workflow = nagoya_population_density_template();
        let digest = WorkflowDigest::new(workflow.stable_digest().expect("digest"));
        let mut envelope = CommandEnvelope::new(
            CommandOrigin::Ai,
            Command::RunWorkflow {
                workflow_id: workflow.id,
            },
        )
        .with_workflow_digest(digest);
        for contract in &workflow.input_contracts {
            if let Some(source) = &contract.source_snapshot {
                envelope = envelope
                    .with_input_snapshot(InputSnapshot::new(contract.name.clone(), source.clone()));
            }
        }
        let mut project = Project::new("workflow-replay");
        let mut bus = CommandBus::new(project.clone());
        bus.register_workflow(workflow).expect("register");
        let executor = EchoExecutor {
            result_digest: "sha256:stable-output".into(),
            fail: false,
        };
        let applied = bus
            .apply_with_executor(&mut project, envelope.clone(), &executor)
            .expect("execute");
        let expected_state = project.state_digest();
        bus.persist(&path).expect("persist");

        let mut loaded = CommandBus::load(&path).expect("load");
        let replayed = loaded.replay().expect("recorded replay");
        assert_eq!(replayed.state_digest(), expected_state);
        assert_eq!(
            loaded
                .workflow_execution(envelope.id)
                .expect("execution record")
                .result_digest,
            applied.result_digest.as_deref().expect("result digest")
        );

        let replayed_live = loaded.replay_with_executor(&executor).expect("live replay");
        assert_eq!(replayed_live.state_digest(), expected_state);
        std::fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn persisted_undo_redo_event_stream_replays_the_same_cursor_state() {
        let path = std::env::temp_dir().join(format!("genegis-command-{}.json", Uuid::new_v4()));
        let mut project = Project::new("undo-replay");
        let mut bus = CommandBus::new(project.clone());
        bus.apply(&mut project, layer_command("first"))
            .expect("first");
        bus.apply(
            &mut project,
            CommandEnvelope::new(CommandOrigin::Cli, Command::Undo),
        )
        .expect("undo");
        bus.apply(&mut project, layer_command("second"))
            .expect("branch");
        let expected = project.state_digest();
        bus.persist(&path).expect("persist");
        let mut loaded = CommandBus::load(&path).expect("load");
        let replayed = loaded.replay().expect("replay");
        assert_eq!(replayed.state_digest(), expected);
        assert_eq!(replayed.workspace().layers.len(), 1);
        assert_eq!(replayed.workspace().layers[0].name, "second");
        assert!(!loaded.can_undo() || loaded.cursor() == 1);
        std::fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn view_camera_and_visibility_are_real_state_mutations() {
        let mut project = Project::new("view");
        let source = crate::DataSource::new("source", crate::SourceKind::File, "file:///source");
        let source_id = source.id;
        project.workspace_mut().add_source(source);
        let layer = Layer::new("layer", LayerKind::Vector, source_id);
        let layer_id = layer.id.0;
        project.workspace_mut().add_layer(layer);
        let view = View::new("map", ViewKind::Map);
        let view_id = view.id.0;
        project.workspace_mut().add_view(view);
        let mut bus = CommandBus::new(project.clone());
        bus.apply(
            &mut project,
            CommandEnvelope::new(
                CommandOrigin::Ui,
                Command::SetLayerVisibility {
                    layer_id,
                    visible: false,
                },
            ),
        )
        .expect("visibility");
        bus.apply(
            &mut project,
            CommandEnvelope::new(
                CommandOrigin::Ui,
                Command::SetViewCamera {
                    view_id,
                    center: [136.9, 35.18],
                    zoom: 11.0,
                },
            ),
        )
        .expect("camera");
        assert!(!project.workspace().layers[0].visible);
        assert_eq!(project.workspace().views[0].center, Some([136.9, 35.18]));
        bus.undo(&mut project).expect("undo camera");
        assert_eq!(project.workspace().views[0].center, None);
        bus.undo(&mut project).expect("undo visibility");
        assert!(project.workspace().layers[0].visible);
    }
}
