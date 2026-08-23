# ADR 0013: Classify Semantic Changes and Bind Approval to Digests

- **Status:** Accepted
- **Date:** 2026-08-23

## Context

Byte diffs make repeated executions noisy because runtime UUIDs, timestamps,
and retrieval observations change. Conversely, a free-form “approved” flag can
be replayed after data, contracts, workflow parameters, policies, results, or
artifacts change.

## Decision

`capsule diff` compares known JSON subjects at leaf level after removing only
runtime identity fields. It classifies changes as source, contract, workflow,
result, verification, policy, or artifact. Unknown manifest roles remain
explicitly `unclassified`; they never disappear into a generic success.

An `AnalysisApproval` binds the exact capsule manifest, canonical result,
Workflow Graph, VerificationPolicy, Verification Graph, and optional semantic
diff digests. Validation recomputes all identities. Any mismatch makes the
approval stale.

## Consequences

- Equivalent replays produce an empty semantic diff despite new runtime IDs.
- Review tooling can group and navigate changes without reading raw JSON.
- An approval cannot authorize a later result, policy, graph, or diff.
- Approval records review intent; cryptographic signer authentication remains
  the separate attestation work in P2.
