# RFC 0003: Proof-Carrying Spatial Analysis

- **Status:** Proposed
- **Authors:** GeneGIS core team
- **Created:** 2026-08-23
- **Scope:** Product differentiation after the evidence-first Nagoya milestone

## Summary

GeneGIS will differentiate as an open **proof-carrying spatial analysis
workbench**: every releasable map, table, or dataset carries a machine-checkable
case for why it is fit for the stated question.

Natural-language GIS, workflow graphs, MCP tools, cloud execution, lineage, and
agent traceability are no longer defensible categories by themselves. GeneGIS
makes a narrower and stronger promise:

> A result is not merely generated. Its data meaning is contracted, execution
> is deterministic, important claims are independently checked, and evidence
> can be reviewed, diffed, exported, and verified outside the producing system.

The product primitive remains `Command + GeoWorkflow`. The new product unit is
the **verified result capsule**: open artifacts plus source snapshots, semantic
contracts, workflow identity, assumptions, checks, backend identity, and
attestations. This sharpens [RFC 0002](/home/sasaki/workspace/GeneGIS/docs/rfcs/0002-evidence-first-oss-interoperability.md).

## Research findings

The comparison was refreshed on 2026-08-23 using official documentation and
primary research. It compares documented centers of gravity, not every feature
obtainable through extensions.

| System | Documented center of gravity | Consequence for GeneGIS |
| --- | --- | --- |
| **QGIS Processing** | Algorithms, third-party providers, graphical models, validation, history, logs, and repeatable calls ([framework](https://docs.qgis.org/3.44/en/docs/user_manual/processing/intro.html), [modeler](https://docs.qgis.org/3.44/en/docs/user_manual/processing/modeler.html)) | A graph, history, or plugin catalog is not a moat. |
| **CARTO Agentic GIS** | Natural-language agents, deterministic workflows as MCP tools, CLI/API control, warehouse governance, and traceability ([AI Agents](https://docs.carto.com/carto-user-manual/ai-agents), [CARTO for Agents](https://carto.com/blog/introducing-carto-for-agents-gis-for-the-agentic-enterprise/)) | AI-native, MCP, workflow automation, and traceability are competitive requirements. |
| **Wherobots** | Serverless spatial SQL, raster/vector processing, notebooks, catalog, AI algorithms, and MCP at scale ([docs](https://docs.wherobots.com/)) | Cloud scale, notebooks, spatial SQL, and AI access are insufficient positioning. |
| **Felt AI** | Prompt-to-application and natural-language spatial SQL with confirmation ([Felt AI](https://www.felt.com/platform/felt-ai)) | Prompt-to-map is useful UX, not a trust model. |
| **Earth Engine** | Hosted catalog, planetary computation, task lifecycle, retries, and asset manifests ([guide](https://developers.google.com/earth-engine/guides/getstarted), [jobs](https://developers.google.com/earth-engine/guides/processing_environments)) | Scale and managed jobs alone are not the wedge. |
| **openEO / OGC API - Processes** | Portable process graphs, descriptions, jobs, status, and results ([openEO](https://docs.ogc.org/cs/24-059/24-059.html), [Processes](https://docs.ogc.org/is/18-062r2/18-062r2.html)) | A JSON DAG or job API without stronger semantics duplicates standards work. |
| **GeoGig** | Git-like history, branch, merge, and diff for geospatial datasets ([project](https://geogig.org/), [manual](https://geogig.org/docs/start/introduction.html)) | GeneGIS must diff analytical meaning and evidence, not only feature revisions. |

The root README's AI-native, cloud-native, GPU, workflow, and plugin pillars are
architectural choices—not unique product claims.

### The remaining gap

The strongest gap is between **provenance capture** and **claim verification**.
W3C PROV describes entities, activities, agents, and derivation
([PROV-O](https://www.w3.org/TR/prov-o/)). OpenLineage records datasets, jobs,
runs, and state transitions ([object model](https://openlineage.io/docs/next/spec/object-model/)).
OGC Testbed 20 demonstrated workflow provenance endpoints and Research Object
bundles ([report](https://docs.ogc.org/per/24-036.html)). These models do not by
themselves establish that:

- population and boundaries refer to compatible places and times;
- numerator and denominator have compatible aggregation semantics;
- area used an admissible CRS, method, and unit;
- a join preserved coverage and cardinality;
- an independent implementation agrees within a declared tolerance;
- a later change preserves the analytical conclusion;
- a reviewer can verify the result without trusting the original UI or LLM.

GeneGIS adds this claim-verification layer while exporting existing provenance
standards instead of inventing a replacement vocabulary.

The need is measurable. GISAgentBench reports that its best evaluated agent
completed only 32.7% of 349 realistic multi-step tasks under strict,
tolerance-aware output scoring, and warns that code similarity, trajectory
matching, or model judges can confuse resemblance with correctness
([paper](https://arxiv.org/abs/2608.01645)). MapEval also reports a material
model-to-human spatial reasoning gap ([paper](https://proceedings.mlr.press/v267/dihan25a.html)).
GeneGIS must score actual artifacts and invariants; the planner never grades
itself.

## Positioning decision

**Category:** Open geospatial build, review, and verification system.

> GeneGIS turns spatial intent into a proof-carrying result: an open map or
> dataset whose meaning, sources, workflow, checks, and changes can be reviewed
> like code and verified like a build artifact.

The initial wedge is public-sector and research analysis using published
statistics and administrative or environmental data. Such work has reference
values, recurring updates, review obligations, and expensive silent errors.
Nagoya population density is the seed because it exercises semantic joins,
reference years, CRS/area choices, public sources, and published totals.

The first user problem is: **update or reproduce a consequential spatial
result, explain every change, and release it only when required evidence still
passes.**

## Seven differentiating product primitives

### D1. GeoContract spatial semantic types

Every input/output port carries geometry kind, CRS/axis/unit, extent/resolution,
value kind and units, numerator/denominator meaning, reference time,
population/universe, aggregation basis, coverage/cardinality, null/join policy,
source identity, license, freshness, and uncertainty/tolerance. Unknown meaning
stays `unknown`; the agent cannot silently choose a plausible default.

### D2. Independent verification graph

Each verified claim declares verifier, oracle, tolerance, and independence
class. Verification may use a second engine/algorithm, authoritative total,
conservation law, or domain invariant. Re-running the same function or asking
an LLM is not independent verification. Execution and verification graphs are
separately inspectable.

### D3. Open verified result capsule

An ordinary directory or ZIP-compatible Research Object contains or references
artifacts and digests, Command/DAG, GeoContracts, sources, assumptions,
approvals, policies, checks, engine identities, and relevant logs. It exports
W3C PROV or Workflow Run RO-Crate, with optional OpenLineage events and signed
in-toto-compatible attestations. SLSA is only an analogy; GeneGIS will not claim
SLSA conformance for spatial results ([SLSA provenance](https://slsa.dev/spec/v1.2/provenance)).
The minimum verifier works offline without an LLM or GeneGIS server.

### D4. Spatial CI and semantic diff

`genegis diff` compares source/time/license drift; contract/schema/coverage;
DAG nodes, edges, parameters and policies; feature and join changes; geometry,
topology, extent and distribution changes; raster resolution/nodata/statistics;
result statistics, rankings and classifications; and verification outcomes.
Pixel diff is advisory, never the sole correctness test. Pull requests and
scheduled updates can fail when a semantic budget is exceeded.

### D5. Machine-derived trust levels

| Level | Meaning |
| --- | --- |
| `exploratory` | Result exists; unknowns and unverified operations are visible. |
| `replayable` | Resolved inputs, workflow, engines, and artifact identities support replay. |
| `verified` | Required contracts and independent checks passed under a named policy. |
| `attested` | A recognized builder/verifier signed the verified capsule and subjects. |

These are evidence-derived policy states, not model confidence. Mutable sources,
expired policies, revoked signers, or incompatible engines can downgrade trust.

### D6. Cross-engine equivalence

Native, DuckDB Spatial, GDAL, PostGIS, GRASS, QGIS Processing, and OGC API
adapters declare semantic capabilities and evidence hooks. A conformance corpus
checks equivalent operations within documented tolerances. Opaque script nodes
are allowed for exploration but lower trust.

### D7. Trust debugger UX

The TUI/workbench is a review cockpit, not a small QGIS canvas:

```text
Claim -> Contract -> Plan -> Inputs -> Execution -> Checks -> Artifact
                     |                     |
                     +---- assumptions ----+
```

Its verbs are `plan`, `inspect`, `approve`, `run`, `verify`, `diff`, `explain`,
`replay`, and `seal`. The map is linked visual evidence. The primary view shows
contract failures, source drift, changed conclusions, and safe remediation.

## Defensibility

The moat is the accumulated, openly testable trust system:

1. spatial semantic contracts and compatibility rules;
2. mutation-tested positive and negative workflow corpus;
3. independent public-data oracles and tolerances;
4. adapter conformance results across engines and versions;
5. institutional verification policies;
6. portable capsules useful outside GeneGIS.

Open outputs reduce adoption risk and let contracts, fixtures, policies, and
verifier plugins create a stronger ecosystem than an algorithm marketplace.

## Claims we will not make

- “The first AI-native GIS,” “the only agentic GIS,” or “GIS by prompt.”
- That DAGs, MCP, traceability, cloud formats, GPU, or natural-language SQL are unique.
- That provenance, signatures, visual similarity, or two correlated engines prove truth.
- That existing GIS, databases, processors, or agent platforms must be replaced.

## Success metrics

| Dimension | Phase-11 target |
| --- | --- |
| False trust | **0** negative fixtures obtain `verified` or `attested`. |
| Mutation strength | Catch **≥95%** of defined source/contract/graph/result mutations; review every survivor. |
| Independent checks | **100%** of release claims name verifier, independence class, tolerance, and evidence. |
| Portability | A clean offline verifier validates capsule digests and policy with no server or LLM. |
| Semantic diff | Classify every Nagoya update change; unclassified changes are **0**. |
| Cross-engine agreement | Native and DuckDB/GDAL paths agree within policy for Nagoya and at least **20** cases. |
| Review UX | Reviewer finds each seeded failure in **≤2 minutes median** without raw JSON. |
| Agent evaluation | Report exact artifact/tolerance scores on a licensed benchmark subset. |

## Decision filter

A roadmap feature must improve semantic error prevention, independent
verification, portable evidence, spatial diff/review, cross-engine openness, or
time-to-diagnose—without weakening another. Prompt spectacle, generic editing,
provider count, or raw throughput alone stay outside this track.

The bounded implementation sequence is [Phase 11](/home/sasaki/workspace/GeneGIS/docs/roadmap/phase-11-proof-carrying-analysis.md).
