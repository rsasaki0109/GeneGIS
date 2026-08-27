# GeneGIS Long-Term Product Roadmap

**Planning window:** 36 months from 2026-08.

**Product outcome:** GeneGIS becomes the open, verifiable spatial workbench
where a user can move from intent to a live map, analysis, review, and
publication without leaving the Workflow Graph or losing source truth.

This roadmap coordinates product horizons. Detailed architecture and trust
decisions continue to live in RFCs and ADRs, and every delivery remains
subordinate to the north-star prompt:

```text
名古屋市の人口密度を表示
```

## Current closure state (2026-08-27)

- Horizon 2 is complete (`H2.1`–`H2.6`).
- Horizon 3 is complete (`H3.1`–`H3.6`).
- Horizon 4 is accepted through `H4.6`. The evaluator, profiles, APIs, and
  fail-closed tests are complete, with passing local-first and air-gapped
  receipts. Managed-cloud execution is explicitly waived without a performance
  pass claim.
- Phase 14 M1 is complete with a release-mode NVIDIA/DX12 hardware receipt.
  No external acceptance gates remain: Phase 12 Gate E human review and H4.6
  managed-cloud execution were explicitly waived by the project owner on
  2026-08-27, without claiming either external validation passed.
- The deterministic EPSG:6675 COPC/LOD1 acceptance fixture, per-metric fixture
  identity, GPU FPS budget, Command + Workflow acceptance binary, and outer
  process timeout are ready. Twenty accepted M1 runs from one release
  executable record a nearest-rank p95 first frame of 1.1180932 seconds and a
  minimum 555.3010 FPS over 120 frames per run on a GTX 1660 Ti.
- Gate E has a pinned release CLI, digest-bound study manifest, sealed
  aggregate receipt, and three-script execution packet. Its automated smoke
  remains excluded; the human study is waived and no human result is claimed.
- The software acceptance corpus is closed at 206 passing tests with no
  failures. Five hardware-bound GPU tests are excluded from that count; the M1
  hardware path is covered by its separate release receipt. The managed-cloud
  class remains unmeasured under its documented waiver. The
  consolidated record is
  [`docs/reports/long-term-roadmap-software-acceptance.json`](../reports/long-term-roadmap-software-acceptance.json).

## Strategic pillars

| Pillar | User outcome | Platform commitment |
| --- | --- | --- |
| Compose | Build maps, analyses, dashboards, and narratives without code | Every UI action emits a typed Command and updates the Workflow Graph |
| Explore | Move fluidly across 2D, 3D, time, and linked statistics | One GPU renderer, bounded cloud reads, explicit performance budgets |
| Connect | Discover files, catalogs, services, geocoders, and live feeds | Adapter admission, open protocols, I/O receipts, no hidden vendor dependency |
| Decide | Compare places, routes, time periods, and scenarios | Verification and uncertainty are visible beside every result |
| Publish | Share an interactive result that can be independently reviewed | Digest-bound views, portable capsules, accessible embeds |
| Operate | Run repeatable spatial workflows in desktop, browser, and server modes | Same workflow contract, policy, provenance, and replay semantics everywhere |
| Extend | Add domain analysis without growing the core | Stable SDK, sandboxed plugins, signed capabilities, conformance tests |

## Delivery horizons

### Horizon 1 — Interactive foundation (0–6 months)

Finish the operational verification boundary and turn the existing showcase
capabilities into an interactive workbench surface.

| ID | Deliverable | Exit evidence |
| --- | --- | --- |
| `H1.1` | Operational verification release | Phase 12–13 capsule integration and final acceptance report are complete |
| `H1.2` | Native 3D scene | COPC points, extruded buildings, roads, and POIs render through WebGPU with orbit controls and hardware-bound first-frame/FPS evidence |
| `H1.3` | Linked dashboard runtime | Selection, filtering, KPI, histogram, and category widgets consume verified workflow outputs and share the result digest |
| `H1.4` | External service layers | WMS/WFS and OGC API discovery/read paths enter through adapter manifests with positive and fail-closed tests |
| `H1.5` | Temporal exploration | Time slider replays NDVI, change, and other epoch layers without bypassing source snapshots or workflow identity |

**Gate:** Do not start a second rendering stack or ship dashboard-only data
paths. Phase 14 M1–M4 are the implementation roadmap for this horizon.

### Horizon 2 — No-code analysis and publication (6–12 months)

Make common spatial work understandable, reusable, and publishable by people
who do not write GIS code.

| ID | Deliverable | Exit evidence |
| --- | --- | --- |
| `H2.1` | Visual workflow composer | Users can create, connect, inspect, rerun, and undo typed nodes; invalid CRS, units, schema, or capability edges fail before execution |
| `H2.2` | Geocoding adapter family | Batch and interactive geocoding use swappable providers, rate/privacy policies, confidence fields, and source receipts |
| `H2.3` | Analysis templates | At least ten verified templates cover density, exposure, accessibility, change, suitability, proximity, aggregation, and geocoding |
| `H2.4` | Narrative map composer | A sequence of map states, text, media references, and dashboards is saved as a digest-bound project view rather than copied screenshots |
| `H2.5` | Portable publishing | Share links, static exports, and embeddable viewers preserve attribution, limitations, verification state, and offline capsule download |
| `H2.6` | Desktop bridge | A thin plugin sends declared layers into a GeneGIS project capsule through the SDK and never invokes a direct-render shortcut |

**Gate:** Publishing cannot precede stable result identity and redaction rules.
The plugin catalog cannot accept third-party packages before SDK conformance and
signature verification are enforced.

#### H2.1 implementation status (complete 2026-08-26)

The workbench now exposes eleven reviewed templates through a no-code composer.
Every UI edit is a serialized `ComposerCommand`; sessions retain an append-only
event list and reversible graph states. Users can instantiate a draft, inspect
typed inputs/nodes/ports/dependencies, clone reviewed nodes, connect or
disconnect edges, edit the goal, undo/redo, and rerun the reviewed graph.

Execution uses the runtime-resolved source snapshots rather than catalog
placeholders. The North Star path proved that the dispatched command and
execution receipt carry the exact composer workflow digest. Unknown output
ports, cycles, CRS/unit disagreement, and unknown templates fail before
execution. A structurally valid edit with a new digest is retained as a draft
but receives HTTP 409 until an executor review admits that exact graph; undoing
to the reviewed digest restores execution.

Evidence is recorded in
[`docs/reports/horizon-2-h2-1-no-code-composer.json`](../reports/horizon-2-h2-1-no-code-composer.json).

#### H2.2 implementation status (complete 2026-08-26)

Interactive and batch geocoding now share one provider-neutral request and
receipt contract. The first slice includes a deterministic offline Nagoya
gazetteer and an allowlisted HTTP JSON provider. Both return validated WGS84
candidates with confidence, match kind, provider identity, and an immutable
source snapshot. Query text is never written to receipts; ordered SHA-256
digests preserve request identity instead.

Privacy policy is checked before transport, and request/candidate limits are
admitted under an explicit rate policy. Remote full-text requests require an
explicit policy and still pass the shared host allowlist. Malformed media
types, coordinates, confidence values, result identities, candidate counts,
adapter manifests, and capability declarations fail closed.

The Workbench exposes the governed path at `POST /api/geocode` and through an
interactive/batch UI. Execution always emits `RunWorkflow`; the CommandBus
result digest must equal the adapter receipt digest. Live HTTP verification
proved interactive and batch execution, verified source checksums, WGS84
output, raw-query redaction, and a pre-transport privacy rejection.

Evidence is recorded in
[`docs/reports/horizon-2-h2-2-geocoding-adapters.json`](../reports/horizon-2-h2-2-geocoding-adapters.json).

#### H2.3 implementation status (complete 2026-08-26)

The reviewed template catalog now exposes twelve digest-bound Workflow Graphs.
Each template declares one or more analytical categories and the evidence
profile required from its executor. A catalog conformance test requires
coverage of density, exposure, accessibility, change, suitability, proximity,
aggregation, and geocoding; dropping any required family fails CI.

The catalog audit also repaired reproducibility gaps exposed by the complete
analysis suite. Deterministic synthetic COG and LAS epochs are now committed
with their in-repository generators and explicit non-observational provenance,
so offline clean clones can execute NDVI and point-cloud change verification.
The immutable population-source checksum was corrected to the hash of the
unchanged tracked bytes across code, manifest, ADR, RFC, testkit, and reports.
The full analysis suite covers the actual density, exposure, accessibility,
change, aggregation, and geocoding executors; graph/category conformance covers
the suitability and proximity compositions.

Evidence is recorded in
[`docs/reports/horizon-2-h2-3-verified-analysis-templates.json`](../reports/horizon-2-h2-3-verified-analysis-templates.json).

#### H2.4 implementation status (complete 2026-08-26)

Narrative maps are now sealed project views rather than screenshot sequences.
Each ordered frame carries restorable camera, temporal cursor, layer visibility,
opacity, result/style digests, narrative text, accessible content-addressed
media references, and an optional dashboard bound to the same result. The
canonical view digest covers the complete semantic sequence.

Composition itself emits `RunWorkflow`. The Workbench API accepts only result
digests that achieved `verification_passed` in the current session; arbitrary
client-supplied digests receive HTTP 409. Local media paths, invalid camera or
opacity values, duplicate frame/layer identities, cross-result dashboards, and
post-seal mutations fail closed. Live verification composed a North Star view
with a second command/workflow identity and no screenshot copies.

Evidence is recorded in
[`docs/reports/horizon-2-h2-4-narrative-map-composer.json`](../reports/horizon-2-h2-4-narrative-map-composer.json).

#### Horizon 2 closeout (complete 2026-08-26)

H2.1–H2.4 are complete as recorded above. H2.5 portable publishing and H2.6
desktop bridge were delivered in Phase 14 M5, including redaction, offline
capsule verification, open transfer formats, and SDK-only bridge behavior.
Their implementation and exit evidence are recorded in
[`docs/reports/phase-14-m5-portable-publishing-desktop-bridge.json`](../reports/phase-14-m5-portable-publishing-desktop-bridge.json).

### Horizon 3 — Live spatial operations (12–24 months)

Extend proof-carrying analysis from bounded runs to continuously changing
spatial state.

| ID | Deliverable | Exit evidence |
| --- | --- | --- |
| `H3.1` | Live-feed adapters | Weather, hazard, mobility, sensor, and change feeds use cursor/watermark semantics, freshness policy, and immutable observation snapshots |
| `H3.2` | Incremental Workflow Graph | Nodes recompute only affected partitions while recording input windows, late data, retries, and replacement events |
| `H3.3` | Operational dashboards | Maps, charts, status, and alert history remain linked to one versioned view of the workflow result |
| `H3.4` | Verified alerting | Threshold and anomaly alerts include the triggering data window, policy, verifier result, and acknowledgement history |
| `H3.5` | Scenario comparison | Users branch assumptions, compare spatial outcomes, and merge reviewed changes through semantic diff and digest-bound approval |
| `H3.6` | City-scale 3D and 3D Tiles | Streamed terrain, point clouds, and building models share camera, selection, temporal, and provenance contracts with 2D layers |

**Gate:** No alert may be emitted from an LLM judgement alone. Continuous
execution must have bounded retention, backpressure, and replay before it can
be used for operational decisions.

#### H3.1 implementation status (complete 2026-08-26)

Weather, hazard, mobility, sensor, and detected-change feeds now share one
manifest-admitted HTTP JSON adapter. Every page is bounded by an exclusive
numeric cursor, event-time watermark, page/retention limits, explicit
evaluation time, maximum age, and allowed lateness. Accepted observations carry
CRS, geometry, values/units, provider revision, and a canonical immutable
snapshot digest.

Cursor and watermark regressions, stale or over-late observations, invalid
geometry, non-monotone sequences, oversized pages, unsupported media types,
and capability/manifest drift fail before commit. Cursor commit is the final
node of a `RunWorkflow` graph and occurs only after response validation and
snapshot sealing. The Workbench exposes the same boundary at
`POST /api/live/ingest`.

Evidence is recorded in
[`docs/reports/horizon-3-h3-1-live-feed-adapters.json`](../reports/horizon-3-h3-1-live-feed-adapters.json).

#### H3.2 implementation status (complete 2026-08-26)

The core now has a partition-aware incremental scheduler over the existing
Workflow Graph IR. A changed graph input invalidates only consuming nodes and
their downstream nodes for the named partition; committed outputs for every
other partition remain untouched. Decisions are emitted in topological order.

Each run receipts the exact cursor/watermark input window, append/late/
replacement semantics, immutable snapshot digest, per-node attempts, retries,
failures, committed output digests, replaced output digests, complete graph
digest, change digest, and post-run scheduler-state digest. Retry exhaustion
stops downstream execution and leaves the run explicitly incomplete.

Evidence is recorded in
[`docs/reports/horizon-3-h3-2-incremental-workflow-graph.json`](../reports/horizon-3-h3-2-incremental-workflow-graph.json).

#### H3.3 implementation status (complete 2026-08-26)

Operational maps, KPI/chart widgets, cursor/watermark freshness status,
incremental scheduler state, and alert history now form one sealed view version.
Every map layer shares the view result digest and carries a style digest; widget
IDs are unique; feed sources are checksum-verified. Updates link to the exact
previous view digest and require monotone version, cursor, watermark, and
append-only alert history.

Composition emits `RunWorkflow`, and its result is the canonical complete-view
digest. Regression or component tampering fails closed. The Workbench exposes
the same composition boundary at `POST /api/operational/views`.

Evidence is recorded in
[`docs/reports/horizon-3-h3-3-operational-dashboards.json`](../reports/horizon-3-h3-3-operational-dashboards.json).

#### H3.4 implementation status (complete 2026-08-26)

Verified alerts now use a closed deterministic rule set: numeric thresholds or
absolute z-scores against a digest-bound baseline. There is deliberately no
LLM-judgement rule. Every evaluation is a `RunWorkflow` over a checksum-verified
triggering source and records cursor/watermark window, immutable observation
digests, triggering result, policy bytes/digest, metric and unit, evaluation
value, verifier identity/checks, and evaluation digest.

Triggered alerts have a stable trigger identity and a separate current-record
digest. Human acknowledgements are appended through another `RunWorkflow`;
they preserve the trigger identity while updating the record digest. Stale
required-fresh windows, unit/field disagreement, invalid baseline parameters,
tampering, duplicate or time-regressing acknowledgements fail closed. The
Workbench exposes evaluation and acknowledgement APIs under `/api/alerts/`.

Evidence is recorded in
[`docs/reports/horizon-3-h3-4-verified-alerting.json`](../reports/horizon-3-h3-4-verified-alerting.json).

#### H3.5 implementation status (complete 2026-08-26)

Scenario work now has three explicit Command + Workflow operations: create a
branch from a base project, compare two branches sharing that base, and merge
an exact reviewed diff. Assumptions are typed and unit-bearing; spatial
outcomes retain metric units and values by stable area identity. Branch,
semantic diff, merged project, and merge commit each have canonical digests.

Comparison reports deterministically ordered assumption and per-area outcome
changes. Merge recomputes the diff and requires reviewer identity, RFC 3339
approval time, exact diff digest, and exact target branch digest. Cross-base
comparison, tampered branches, and stale approvals fail closed. Workbench APIs
are exposed at `/api/scenarios/{branches,compare,merge}`.

Evidence is recorded in
[`docs/reports/horizon-3-h3-5-scenario-comparison.json`](../reports/horizon-3-h3-5-scenario-comparison.json).

#### H3.6 implementation status (complete 2026-08-26)

City-scale rendering now plans COG terrain, COPC point clouds, OGC 3D Tiles
buildings, and PMTiles/COG 2D context through one shared spatial view state.
The state binds CRS, camera, viewport, feature selection, temporal cursor, and
verified source snapshots instead of creating separate 2D and 3D identities.

The renderer selects a deterministic screen-space-error hierarchy frontier and
enforces hard tile-count and transfer-byte budgets. Duplicate identities,
rootless or cyclic hierarchies, unknown CRS, unverified sources, and plan
tampering fail closed. The resulting canonical frame-plan digest is produced by
a `RunWorkflow` command and is available at `/api/city-scene/plan`.

Evidence is recorded in
[`docs/reports/horizon-3-h3-6-city-scale-3d.json`](../reports/horizon-3-h3-6-city-scale-3d.json).

#### Horizon 3 closeout (complete 2026-08-26)

All six live spatial operations milestones are implemented. Feed windows enter
an incremental graph, drive digest-bound operational views and deterministic
alerts, remain branchable for reviewed scenarios, and share selection and time
state with budgeted city-scale 2D/3D rendering.

### Horizon 4 — Governed ecosystem (24–36 months)

Make GeneGIS deployable as shared infrastructure without compromising its open
formats or verification model.

| ID | Deliverable | Exit evidence |
| --- | --- | --- |
| `H4.1` | Organization governance | Projects, roles, approvals, retention, and audit export are policy-driven and testable in desktop, browser, and server deployments |
| `H4.2` | Private federated catalogs | Organizations can combine local, cloud, and public catalogs while preserving access boundaries and source identity |
| `H4.3` | SDK v1 and plugin registry | Versioned Rust, TypeScript, Python, and WASM contracts ship with conformance suites, capability declarations, signatures, and revocation |
| `H4.4` | Deployment profiles | Local-first, managed cloud, and air-gapped profiles pass the same workflow replay and verification corpus |
| `H4.5` | Domain solution packs | Urban, environment, disaster, mobility, and infrastructure packs consist of templates and plugins rather than core forks |
| `H4.6` | Reproducible performance matrix | Published CPU/GPU, dataset, network, and concurrency profiles gate regression across supported deployment classes |

**Gate:** Billing and marketplace economics remain outside the core. A domain
pack cannot introduce proprietary project state or bypass adapter admission.

#### H4.1 implementation status (complete 2026-08-26)

Organization governance is now a runtime-neutral, versioned policy contract.
Active membership and roles grant explicit project capabilities; selected
actions require a threshold of distinct independent approvals bound to the
exact resource digest. Cross-organization access, self-approval, duplicate
approval, stale identities, and malformed timestamps fail closed.

Retention is evaluated as a non-destructive plan with legal holds and protected
record classes. Every authorization and approval transition appends a SHA-256
hash-chained audit event, and authorized audit export binds the complete chain
to the exact policy identity. All transitions execute through `RunWorkflow` and
are exposed by both Workbench (desktop/browser) and Server at
`/api/governance/execute`.

Evidence is recorded in
[`docs/reports/horizon-4-h4-1-organization-governance.json`](../reports/horizon-4-h4-1-organization-governance.json).

#### H4.2 implementation status (complete 2026-08-26)

Local, organization-private, and public STAC endpoints can now be combined only
after an exact access-policy admission workflow. Admission binds subject,
organization, and roles; inaccessible endpoints are represented by a count and
do not disclose their IDs or URLs. Private credentials remain secret-free host
environment references.

Each admitted endpoint retains a canonical configuration digest, and existing
federated STAC results continue to carry all origin endpoint IDs on every
deduplicated item and asset binding. Policy coverage gaps, cross-organization
access, endpoint mutation, and admission mutation fail closed. Workbench and
Server expose the shared workflow at `/api/catalogs/private/admit`.

Evidence is recorded in
[`docs/reports/horizon-4-h4-2-private-federated-catalogs.json`](../reports/horizon-4-h4-2-private-federated-catalogs.json).

#### H4.3 implementation status (complete 2026-08-26)

SDK v1 fixes a language-neutral plugin manifest and capability vocabulary at
API `1.0.0`. Rust is the reference implementation; TypeScript and Python
bindings, a JSON Schema, a shared conformance fixture, and a WIT component world
ship under `sdk/v1/`. The WASM host verifies the manifest and exact artifact
SHA-256 before compilation, after capability-policy intersection.

Registry publication requires a trusted Ed25519 signature over the canonical
manifest and prevents replacement of an existing plugin ID/version. Signed
revocation targets one exact release and prevents future resolution. Publish and
revoke mutations execute through `RunWorkflow`; Workbench and Server expose the
shared operation at `/api/plugins/registry/execute`. Marketplace billing remains
outside the core.

Evidence is recorded in
[`docs/reports/horizon-4-h4-3-sdk-v1-plugin-registry.json`](../reports/horizon-4-h4-3-sdk-v1-plugin-registry.json).

#### H4.4 implementation status (complete 2026-08-26)

Three versioned deployment profiles now share one verification contract:
local-first uses local identity and filesystem persistence; managed cloud uses
OIDC, object storage, and explicit outbound origins; air-gapped disables the
network and requires pinned offline plugin/catalog material. Every profile
requires Command + Workflow operations, offline capsule verification, build
identity, and the same open cloud-native formats.

The deployment conformance runner executes the north-star prompt independently
under all three profiles, seals three separate content-addressed capsules, and
verifies each offline. The corpus fails if workflow, result, capsule result, or
verification-graph digests differ. The accepted run used no external network in
any profile.

Evidence is recorded in
[`docs/reports/horizon-4-h4-4-deployment-profiles.json`](../reports/horizon-4-h4-4-deployment-profiles.json).

#### H4.5 implementation status (complete 2026-08-26)

Urban, environment, disaster, mobility, and infrastructure solution packs now
ship as portable manifests under `packs/`. A pack may contain only existing
reviewed Workflow template IDs, exact active SDK v1 plugin releases and their
required capabilities, and open GeneGIS project/workflow/capsule state formats.
The deny-unknown-fields contract has no representation for a core fork or a
proprietary project-state override.

Admission resolves every template from the reviewed palette and every plugin
from the signed, non-revoked registry before sealing the pack digest through a
`RunWorkflow` command. Workbench and Server expose the same operation at
`/api/solution-packs/admit`.

Evidence is recorded in
[`docs/reports/horizon-4-h4-5-domain-solution-packs.json`](../reports/horizon-4-h4-5-domain-solution-packs.json).

#### H4.6 implementation status (accepted with managed-cloud execution waiver)

Versioned local-first, managed-cloud, and air-gapped performance profiles now
cover CPU, GPU, dataset, network, and concurrency budgets over the pinned
Nagoya fixture and exact build-lock digest. Measurements carry fixture, build,
OS, CPU, optional GPU adapter, network profile, concurrency, observation time,
unit, and iteration evidence.

The evaluator produces `pass`, `fail`, or `pending`: a missing required GPU
measurement can never become green, while any measured regression fails the
matrix. Evaluation executes through `RunWorkflow` and is exposed at
`/api/performance-matrix/evaluate` in Workbench and Server.

The software gate and mutation coverage pass. Twenty release-mode local GPU
receipts share one executable, fixture, adapter, and backend identity; their
nearest-rank p95 first-frame and minimum-FPS aggregates meet the local-first
and air-gapped budgets. Complete Command + Workflow matrix receipts now pass
for those two deployment classes, including zero external requests for the
air-gapped profile. Managed-cloud execution is waived, and the local GTX 1660
Ti result is not represented as evidence for that hardware or network class.
Evidence and the waiver are recorded in
[`docs/reports/horizon-4-h4-6-performance-matrix.json`](../reports/horizon-4-h4-6-performance-matrix.json).

## Cross-cutting release gates

Every milestone above must satisfy all applicable gates:

1. All user-visible operations flow through Command + Workflow Graph.
2. CRS, units, source identity, license, freshness, and provenance are explicit.
3. Verification fails closed; LLM output is never its own verifier.
4. A deterministic offline fixture and at least one declared real-data path
   exercise the same contract.
5. Negative and mutation tests demonstrate zero false verified or attested
   results in the release corpus.
6. Cloud reads, memory, first frame, steady-state rendering, and workflow
   latency use pinned fixtures and hardware-aware budgets.
7. Published results expose attribution, limitations, digest, and capsule
   export without requiring GeneGIS to verify them.
8. Public contracts prefer GeoParquet, COG, COPC, PMTiles, STAC, OGC APIs,
   PROV, OpenLineage, and other open standards.

## Portfolio measures

The roadmap is succeeding when:

- the north-star workflow remains offline, deterministic, and green;
- all first-party interactive actions are replayable from Commands;
- all first-party connectors emit common source and I/O receipts;
- ten or more reusable workflow templates pass the same verification policy as
  hand-built workflows;
- 2D, 3D, temporal, dashboard, narrative, and published views reference one
  result identity rather than duplicating data;
- mutation suites continue to report zero false verified or attested results;
- desktop, browser, server, and air-gapped profiles can independently verify a
  shared capsule.

## Sequencing rules

- Complete Phase 12–13 before operational feeds or organization governance.
- Complete Phase 14 M1–M4 before narrative publishing depends on interactive
  3D, dashboards, or temporal state.
- Stabilize provider-neutral geocoding and service adapters before adding
  provider-specific convenience features.
- Prove incremental replay before alerts, and prove policy/redaction before
  public sharing.
- Stabilize SDK v1 before expanding the plugin registry or domain packs.
- Keep the core small; add product breadth through typed adapters, templates,
  plugins, and SDK contracts.

## Post-acceptance actions

1. Keep both waived execution packets available so either external validation
   can be reopened without weakening its evidence rules.
2. Prepare commit and push only when explicitly requested.
