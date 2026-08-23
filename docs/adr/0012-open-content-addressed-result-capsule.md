# ADR 0012: Use an Open Content-Addressed Result Capsule

- **Status:** Accepted
- **Date:** 2026-08-23

## Context

A GeneGIS result must remain inspectable and verifiable without the producing
service, planner, UI, or LLM. A receipt alone does not carry the numeric result,
workflow, policy, verification graph, and rendered artifacts whose identities
it claims.

## Decision

The initial capsule is an ordinary directory described by a strict
`capsule.json` manifest. Every subject has a portable relative path, semantic
role, media type, exact byte length, and SHA-256 digest. The Nagoya profile
contains typed analysis JSON, Command, Workflow Graph, Execution Receipt,
VerificationPolicy, Verification Graph, HTML, and PNG.

The offline verifier:

- rejects absolute, parent, non-normal, duplicate, missing, and non-regular
  subject paths;
- verifies every subject's byte length and digest;
- optionally requires exact equality with an external policy;
- validates Command, Workflow Graph, receipt, policy, and verification-graph
  identities;
- recomputes workflow, verification-graph, artifact, analysis, and canonical
  result digests;
- derives trust again from normalized evidence and rejects stored-state drift.

JSON floating-point values are canonicalized to nine decimal places before
semantic digesting. This is finer than 0.1 mm for geographic coordinates while
removing adjacent IEEE-754 parse differences across portable JSON round trips.

## Consequences

- `genegis capsule verify PATH --policy POLICY` requires no network, server,
  agent, or LLM.
- The directory can later be packaged as ZIP or mapped into Workflow Run
  RO-Crate without changing subject identities.
- A capsule without an externally trusted digest, policy, or attestation proves
  internal consistency, not publisher identity.
- Signature and standards exports remain separate P2 work.
