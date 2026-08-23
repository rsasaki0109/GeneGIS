# ADR 0008: Explicit Workflow DAG and Stable Workflow Digest

- Status: Accepted
- Date: 2026-08-22

## Context

`GeoWorkflow` was serialized as an ordered list of steps. A step had a
runtime UUID, but no stable graph identity, dependency edge, or explicit data
reference. Execution therefore could not reject a cycle, an unresolved input,
or a disconnected operation before a command reached an engine. The UUID and
review status also changed between executions, which made the serialized
workflow unsuitable as a replay identity.

The north-star workflow, 「名古屋市の人口密度を表示」, needs to be represented
as one verifiable graph. Spatial graph inputs must retain their CRS, coordinate
unit, value unit, and source snapshot so that a replay does not silently change
the meaning of a value.

## Decision

1. Every `WorkflowStep` has a deterministic `stable_id`, separate from its
   execution UUID. A node also carries explicit `depends_on` edges and
   structured `inputs`/`outputs` references. A reference with no node points
   to a named `WorkflowInputContract`.
2. `GeoWorkflow::validate()` runs before execution. It rejects duplicate IDs,
   self and unresolved dependencies, unresolved input/output references,
   cycles, missing input contracts, and disconnected or unreachable nodes. The
   topological order is deterministic because node IDs are ordered
   lexicographically when several nodes are ready.
3. `WorkflowInputContract` stores an optional typed CRS, its derived
   coordinate-axis unit, a value unit, and a `SourceSnapshot`. A declared CRS
   must be known and its coordinate unit must match the registry definition.
4. `GeoWorkflow::stable_digest()` hashes canonical JSON with SHA-256. Object
   keys are sorted and nodes are ordered by stable ID. Workflow UUID,
   execution review status, and source/citation `retrieved_at` events are
   excluded; graph structure, operation parameters, contracts, checksums,
   licenses, versions, and citations remain part of the digest.
5. New templates use explicit DAG edges and graph output references. JSON
   written by the previous ordered-step schema remains deserializable through
   serde defaults; a legacy list is interpreted as a deterministic linear DAG
   for validation and digest calculation.

## Consequences

- A command runtime can fail closed before invoking an operation when the
  graph is malformed or spatial input semantics are incomplete.
- Replaying the same graph with a different runtime UUID, review transition,
  or retrieval event produces the same stable digest. Changing a parameter,
  edge, contract, checksum, or source identity changes it.
- The legacy `steps`, `inputs`, and `outputs` fields remain available to
  existing UI and receipt consumers. The typed `input_contracts` and
  `output_refs` fields are the authoritative graph contract for new code.
- Command apply/undo/redo and execution receipt persistence will consume the
  validated topological order in the next P0-2 slice.

## Verification

`genegis-workflow` tests validate every migrated template, assert the Nagoya
topological order and digest determinism, prove runtime-only fields do not
perturb the digest, round-trip a legacy workflow JSON payload, and reject
duplicate IDs, self/unresolved dependencies, cycles, disconnected nodes,
unknown input contracts, and CRS/unit contract mismatches.
