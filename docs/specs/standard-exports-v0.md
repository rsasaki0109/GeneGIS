# GeneGIS Standard Exports v0

`genegis capsule export-standards CAPSULE OUT` verifies the capsule first and
writes the following views.

| File | Profile | Identity mapping |
| --- | --- | --- |
| `prov.json` | PROV-JSON | capsule subjects → entities; command run → activity; engine → software agent |
| `ro-crate-metadata.json` | Workflow Run RO-Crate 0.5 | workflow → `ComputationalWorkflow`; receipt → `CreateAction`; sources/results → `workExample` values |
| `openlineage-complete.json` | OpenLineage RunEvent 1.0.0 | command ID → run ID; workflow goal → job; source snapshots/artifacts → datasets |
| `in-toto-statement.json` | in-toto Statement v1 | analysis/map subjects → signed resource descriptors; proof identities → predicate |
| `ogc-process-description.json` | OGC API - Processes 1.0 | offline verifier → synchronous process |
| `ogc-execute-request.json` | OGC execution request | capsule and external policy → process inputs |

All copied capsule subjects retain exact SHA-256 identities. OpenLineage custom
facets include `_producer` and `_schemaURL`; PROV and RO-Crate additions use the
`https://genegis.org/ns#` namespace.

The local structural validators are intentionally narrower than the standards.
Release evidence therefore also records independent profile/schema validation.
The DSSE verifier authenticates payload type and exact Statement bytes using
pre-authentication encoding, then checks every Statement subject against the
local capsule without network access.
