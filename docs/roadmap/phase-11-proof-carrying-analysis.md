# Phase 11: Proof-Carrying Spatial Analysis

**Goal:** Prove that GeneGIS can prevent, explain, diff, and portably verify
spatial-analysis errors that modern agentic GIS products can plausibly produce.

**Status:** Complete on 2026-08-23. The optional human timing evaluation was
explicitly skipped by the user and is not reported as passed.

## Progress

- `P0-1` complete on 2026-08-23: GeoContract v0 Rust types, committed JSON
  Schema, compatibility truth table, workflow input/output integration, Nagoya
  contracts, fail-closed fixtures, and real CLI evidence.
- `P0-2` complete on 2026-08-23: versioned VerificationPolicy derives
  exploratory/replayable/verified/attested from evidence and fails closed.
- `P0-3` complete on 2026-08-23: the Nagoya executor emits a separate,
  content-addressed Verification Graph with verifier identity, independence,
  evidence inputs, dependencies, and tolerances; the receipt rejects any
  disagreement with policy-derived trust.
- `P0-4` complete on 2026-08-23: the open directory capsule binds nine
  content-addressed subjects and the CLI recomputes result/trust offline with
  an optional externally required policy.
- `P0-5` complete on 2026-08-23: 29 manifest-resynchronized mutations across
  contract, DAG, geometry, result, artifact, receipt, policy, and verification
  graph categories are caught (100% score; false verified/attested = 0).
- Gate A and the technical Gate C checks passed. Gate B's automated functional
  checks passed; its human timing metric was explicitly skipped by the user and
  remains `not_evaluated`, so no reviewer-speed claim is made.
- `P1-1` complete on 2026-08-23: `capsule diff` suppresses runtime-only noise,
  emits leaf-level classified JSON, and reports unclassified changes.
- `P1-4` complete on 2026-08-23: approval binds capsule, result, workflow,
  policy, verification graph, and optional semantic-diff digests; stale reuse
  is rejected.
- `P1-2` complete on 2026-08-23: the Trust Debugger has keyboard-navigable
  Claims, Contracts, Sources, Workflow, Artifacts, Failures, and Diff panes,
  safe source preview, failure-to-node focus, and stable non-TTY JSON.
- `P1-3` complete on 2026-08-23: integrity and policy failures remain
  inspectable through the same review model. The fixed timing corpus and runner
  are implemented; the user explicitly skipped human execution, so the timing
  metric remains unmeasured.
- `P1-5` complete on 2026-08-23: JSON, TUI, capsule, and deterministic HTML
  report share policy-derived trust and the same semantic digest identities.
- `P2-1` through `P2-5` complete on 2026-08-23: the verified export bundle
  covers PROV-JSON, externally validated Workflow Run RO-Crate 0.5,
  schema-validated OpenLineage, signed DSSE/in-toto, and a locally executing
  OGC API - Processes fixture.
- `P2-4` additionally measures 33 Native/DuckDB/GDAL valid and invalid cases,
  including preserved and dropped multipart shells.
- `P2-6` complete as an explicitly non-leaderboard GeoBenchX-derived Nagoya
  adapter slice with strict artifact scoring, legal attribution, runner
  identity, passing and failing fixtures, and zero false accepts.

**Strategy:** Implement the smallest vertical slice of [RFC 0003](/home/sasaki/workspace/GeneGIS/docs/rfcs/0003-proof-carrying-spatial-analysis.md)
on the Nagoya workflow before adding more engines, prompts, or desktop editing.

## Exit demonstration

Given a verified Nagoya population-density capsule and a proposed annual
update, a reviewer can:

1. inspect the resolved data meaning and assumptions;
2. see a semantic diff of sources, reference year, workflow, and results;
3. run required independent checks;
4. reject seeded CRS, unit, join, geometry, time, and checksum faults;
5. seal a portable result capsule;
6. verify it offline without an LLM or GeneGIS server.

## Work packages

### P0 — Make the claim machine-checkable

Target: weeks 1–3. Do not begin P1 until all P0 gates pass.

| ID | Deliverable | Exit gate |
| --- | --- | --- |
| `P0-1` | `GeoContract v0` Rust types and JSON Schema for spatial, measure, temporal, coverage, source, license, and uncertainty semantics | Round-trip/schema tests and negative fixtures for unknown CRS/unit/time, incompatible measures, duplicate keys, and coverage gaps |
| `P0-2` | Versioned `VerificationPolicy` and derived `exploratory`, `replayable`, `verified`, `attested` states | Trust derives only from policy/evidence; missing or failed requirements fail closed |
| `P0-3` | Separate verification graph and verifier independence metadata | Every Nagoya release claim identifies implementation, input, tolerance, and independence class |
| `P0-4` | Open capsule layout with canonical manifests and content-addressed subjects | `genegis capsule verify PATH --policy POLICY` works offline in a clean temporary directory |
| `P0-5` | Mutation harness | At least 20 source, contract, DAG, geometry, result, receipt, and policy mutations; score ≥95%, false `verified` = 0 |

### P1 — Make review materially better than logs

Target: weeks 4–7.

| ID | Deliverable | Exit gate |
| --- | --- | --- |
| `P1-1` | `genegis diff OLD NEW` semantic-diff engine | Classifies every source, contract, graph, result, and check change in an annual-update fixture |
| `P1-2` | TUI trust debugger | Claim/contract/DAG/check/artifact panes, keyboard navigation, failure-to-node focus, source opening, and diff mode |
| `P1-3` | Structured actionable failure explanations | Median reviewer time-to-root-cause ≤2 minutes across seeded tasks; no LLM required |
| `P1-4` | Approval object bound to workflow, source, policy, and diff digests | Any semantic mutation invalidates approval; replay cannot reuse stale approval |
| `P1-5` | Human report plus stable JSON | TUI, CLI, HTML, and capsule show identical trust state and digest identities |

The TUI is intentionally not a layer editor. Its initial commands are:

```text
genegis plan "名古屋市の人口密度を表示"
genegis review <run-or-capsule>
genegis diff <old-capsule> <new-capsule>
genegis verify <capsule> --policy <policy>
genegis replay <capsule>
```

### P2 — Prove openness and external utility

Target: weeks 8–12.

| ID | Deliverable | Exit gate |
| --- | --- | --- |
| `P2-1` | W3C PROV / Workflow Run RO-Crate mapping | Export validates against the selected profile; GeneGIS fields are namespaced and documented |
| `P2-2` | Optional OpenLineage events | Round-trip preserves run, job, dataset, source, and artifact identities |
| `P2-3` | In-toto-compatible signed-analysis attestation experiment | Offline integrity verification works; docs distinguish integrity from truth |
| `P2-4` | DuckDB/GDAL/native equivalence suite | At least 20 valid/invalid cases cover CRS, units, topology, joins, nulls, ordering, and tolerances |
| `P2-5` | OGC API - Processes interoperability spike | One process executes through a local standard fixture; semantic/evidence gaps are explicit |
| `P2-6` | External GIS-agent benchmark adapter | Publish strict artifact/tolerance results for a legally reusable subset, including failures and runner identity |

## Required fault corpus

- EPSG identifier present but axis/coordinate interpretation wrong;
- geographic degrees treated as metres;
- persons confused with thousands of persons;
- population and boundary reference periods incompatible;
- duplicate, missing, or renamed ward join keys;
- one multipart shell dropped or a hole filled;
- source bytes changed behind a stable URL;
- checksum or license missing;
- DAG edge/parameter altered after approval;
- backend version changes numeric behavior;
- renderer hides a feature present in the numeric result;
- verifier accidentally shares the executor implementation;
- receipt or artifact changed after execution;
- failed check downgraded to a warning during export.

Every fixture states expected trust level, failing predicate, affected node, and
safe remediation. Testing only an error string is insufficient.

## Measurement report

Create `/home/sasaki/workspace/GeneGIS/docs/reports/phase-11-acceptance.json`
containing:

- commit and binary/container identities;
- schema, policy, adapter, and fixture versions;
- pass/fail/not-measured for every RFC 0003 metric;
- mutation survivors and disposition;
- cross-engine numeric deltas;
- TUI review-task completion times;
- benchmark licenses, exact inputs/outputs, and scoring method.

Record performance and peak memory, but optimize only after correctness and
review gates pass.

## Go/no-go gates

### Gate A — after P0

Proceed only when no negative fixture becomes `verified`, offline capsule
verification works, and mutation score is at least 95%. Otherwise strengthen
contracts before building the TUI.

### Gate B — after P1

Proceed only when reviewers diagnose seeded faults without raw JSON and approval
invalidation is exact. Otherwise do not add agent autonomy.

### Gate C — after P2

Publish the differentiation claim only when external exports validate, two
execution paths agree within policy, and benchmarks score actual artifacts
rather than prose or model judgment.

## Explicit deferrals

- QGIS-style layer editing and Processing Toolbox parity;
- broad adapter/plugin marketplace;
- unrestricted agent-generated Python or shell execution;
- multi-agent planning as a product claim;
- GPU work not required by the acceptance fixture;
- enterprise billing and multi-cloud dashboards;
- proprietary capsule or provenance formats.

## First ten development items

1. Write `GeoContract v0` schema and compatibility truth table.
2. Add `VerificationPolicy` and computed trust-state machine.
3. Extract Nagoya checks into an explicit verification graph.
4. Define the open capsule layout and canonical manifest.
5. Implement offline `capsule verify`.
6. Build the 20-case mutation harness.
7. Implement semantic-diff types and CLI JSON.
8. Build the minimal TUI for review, diff, and failure navigation.
9. Bind review approval to every semantic digest.
10. Produce the Gate A acceptance report before expanding scope.
