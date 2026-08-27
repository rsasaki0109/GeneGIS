# RFC 0002: Evidence-First Intent-to-Verified Workbench and OSS Interoperability

- **Status:** Proposed
- **Authors:** GeneGIS core team
- **Created:** 2026-08-22
- **Scope:** Product positioning and cross-cutting architecture after the Phase 10 federated-catalog work

## Summary

GeneGIS is an **evidence-first intent-to-verified geospatial workbench**. It
compiles a human or agent request into a typed `Command` and `GeoWorkflow` DAG,
executes the graph through suitable open-source engines, and returns a map
together with the evidence needed to inspect, verify, and replay the result.

The differentiation is the open execution contract and trust surface between
intent, data, engines, and product UX. It is not a claim that GeneGIS has a
better geometry algorithm, a new file format, or a more complete desktop GIS
than the existing OSS ecosystem. Existing projects remain the default backends
where they are the best tool for the job.

The north-star acceptance case is still **「名古屋市の人口密度を表示」**. A
successful result must make the following visible and machine-readable:

```text
Intent
  -> Catalog and source selection
  -> Typed Command + GeoWorkflow DAG
  -> Policy admission and backend execution
  -> Independent verification
  -> Map + explanation + provenance receipt
  -> Stable replay
```

This RFC complements, and does not replace, [RFC 0001: Master Architecture](/home/sasaki/workspace/GeneGIS/docs/rfcs/0001-master-architecture.md),
[ADR 0007: CRS, Units, and Source/Provenance Contract](/home/sasaki/workspace/GeneGIS/docs/adr/0007-crs-units-source-provenance-contract.md),
and [ADR 0008: Explicit Workflow DAG and Stable Workflow Digest](/home/sasaki/workspace/GeneGIS/docs/adr/0008-workflow-dag-and-stable-digest.md).

## Context: what the OSS ecosystem already does well

This is a capability and product-fit comparison, not a feature-count ranking.
The projects below are mature, valuable, and complementary. Their official
documentation describes different centers of gravity; the GeneGIS opportunity
is the cross-engine contract and user experience that joins those centers of
gravity into one verifiable run.

| OSS project | Documented strength | GeneGIS fit gap and response |
| --- | --- | --- |
| **QGIS** | The Processing framework includes providers, a graphical model designer, batch execution, history, and logs. The model designer can validate models and export them as scripts; the history records execution parameters for later re-use ([Processing framework](https://docs.qgis.org/3.44/en/docs/user_manual/processing/index.html), [model designer](https://docs.qgis.org/3.44/en/docs/user_manual/processing/modeler.html), [history manager](https://docs.qgis.org/3.44/en/docs/user_manual/processing/history.html)). | QGIS already supports modeling and reproducible *calls*. GeneGIS addresses a narrower cross-tool gap: a typed CRS/unit/source contract, content identity, independent verification, and one append-only receipt spanning UI, AI, CLI, and remote assets. GeneGIS must not claim that QGIS lacks scripting or history, and will interoperate with QGIS rather than imitate its desktop UI. |
| **GRASS GIS** | GRASS describes itself as a processing engine for advanced analysis and visualization with command-line, Python, Jupyter, and GUI interfaces; its temporal framework covers space-time raster and vector data ([GRASS documentation](https://grass.osgeo.org/grass-stable/manuals/), [temporal framework](https://grass.osgeo.org/grass-stable/manuals/temporalintro.html)). | Its module depth is an asset, but selecting modules from intent, applying network/native-code policy, and stitching evidence across heterogeneous modules are product responsibilities GeneGIS can add. GRASS remains an optional specialized worker, not a competitor to reimplement. |
| **GDAL** | GDAL provides broad raster/vector translation and inspection utilities, including strict conversion behavior and geometry checks ([`gdal_translate`](https://gdal.org/en/stable/programs/gdal_translate.html), [vector commands](https://gdal.org/en/stable/programs/gdal_vector.html)). | GDAL is a data-plane library and CLI, not a product-level intent planner or verification receipt. GeneGIS should wrap GDAL operations as typed nodes and preserve driver, version, options, warnings, and output identity. |
| **PostGIS** | PostGIS extends PostgreSQL with spatial types, indexes, functions, coordinate systems, transactions, access control, and normal database tooling ([PostGIS documentation](https://postgis.net/docs/)). | PostGIS is the right system for many authoritative and transactional datasets. GeneGIS adds portable graph identity, execution policy, source snapshots, and cross-backend evidence around read-only or explicitly approved queries; it does not replace the database. |
| **GeoServer** | GeoServer publishes OGC services such as WMS, WFS, WCS, and OGC API Features, with a service/data-directory configuration model ([services](https://docs.geoserver.org/latest/en/user/services/), [data directory](https://docs.geoserver.org/latest/en/user/datadirectory/)). | Publishing and serving are not the same as intent-to-analysis. GeneGIS can consume OGC APIs and publish verified outputs through GeoServer-compatible standards while keeping the workflow and evidence outside server-specific configuration. |
| **DuckDB Spatial** | DuckDB's Spatial extension provides in-process geospatial processing and can use GDAL-backed import/export ([Spatial extension](https://duckdb.org/docs/current/core_extensions/spatial/overview), [GDAL integration](https://duckdb.org/docs/current/core_extensions/spatial/gdal)). | DuckDB is an excellent local analytical/verifier backend. GeneGIS supplies the graph, source contract, admission policy, independent checks, and product surfaces around it; backend extension/build identity must be recorded because bundled dependencies can differ from a system installation. |

The same pattern appears in the standards layer. STAC deliberately provides a
flexible, extensible catalog/item model ([STAC specification overview](https://stacspec.org/en/about/stac-spec/));
GeoParquet defines geospatial metadata such as the geometry column and CRS
([GeoParquet format specification](https://github.com/opengeospatial/geoparquet/blob/main/format-specs/geoparquet.md));
and OGC API provides predictable discovery and feature access ([OGC API family](https://ogcapi.ogc.org/),
[OGC API - Features Part 1](https://docs.ogc.org/is/17-069r3/17-069r3.html)).
Those standards improve exchange, but they do not by themselves assert that a
population field is semantically compatible with an area field, that a remote
asset is the bytes that were verified, or that a rendered map can be replayed
from the same inputs. GeneGIS adds those execution-time contracts while
preserving the standard representations.

### The actual problem GeneGIS is solving

The problem is not that OSS GIS lacks algorithms. It is that a user who moves
from discovery to analysis to publication often crosses several independently
configured tools. At those boundaries, the following facts can be lost or left
to convention:

- which source bytes and source version were used;
- whether CRS and axis units were known and compatible with the operation;
- whether a value such as population is in persons, thousands of persons, or a
  different reference year;
- which backend, driver, SQL, module, or rendering parameters ran;
- which checks actually passed, and whether a failed check blocked publication;
- whether a later replay is equivalent or merely looks similar.

This is a design inference from the documented capabilities above, not a claim
that any individual OSS project cannot be extended to cover one of these
concerns. GeneGIS is valuable only if it makes the cross-tool contract cheap,
visible, and testable.

## Product positioning

### Positioning statement

> **GeneGIS is the open evidence layer for spatial intent:** a workbench that
> turns a request into a typed, policy-checked, replayable workflow and shows
> the map, the method, the sources, and the verification evidence together.

The primary user promise is not “the agent knows GIS.” It is:

1. **Understandable:** a person can inspect the proposed operations and
   assumptions before execution.
2. **Correctly typed:** spatial reference, coordinate units, value units, schema,
   coverage, and source identity are admitted or rejected at boundaries.
3. **Evidence-bearing:** a map is accompanied by checks, source links, backend
   identity, and a receipt rather than only a prose answer.
4. **Reproducible:** the stable workflow digest and source snapshot determine
   what “the same run” means.
5. **Open:** the graph calls open standards and existing OSS engines; a user can
   inspect or replace a worker without changing the product-level contract.

The product category is therefore closer to a **geospatial execution and
evidence control plane with a workbench UX** than to a new monolithic desktop
GIS. A map remains the important output, but the unit of value is a verified
run, not an unconnected layer or screenshot.

## Architecture bets

These are the six bets that must remain true for the positioning to be real.
Each bet has an explicit failure mode; a feature that does not strengthen one
of these bets is not automatically strategic.

### B1. Command + Workflow Graph is the product primitive

Every UI action, CLI invocation, agent proposal, and plugin operation emits the
same `Command` path and resolves to a typed `GeoWorkflow` DAG. Nodes declare
inputs, outputs, dependencies, parameters, preconditions, and postconditions.
The graph has a stable digest independent of run UUIDs and event timestamps.

This extends [ADR 0008](/home/sasaki/workspace/GeneGIS/docs/adr/0008-workflow-dag-and-stable-digest):
an ordered list of UI steps or a prompt transcript is not sufficient as the
portable execution identity. A graph that cannot be validated before execution
cannot be the trust boundary.

### B2. Evidence is an execution gate, not an afterthought

Every spatial input and derived output carries structured CRS, coordinate-axis
unit, value unit, source snapshot, and lineage. The execution receipt records
the workflow/command identity, source URI or catalog identity, expected and
observed checksum status, license, source version, retrieval event, parameters,
backend/build identity, verifier, and check results.

The result is exportable only when required checks pass. A warning may be
visible, but a missing or unknown CRS, unit mismatch, unverified external
source, or failed verification cannot silently become a “verified” map. The
contract is defined by [ADR 0007](/home/sasaki/workspace/GeneGIS/docs/adr/0007-crs-units-source-provenance-contract.md)
and should be shared by native and adapter-backed nodes.

### B3. AI compiles intent; deterministic execution remains authoritative

An LLM may propose a place, dataset, operation, or workflow revision. It may
not self-grade the result or bypass graph validation, source policy, or the
verification gate. The offline rule planner remains a supported path for the
north star, and `--plan-only` remains a first-class review action.

Human approval is required for capabilities classified as network access,
native code, data mutation, or external publication unless an explicit policy
grants that capability. This keeps the agent useful without making model output
the source of truth.

### B4. Cloud-native data is lazy, standard, and policy controlled

STAC, OGC API, GeoParquet, COG, COPC, PMTiles, GeoJSON, and WKB/Arrow are
interchange and access surfaces, not proprietary GeneGIS formats. Remote reads
use metadata and range requests where the backend supports them; local caches
are content-addressed and disposable. Host allowlists, redirect rules,
timeouts, response-size limits, credentials redaction, and explicit fallback
behavior are part of the workflow receipt.

Cloud support is not defined by accepting an `https://` URI. A run must say
which bytes it read, why those bytes satisfied the contract, and whether a
full-download fallback was used. Unsupported range or metadata semantics are a
visible capability failure, not a silent performance downgrade.

### B5. OSS engines are typed workers behind an adapter contract

GeneGIS will build a small orchestration core and grow capability through
adapters and plugins. GDAL, DuckDB Spatial, PostGIS, GRASS, QGIS Processing,
and GeoServer are integration targets at the appropriate boundary. Native
GeneGIS implementations are preferred only when they provide a needed contract,
portability, or performance property that an existing worker cannot expose.

The adapter contract is specified in the next section. A worker that cannot
declare its inputs, outputs, semantics, side effects, and evidence is an
unverified worker; it may be used for exploration only and cannot silently
produce a verified release artifact.

### B6. Trust is a product surface, not a log file

The primary result view presents the map and a concise “why this result is
trusted” panel: source cards, CRS and units, workflow graph, verification
status, warnings, backend identity, and replay/diff controls. The same identity
and status are available in CLI JSON, the browser workbench, desktop, and API
responses.

The UX should make a failed check actionable: show the failing contract, the
affected node and source, the safe next action, and whether a human decision is
needed. Hiding evidence behind debug logs would reproduce the integration gap
that this RFC is intended to solve.

## OSS interoperability policy

### Adapter manifest

Each backend adapter or plugin must publish a machine-readable manifest with at
least:

| Field | Requirement |
| --- | --- |
| `operation_id` and version | Stable operation identity and semantic version |
| Input/output schema | Types, geometry kinds, nullable fields, and cardinality |
| CRS and unit contract | Accepted CRS/axis units, produced CRS/units, and transformations |
| Preconditions/postconditions | Validity, coverage, required columns, and result invariants |
| Side effects and capabilities | Read/write/network/native-code requirements and policy class |
| Backend identity | Engine name, version, driver/module, build or container digest |
| Determinism | Declared deterministic behavior, ordering rules, random seeds, and limits |
| Evidence hooks | Parameters, logs, warnings, source references, output checksums, and verifier hooks |
| License and attribution | Runtime and output obligations, preserved in the receipt |

The manifest is part of the workflow digest when it affects semantics. A backend
version or adapter change that could alter values must invalidate a cached
verified result or require an explicit compatibility declaration.

### Initial backend mapping

| Backend | Initial GeneGIS role | Required evidence |
| --- | --- | --- |
| GDAL | Format inspection, conversion, reprojection, geometry checks, and controlled raster/vector IO | Driver, version, creation options, warnings, input/output checksums |
| DuckDB Spatial | Local SQL, joins, aggregations, and independent density verification | SQL, extension/build identity, schema, row counts, query result digest |
| PostGIS | Read-only enterprise queries and approved materialization/export | Server/database identity, transaction/isolation context, SQL, query plan or plan digest, output snapshot |
| GRASS | Specialized raster/vector/temporal algorithms in a sandbox | Module, flags, region/mapset, environment, version, generated artifacts |
| QGIS Processing | Import/export of compatible models or explicitly selected algorithms | Provider, algorithm ID/version, parameters, model file digest, logs; no assumption that a project file is a full source snapshot |
| GeoServer / OGC API | Discovery, feature/map publication, and consumption of published resources | Service URL, capabilities/conformance response, layer/collection identity, request parameters, retrieval checksum |

Adapters must preserve backend semantics rather than flattening every operation
into a generic “run script” node. When semantics cannot be represented, the
workflow records an explicit `opaque` boundary and lowers the trust level.
There is no implicit fork of GDAL, GRASS, PostGIS, QGIS, GeoServer, or DuckDB
inside the GeneGIS core.

### Interchange rules

1. Prefer open, inspectable standards at boundaries: GeoParquet, COG, COPC,
   PMTiles, STAC, OGC API, GeoJSON, WKB, and Arrow-compatible schemas.
2. Preserve source metadata and provenance when converting. A format conversion
   is a graph node with an input snapshot, options, output checksum, and
   license/attribution propagation—not an invisible cache step.
3. Keep backend execution optional where feasible. The offline Nagoya path must
   work without an LLM, network, PostGIS server, or QGIS installation.
4. Pin or record backend versions and container/build digests for verified runs;
   do not describe a result as reproducible when only a mutable “latest” worker
   is known.
5. Maintain conformance fixtures for each adapter, including valid and invalid
   CRS, units, schema, coverage, license, checksum, and network-policy cases.

## Non-goals and false differentiation

### Explicit non-goals

- QGIS Processing, plugin, or desktop UI parity; the existing [Phase 1
  non-goals](/home/sasaki/workspace/GeneGIS/docs/roadmap/phase-1-mvp.md) remain
  in force.
- Replacing PostGIS as an authoritative transactional database, GDAL as a
  translator, GRASS as an analysis engine, GeoServer as a publisher, or DuckDB
  as an embedded analytical engine.
- A new proprietary geometry, catalog, tile, or workflow file format when an
  open standard is adequate.
- Fully autonomous arbitrary shell execution, unrestricted network access,
  silent data mutation, or unreviewed external publication.
- A marketplace before the adapter manifest, SDK, capability policy, and
  conformance fixtures are stable.
- Treating a screenshot, chat transcript, model confidence score, or generated
  SQL as a provenance record.

### False differentiation we will not market as a moat

Natural-language input, a generic map canvas, support for cloud formats, a GPU
renderer, and an LLM wrapper are individually useful but reproducible by other
products. A list of format names or a benchmark with no workload and data
contract is not differentiation. GeneGIS earns differentiation only when those
features are tied to a typed graph, source identity, independent checks, a
fail-closed policy, and a replayable product artifact.

We also will not describe OSS GIS as “broken” merely because it has a different
center of gravity. The fair claim is that GeneGIS offers an opinionated
cross-engine evidence contract and UX for users who need intent, cloud access,
verification, and replay in one run.

## North-star acceptance metrics

These are acceptance targets for the canonical Nagoya fixture, not claims that
the current implementation already satisfies every target. Correctness and
rejection metrics are hard gates. Timing metrics must report the test runner,
dataset size, backend versions, and network fixture; they are not promises for
an arbitrary internet connection.

The reference input is 16 Nagoya wards, 2020 census population, and an
N03-derived boundary snapshot. The source manifest must link the [MLIT N03
administrative-boundary source](https://nlftp.mlit.go.jp/ksj/gml/datalist/KsjTmplt-N03.html)
and the authoritative [名古屋市 令和2年国勢調査確定値 page](https://www.city.nagoya.jp/shisei/toukei/1003703/1003773/1003809/1034253/1003818.html)
plus its [official Excel table](https://www.city.nagoya.jp/_res/projects/default_project/_page_/001/003/818/toukeihyo.xlsx),
record the exact snapshot checksums, and state any geometry normalization.
The bundled immutable manifest is
`/home/sasaki/workspace/GeneGIS/examples/nagoya-population-density/data/nagoya-source-manifest-2020.json`;
the independent area/population oracle is
`/home/sasaki/workspace/GeneGIS/examples/nagoya-population-density/data/nagoya-oracle-2020.json`.
The frozen boundary SHA-256 is
`sha256:d0f8958813fe28e9428169ca7c638a0ea3b3ed7ae526750156d3f94e1308d30e`,
and the population asset SHA-256 is
`sha256:176aa7996622d4ea3339abc6143591245ae2142177a46501b2fd0c4199f2b54d`.

| Dimension | Numeric acceptance target | Evidence / test shape |
| --- | --- | --- |
| **Accuracy** | **16/16** wards joined exactly once; population total delta from the input is **0 persons**; **100%** of geometries pass validity checks; **100%** of spatial fields have a known CRS and declared units; area and density relative error is **≤ 0.5% per ward** against an independently implemented geodesic/equal-area oracle; density is explicitly `persons/km²`; degree-coordinate scaling is used **0 times** in the canonical path. | Offline fixture test plus independent oracle. The receipt records the area method, CRS transformation, source versions, and unit conversions. Any unknown CRS, missing unit, duplicate join, or mismatched population year fails the run. |
| **Reproducibility** | **100/100** clean offline replays with the same source snapshot and workflow produce the same stable workflow digest, canonical result digest, and receipt digest after runtime UUIDs and event timestamps are normalized; **3/3** mutation classes (parameter, graph edge, source checksum) change identity and invalidate the prior verified cache. | Run the rule planner with LLM and network disabled, clear the disposable cache between runs, and compare canonical JSON. Retrieval time and runtime UUID are event fields, not replay identity. |
| **Provenance** | **100% (N/N)** of output artifacts and executed nodes (baseline canonical graph: **14/14**) link to command ID, workflow digest, source snapshot, CRS, coordinate/value units, operation parameters, backend/build identity, verifier, and check results; **0/100** receipts contain a credential or secret; schema validation passes **100/100** times. | JSON-schema validation, lineage traversal, secret scan, and receipt round-trip tests. Missing or unknown checksum status is never represented as verified for an external URI. |
| **Cloud access** | In a local HTTP range harness with STAC plus a GeoParquet object split into at least four row groups, **20/20** runs discover and execute the selected asset through an allowlisted endpoint; **100%** of remote data reads use `Range` or an explicitly recorded capability fallback; selected-row-group runs read **≤ 50%** of the object bytes and **≤ 8 MiB per response**; p95 end-to-end time is **≤ 10 seconds** on the documented CI harness. | Server-side request log and byte counter are the oracle. A non-range server is not silently treated as cloud-optimized; the receipt states the fallback and trust level. Network timing outside the harness is advisory. |
| **Trust** | **8/8** negative fixtures fail closed (unknown/mismatched CRS, unit mismatch, missing source identity, checksum mismatch, schema/coverage mismatch, unallowlisted host, rejected redirect, failed verification); **0/20** failed-verification runs export a verified map; approval-digest mutation permits **0/20** executions; offline rule-planner mode succeeds **100/100** times with no LLM. | Negative integration matrix across CLI, API, and workbench execution. The error must identify the contract, node, and safe remediation without exposing credentials. |
| **Product UX** | On the documented local runner, **p95 ≤ 10 seconds** from the north-star prompt to a verified map after dependencies are built; **≤ 2 user interactions** from the result view to source, CRS/unit, verification details, and workflow graph; **100% (20/20)** parity runs expose the same workflow digest and verification status in CLI JSON, browser, desktop, and API output; plan-only preview is **p95 ≤ 500 ms** for the local rule planner. | Browser/desktop/CLI smoke tests use the same fixture and compare machine-readable identity fields. The UI must expose a failing check and next action, not only a generic error banner. |

An acceptance report must label each target as `pass`, `fail`, or `not measured`,
include the absolute fixture and report paths, and identify the exact binary or
container digest. A green unit test that does not exercise the corresponding
boundary is not evidence for the target.

## Staged roadmap

The existing phase documents record completed platform slices through the
federated catalog; this roadmap adds a product differentiation track without
rewriting that history. Time windows are relative to RFC acceptance so that
they remain useful if external dependencies move.

| Stage | Window | Deliverable and exit gate |
| --- | --- | --- |
| **A — Contract hardening** | 0–6 weeks | Freeze the canonical Nagoya source manifest and independent area oracle; define receipt/schema versions, adapter manifest, error taxonomy, and all negative fixtures. Exit when the acceptance test harness can report every metric as pass/fail/not-measured. |
| **B — Trustable offline MVP** | 6–12 weeks | Complete the Command + DAG execution boundary, stable replay, typed CRS/unit/source checks, verification gate, and CLI/workbench/API identity parity. Exit when accuracy, reproducibility, provenance, and offline trust targets pass without LLM or network. |
| **C — Interop and cloud execution** | 12–20 weeks | Ship the first GDAL and DuckDB adapters, read-only PostGIS and GRASS integration spikes, STAC/GeoParquet range harness, policy receipts, and standard-format round trips. Exit when cloud-access and adapter conformance targets pass and unsupported capabilities fail visibly. |
| **D — Evidence-first workbench UX** | 20–32 weeks | Add source cards, graph/node inspection, verification explanations, replay/diff, backend identity, human gates, and exportable acceptance reports. Exit when product-UX targets pass across browser, desktop, CLI, and API. |
| **E — Team and ecosystem scale** | 32+ weeks | Add signed receipts or attestations, server-backed policy and review, collaboration over workflow/provenance metadata, stable adapter SDK, compatibility matrix, and community conformance fixtures. Exit only when the trust model and license/attribution propagation remain inspectable at scale. |

Stage C builds on [Phase 10: Federated Catalog Search](/home/sasaki/workspace/GeneGIS/docs/roadmap/phase-10-federated-catalog.md);
the offline north-star invariant from [Phase 6: Autonomous GIS Platform](/home/sasaki/workspace/GeneGIS/docs/roadmap/phase-6-autonomous.md)
and [Phase 9: External STAC & GeoParquet Workflows](/home/sasaki/workspace/GeneGIS/docs/roadmap/phase-9-external-data.md)
must remain true at every stage.

## Risks and mitigations

| Risk | Failure mode | Mitigation and trigger |
| --- | --- | --- |
| Scope expands into QGIS parity | Core becomes a second desktop GIS and loses its trust focus | Keep the non-goal explicit; accept a feature only if it strengthens graph, evidence, policy, cloud execution, or trust UX. Review every release against the six bets. |
| Adapter semantics are too lossy | A generic script wrapper claims verification without knowing what ran | Require manifests, typed pre/postconditions, backend identity, and conformance fixtures. Lower trust to `opaque` or block release when a field cannot be captured. |
| CRS/units remain nominal strings | A plausible density map is numerically wrong | Use typed CRS registry and unit contracts, independent oracle, negative fixtures, and zero-tolerance for silent degree scaling in the north-star path. |
| LLM or agent overreach | Hallucinated source, SQL, command, or network write becomes an official result | Keep deterministic planner/verifier authoritative; apply capability policy and human gates; never use model self-grading as verification. |
| Backend/version drift | Replay changes after an engine or driver upgrade | Include backend/build/container digests, schema/receipt versions, compatibility declarations, and invalidation rules in the stable identity. |
| Cloud access is flaky or expensive | Range assumptions fail, credentials leak, or a “cloud” run downloads everything | Use allowlists, redirect/size/time limits, local range fixtures, server-side byte assertions, secret scans, and visible fallback trust levels. |
| GPU claims distract from correctness | A fast renderer hides a wrong or unverifiable value | Make correctness/provenance gates independent of rendering; publish benchmarks only with dataset, workload, and acceptance contract. |
| License/attribution mismatch | An adapter or generated output violates OSS obligations | Record runtime and output obligations in manifests and receipts; review redistribution and service terms before bundling or shipping an adapter. |
| Evidence UX is too complex | Users ignore receipts and return to screenshots or chat answers | Progressive disclosure: map first, then one-click source/units/checks, with machine-readable detail always available. Measure the interaction targets above. |

## Immediate implementation queue

After this RFC is accepted, the smallest sequence that tests the positioning is:

1. Add a canonical source manifest and independent area/density oracle for the
   16-ward fixture.
2. Make the execution receipt schema and required-field validator explicit;
   connect every canonical DAG node to lineage and backend identity.
3. Add the adapter manifest and a DuckDB/GDAL conformance harness before adding
   more engines.
4. Turn the cloud-access table into a local HTTP range test with byte/request
   assertions and negative network-policy cases.
5. Add CLI/browser/desktop parity checks for workflow digest, verification
   status, source cards, and replay.
6. Publish the acceptance report as a versioned, reproducible artifact and use
   it as the release gate for the north-star demo.

## Decision record

If this RFC is accepted, GeneGIS product and architecture reviews should use
the six bets and the acceptance table as the default test for a proposed
feature. A proposal that adds an engine, format, agent tool, or UI surface must
state which bet it strengthens, which receipt fields it adds, which trust level
it receives, and how it preserves the offline Nagoya path. A proposal that
cannot answer those questions is deferred or explicitly labeled exploratory.
