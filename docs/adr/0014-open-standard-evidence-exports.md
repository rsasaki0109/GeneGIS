# ADR 0014: Project capsules into open provenance and execution standards

## Status

Accepted — 2026-08-23.

## Context

GeneGIS must be verifiable without a GeneGIS service, but a private provenance
vocabulary would make the result difficult to archive or connect to existing
research and data platforms. No single standard carries every GeoContract,
policy, verification graph, signature, and execution concern.

## Decision

Keep the GeneGIS capsule as the normative evidence package and publish
loss-aware projections:

- [PROV-JSON](https://www.w3.org/submissions/prov-json/) for entity, activity,
  agent, use, generation, and association provenance;
- [Workflow Run RO-Crate 0.5](https://w3id.org/ro/wfrun/workflow/0.5) over
  [RO-Crate 1.1](https://www.researchobject.org/ro-crate/1.1/) for a portable
  workflow-run package;
- [OpenLineage RunEvent](https://openlineage.io/docs/spec/object-model/) for
  run, job, input dataset, and output dataset integration;
- [in-toto Statement v1](https://github.com/in-toto/attestation/blob/main/spec/v1/statement.md)
  inside a [DSSE](https://github.com/secure-systems-lab/dsse) Ed25519 envelope
  for offline integrity and signer authentication;
- [OGC API - Processes 1.0](https://docs.ogc.org/is/18-062r2/18-062r2.html)
  for a local synchronous capsule-verification process fixture.

GeneGIS-only fields use `https://genegis.org/ns#`. The signed predicate states
explicitly that signature integrity is not spatial truth. Policy-derived trust
continues to require the independent verification evidence.

## Consequences

The Workflow Run RO-Crate export includes the actual capsule payload, not only
detached metadata. Export first performs offline capsule verification. The same
result and graph identities are retained across all projections. Consumers may
lose GeneGIS-specific semantics if they ignore namespaced fields, so the
capsule remains authoritative.

External acceptance uses `rocrate-validator` against
`workflow-run-crate-0.5`, the official OpenLineage 1.0.0 JSON Schema, and a
PROV-JSON parser in addition to the built-in fail-closed validators.
