# ADR 0009: Command Runtime, Evidence Receipt, and Replay Boundary

- Status: Accepted
- Date: 2026-08-23

## Context

ADR 0008 defined a validated `GeoWorkflow` DAG and a stable graph digest, but
the command bus only retained a history cursor. A cursor cannot restore a
project, prove that a workflow used the graph that was approved, or detect a
modified command log. The Nagoya north-star path also constructed command and
provenance JSON beside the execution path, so CLI, AI, and future UI adapters
could drift.

## Decision

1. `CommandEnvelope` carries an optional typed `WorkflowDigest`, structured
   source snapshots, and named typed input snapshots. The fields use serde
   defaults so envelopes written before this ADR remain readable. A
   `RunWorkflow` command is rejected unless its graph is registered, validates,
   and produces exactly the envelope digest.
2. `CommandBus::apply` is the project-state dispatcher. Layer creation,
   deletion, visibility, camera changes, and workflow provenance mutate the
   supplied `Project`; undo and redo restore captured semantic states rather
   than only moving a cursor. Catalog-specific registries may keep their own
   adapter state, but their authorization still uses the same envelope type.
3. `WorkflowExecutor` is the typed data-plane boundary for `RunWorkflow`.
   After graph, digest, and input-snapshot validation, `CommandBus` passes a
   command context to the executor. Project mutation, history, audit, and the
   execution record are committed only after a non-empty result digest is
   returned; executor failure leaves all of them unchanged. The Nagoya
   executor checks the graph's deterministic topological order before running
   the concrete load → area/density → independent verification → render
   stages. Rendering is gated on verification and is withheld on failure.
   The pipeline also rejects a modified approved Nagoya plan before binding
   runtime source snapshots; it never silently substitutes a different DAG.
4. Persisted command logs contain the initial project, active commands, full
   audit events, cursor, workflow definitions, successful workflow outputs,
   and canonical state digest. A SHA-256 log digest is computed over
   canonical JSON with the digest field omitted. Load rejects tampering, and
   replay rejects a result-state digest mismatch. Replay can use captured
   workflow outputs without creating new observations, or accept a live
   executor for deterministic recomputation.
5. Analysis builds a single `ExecutionReceipt` through the dispatcher. The
   receipt records command ID/time, workflow ID/digest, source/input snapshots,
   typed CRS, coordinate/value units, area method, verifier/checks,
   engine/build identity, executor evidence, retrieval/observation events, and
   the resulting canonical state digest. The result digest is computed from
   actual output fields (Nagoya ward code/name, population, area, density,
   geometry, style), verification evidence, and HTML/PNG artifact digests;
   event timestamps are excluded. `NagoyaExecutionOutput` carries the typed
   analysis, verification result, and render payload from that one executor
   call. Ask/CLI/AI assembly consumes that output and never reruns the
   executor to create a receipt. The existing command/workflow/provenance
   tuple remains available to legacy callers, while the north-star ask result
   exposes the structured receipt.

## Consequences

- The same initial project plus command/event log is sufficient to reproduce
  the active state and its digest without trusting an in-memory cursor.
- A stale workflow, stale source snapshot, unknown CRS, malformed graph, or
  modified log fails before a successful receipt is emitted.
- Runtime timestamps remain event evidence and are excluded from semantic
  project/workflow identities. Command IDs are persisted, so generated layer
  identities are deterministic during replay.
- Source snapshot identity is separate from retrieval/observation events. A
  local read uses the command event time, while an external adapter's supplied
  `retrieved_at` is retained on its event and never changes the stable result
  digest.
- CLI and AI north-star routes use the same `CommandBus` and receipt builder;
  only the command origin and planner mode differ.
- The core depends on the workflow IR for the fail-closed `RunWorkflow`
  boundary; the workflow crate remains independent of core, avoiding a cycle.

## Verification

- `genegis-core` tests cover legacy envelope deserialization, real layer/view
  mutations, apply/undo/redo restoration, digest mismatch before mutation,
  persist/load/replay, branch replay, tamper detection, executor failure
  atomicity, and the rule that tampered workflow/source snapshots never call
  an executor.
- `genegis-analysis` north-star tests exercise the same dispatcher and assert
  receipt/provenance propagation alongside CRS, units, source checksum,
  verification results, topological execution evidence, verify-before-render
  artifact digests, deterministic actual output digests, explicit CLI/AI
  origins, and separated source observation events. Receipt assembly consumes
  the typed executor output, so no second Nagoya executor call is made.
