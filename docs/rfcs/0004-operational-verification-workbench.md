# RFC 0004: Operational Verification Workbench

- **Status:** Accepted for implementation
- **Date:** 2026-08-23
- **Scope:** Phase 12–13 operationalization after proof-carrying analysis

## Decision

GeneGIS will become an operational verification workbench by connecting
editing, cartography, external GIS engines, large cloud-native objects, diverse
source domains, and human review to the existing Command, Workflow Graph,
GeoContract, Verification Graph, and capsule boundary.

It will not reproduce the QGIS desktop interaction model or claim that a source
is universally true. The product category remains an evidence-first control
plane over specialized open GIS engines and formats.

## Problems addressed

Current open GIS components are strong within their respective centers of
gravity, but a consequential result often crosses several boundaries:

1. interactive edits are disconnected from an immutable analytical receipt;
2. an external engine invocation can hide privileges, build drift, warnings,
   or an unrepresented semantic conversion;
3. cloud-native formats can silently fall back to whole-object transfer;
4. schema validity and source reputation can be mistaken for factual truth;
5. logs and provenance can exist without a review workflow that humans can
   complete accurately and quickly.

GeneGIS addresses those cross-boundary failures. It continues to delegate
authoritative storage to PostGIS, specialized algorithms to GRASS, broad format
and processing coverage to GDAL/QGIS, and local analytics to DuckDB.

## Product invariants

1. Every feature or style mutation is a Command and a Workflow node. Direct
   project-state mutation from UI, agent, plugin, or adapter code is rejected.
2. Undo and redo append provenance events; they never erase the original event.
3. An adapter executes only after exact manifest, backend build, operation,
   capability, and evidence-hook admission.
4. A semantic boundary that cannot be represented is marked `opaque` and is
   ineligible for verified trust unless a future policy explicitly defines an
   independent verifier for that boundary.
5. Network and database writes are denied by default. Read-only does not mean
   trusted: source, contract, and output verification remain independent gates.
6. Range-read claims include server-side byte/request evidence. Client intent
   alone is insufficient.
7. Source Assurance reports evidence and limitations. `corroborated` means that
   a named use-case policy passed, never that universal truth was proved.
8. Human UX metrics come from human sessions. Automated key injection cannot
   satisfy a timing or comprehension gate.
9. The Nagoya offline, no-LLM path remains a release invariant.

## Architecture additions

### Source Assurance

`genegis-contract` contains a versioned evidence dossier and policy evaluator
covering:

- immutable snapshot identity;
- publisher and authority relationship;
- publication and assessment time plus observed age;
- schema, completeness, spatial, temporal, anomaly, and cross-source checks;
- independently governed or independently measured corroboration;
- quantified uncertainty scope;
- correction challenges and unresolved disputes;
- explicit limitations and non-claims.

The evaluator derives `unassessed`, `identified`, `checked`, or `corroborated`
and can be required by VerificationPolicy. Missing evidence fails closed.

### Adapter Manifest and admission

`genegis-adapter` owns the small external-engine boundary. A manifest binds:

- adapter and semantic-operation versions;
- backend family, engine version, build digest, and components;
- input/output GeoContracts;
- file, network, database, process, native-code, and GPU capabilities;
- determinism classification;
- evidence hooks;
- an explicit opaque flag.

Runtime capability declarations must exactly match the reviewed operation. This
prevents an adapter from under-declaring a write or process-spawn privilege.

### External adapters

Initial implementations run against pinned container digests in the conformance
environment because the current host has Docker 29.6.2 but no host-installed
PostGIS, GRASS, QGIS Processing, or GDAL executables. Container use is not a
substitute for identity: image manifest digest, engine version, loaded modules,
arguments, environment, inputs, outputs, warnings, and resource metrics enter
the receipt.

- PostGIS runs a read-only transaction with SQL admission, isolation context,
  server/PostGIS identity, query-plan digest, and output snapshot.
- GRASS runs a named module in a disposable mapset and sandbox, recording
  region, projection, flags, module version, and artifacts.
- QGIS Processing runs a named provider/algorithm through `qgis_process` in a
  disposable profile, recording provider version, normalized parameters, logs,
  and outputs.

Arbitrary SQL, shell, Python, GRASS command strings, and QGIS expressions are
not generic verified operations. They remain blocked or opaque until a typed
operation and verifier are defined.

### Editing and cartography

The first editing surface is operation-centric rather than layer-panel-centric:

- create, update, and delete feature geometry;
- update attributes with schema and unit validation;
- split, merge, and repair operations with topology postconditions;
- classified style, legend, labels, and deterministic map layout;
- review diff, approve digest, replay, undo/redo, and seal capsule.

The map is the primary evidence view, while source, contract, workflow, and
verification detail remain reachable without raw JSON.

### Large-object execution

COG, GeoParquet, COPC, and PMTiles runners expose a common I/O receipt:

- object size and logical dataset size;
- request count, requested ranges, bytes transferred, cache hits, and fallback;
- decode/compute/upload/render durations;
- peak resident memory and GPU adapter/backend;
- selected spatial/temporal predicate and output digest.

## Acceptance matrix

### A. Editing and cartography

- Every supported edit and style operation has apply, undo, redo, replay, and
  digest-stability tests through Command plus DAG.
- At least 30 negative cases cover invalid geometry, CRS/unit mismatch, stale
  revision, schema/null violation, topology damage, policy denial, and approval
  digest drift; false accepted edits are zero.
- Replaying 100 mixed edit/style commands produces identical canonical project,
  workflow, and artifact digests.
- SVG/PNG/map package outputs carry source, CRS, units, style, font, renderer,
  and provenance identity.

### B. Three real external adapters

- PostGIS, GRASS, and QGIS Processing each run at least five typed positive and
  five typed negative conformance cases against pinned container digests.
- Database/network writes, undeclared process spawn, backend drift, missing
  evidence, and opaque semantics have zero false admissions.
- Equivalent geometry, reprojection, area, join, null, multipart, and ordering
  cases agree with the native oracle within policy tolerance.
- The report distinguishes `not installed`, `not supported`, `opaque`,
  `replayable`, and `verified`; none are silently collapsed to success.

### C. Large cloud-native objects

The reproducible benchmark lane uses, for each format, an object of at least
256 MiB and a logical decoded dataset of at least 1 GiB. A smaller deterministic
CI fixture exercises the same request assertions.

- COG window, GeoParquet row-group/bbox, COPC hierarchy-node, and PMTiles tile
  selection transfer no more than 20% of the object and no single response over
  8 MiB unless a manifest-declared format constraint explains the exception.
- Whole-object fallback is visible and cannot satisfy the optimized-I/O gate.
- p50/p95 wall time, peak RSS, bytes, request count, decoded records/points,
  GPU upload time, first-frame time, and steady-state frame rate are recorded.
- The baseline budget is peak RSS at most 1 GiB and local-fixture first frame at
  most 2 seconds for the selected view. Any revised budget requires an ADR and
  before/after evidence.

### D. Diverse real-data verification

The corpus contains at least five independently sourced domains: administrative
boundaries, population/statistics, raster observation, point cloud, and temporal
change. Each records license, immutable snapshot, source assurance, GeoContract,
oracle method, and known limitations.

- At least 100 total mutations span source, schema, CRS/axis/unit, geometry,
  topology, coverage, time, uncertainty, adapter, result, policy, and artifact.
- Mutation score is at least 95%; false `verified` or `attested` is zero.
- No single executor implementation is accepted as its own independent oracle.

### E. Human trust UX

- A fixed preregistered corpus contains at least 12 tasks across source drift,
  edit conflict, CRS/unit error, adapter denial, cloud fallback, changed result,
  uncertainty, and open dispute.
- At least three human reviewers complete the corpus without raw JSON.
- Aggregate correctness is at least 90%, median diagnosis time is at most 120
  seconds, and median interactions from map to decisive evidence are at most 2.
- The report includes anonymized reviewer ID, task/corpus version, per-task wall
  time, answer, correctness, interaction count, aborts, runner identity, and
  aggregate statistics. Automated sessions are labeled separately and cannot
  pass this gate.

### F. Source truth boundaries

- Every release source has a Source Assurance dossier copied into the capsule
  and bound by digest to trust evidence.
- Policies can require authority class, freshness, checks, uncertainty,
  limitations, independent corroboration, and resolved disputes.
- A mirror of the same publication never counts as independent corroboration.
- Stale data, failed checks, missing limitation, identity mismatch, and open
  dispute mutations cannot obtain verified trust.
- User-facing language always states the policy scope and avoids “proved true”.

## Delivery and reporting

Architecture decisions live in `docs/adr/`; implementation and benchmark
progress lives in `docs/roadmap/phase-12-operational-verification.md`. The final
machine-readable acceptance report labels every row `pass`, `fail`, or
`not_measured` and records exact commands, fixture paths, backend/container
digests, host identity, and artifact digests.

The implementation may not declare completion while any matrix row lacks direct
evidence. Human measurements and external-engine execution cannot be replaced
by unit-test mocks.
