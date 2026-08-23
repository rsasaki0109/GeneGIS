# ADR 0011: Derive Trust from a Versioned Policy and Verification Graph

- **Status:** Accepted
- **Date:** 2026-08-23

## Context

The Nagoya workflow previously exposed a boolean `verification_passed`. A
boolean cannot explain which release rule was applied, whether a verifier was
independent from the executor, or why a result did not reach a stronger trust
state. It can also become stale when evidence or policy changes.

## Decision

GeneGIS stores a versioned `VerificationPolicy`, a separate
`VerificationGraph`, and their evidence in the canonical execution result.
The receipt derives one monotonic trust level from those objects:

1. `exploratory` requires no release claim;
2. `replayable` requires stable workflow, result, backend, and artifact
   identities;
3. `verified` additionally requires compatible GeoContracts, admitted source
   snapshots, and every policy-required independent check within tolerance;
4. `attested` additionally requires a valid signature by a policy-accepted
   attester.

The verification graph is distinct from the execution workflow. Every node
names its claim, verifier implementation, evidence inputs, dependency claims,
independence class, and numeric tolerance. Its canonical digest is insensitive
to serialization order and is bound into the execution result digest.

Release checks fail closed. Missing contracts, source evidence, content
digests, independence metadata, tolerances, or policy fields prevent
`verified`. The legacy boolean remains temporarily for compatibility, but
receipt construction rejects it when it disagrees with policy-derived trust.

## Consequences

- A UI, CLI, or agent cannot promote a result by setting a boolean.
- Re-running the executor's own implementation is not accepted as independent
  verification unless a policy explicitly permits that class.
- Policy and verification-graph changes alter canonical result identity.
- `attested` means accepted integrity evidence, not proof that the spatial
  conclusion is true.
- Capsule verification can reuse the same policy engine without an LLM or the
  producing GeneGIS service.
