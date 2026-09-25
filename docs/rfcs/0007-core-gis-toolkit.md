# RFC 0007: Core GIS Toolkit

- **Status:** Accepted for implementation — **landed** (2026-09-25) in
  `crates/genegis-toolkit`, `/api/gis/*` on the Workbench, and the
  「データ分析」 view.
- **Date:** 2026-09-25
- **Scope:** The general-purpose GIS foundation that the scenario-specific
  workflows lacked: bring your own data, generic spatial operations, an AI
  planner that composes them, arbitrary places, attribute inspection, and
  export — all through Command + Workflow Graph with CRS, units, sources, and
  provenance recorded.

## Problem

Before this RFC, every analysis in GeneGIS was a hand-built scenario module
(`nagoya.rs`, `flood.rs`, `evacuation.rs`, …) selected by keyword matching in
`genegis-ai`. A user could not:

1. load their own CSV, Shapefile, GeoPackage, or GeoJSON;
2. run a buffer, clip, overlay, dissolve, spatial join, or reprojection;
3. ask a question that no template anticipated
   (「駅から500m以内の避難所を数えて」);
4. work anywhere other than Nagoya;
5. inspect, filter, or classify attributes;
6. take results out as CSV, GeoPackage, or a print map.

## Decision

Add one crate, `genegis-toolkit`, that owns the general-purpose path end to
end and plugs into the existing Command bus and Workflow IR instead of
replacing them.

```text
file / place / prompt
      │
      ▼
import_through_workflow ─┐           planner (rules | LLM)
resolve_place ───────────┤                 │ ToolkitPlan (JSON)
                         ▼                 ▼
               LayerStore (content-addressed, digest-verified)
                         │                 │
                         └──► plan_to_workflow ──► GeoWorkflow
                                   (input contracts pin every layer by
                                    digest + CRS; steps = toolkit.<op>)
                                              │
                               Command::RunWorkflow (digest-checked)
                                              │
                               executor runs the *registered* graph,
                               every op returns independent checks;
                               any failed check aborts the command
                                              │
                                   output layer + RunReceipt
```

### 1. Import

`import::import_bytes` detects the format from extension and magic bytes and
reads GeoJSON, CSV/TSV (lon/lat, 経度/緯度, x/y, or WKT columns), zipped
Shapefile (`.shp/.dbf/.prj/.cpg`, hand-written reader), GeoPackage (via
bundled SQLite), and GeoParquet (WKB). Text decoding honours an explicit
encoding or `.cpg`, then UTF-8, then Shift_JIS — the common encoding of
Japanese government data.

CRS resolution is ordered and recorded as `CrsStatus`:
`UserSupplied > Declared (.prj, GPKG SRS, PROJJSON, CSV crs column) >
FormatDefault (RFC 7946) > Inferred (lon/lat ranges; flagged for
confirmation) > CrsRequired (fail closed)`. Rows that cannot be converted
are reported with row numbers, never dropped silently. Codes with leading
zeros (市区町村コード) stay text.

Imports run through `RunWorkflow`: the upload is an input contract pinned by
its SHA-256, and the executor rejects bytes that do not match.

### 2. Operations

`ops::catalog()` defines 17 operations: `make_points`, `buffer`, `clip`,
`erase`, `intersect`, `dissolve`, `spatial_join`, `select_by_location`,
`distance_to_nearest`, `reproject`, `assign_crs`, `make_valid`, `centroid`,
`measure`, `filter`, `calculate`, `summarize`. Rules shared by all of them:

- **Units are mandatory.** Distances must be `"500 m"`, `"1.5 km"`, or
  `{value, unit}`; bare numbers and degrees are rejected.
- **Metric work happens in a metric CRS.** A projected metre CRS is used as
  is; geographic data and Web Mercator are projected to the local UTM zone.
  The working CRS is written to the step notes.
- **Measurements are geodesic.** Areas and lengths use Karney's method on the
  WGS 84 ellipsoid after normalising ring orientation (a clockwise exterior
  would otherwise be measured as "the Earth minus the polygon").
- **Polygon operations require valid geometry.** Rings are normalised to
  OGC orientation before buffer, union, and clipping (the overlay engine
  reads ring direction), and self-intersecting polygons are rejected with a
  message naming the feature IDs and the `make_valid` step that repairs them.
  Imports and layer summaries report invalid features up front.
- **Output fields carry units** (`area_km2: km²`, `buffer_m: m`,
  `count: features`, `population: persons`).
- **Empty results keep their schema,** and an ungrouped `summarize` over
  nothing still answers (count 0, sum 0).
- **Every operation returns independent checks**, computed by a different
  method than the result itself, for example:

| Operation | Independent check |
|-----------|-------------------|
| `buffer` | buffer contains its source; point buffers match πr² within 2 % |
| `spatial_join` | match counts recomputed with ray casting / DE-9IM; area-weighted totals never exceed the source total for disjoint targets |
| `select_by_location` | point selections re-measured geodesically away from the threshold |
| `distance_to_nearest` | projected distances vs. geodesic distances within 0.5 % |
| `measure` | geodesic area vs. planar area in the UTM zone within 1 % |
| `reproject` | coordinate round trip within 1e-7° / 1 mm |
| `clip` / `erase` | output within mask extent / no residual overlap |
| `dissolve`, `summarize`, `filter` | membership conserved |

Independent checks caught three real defects during development before any
result was shown: clockwise rings measured as "the Earth minus the polygon"
(geodesic vs. planar area), clockwise rings buffered to nothing, and
self-intersecting sample flood zones buffered to empty polygons (buffer
contains source).

### Built-in projections

`proj.rs` implements transverse Mercator with the Krüger n-series on GRS80,
covering JGD2011/JGD2000 Japan Plane Rectangular CS I–XIX, WGS 84 UTM 1–60
N/S, JGD UTM 51–55 N, Web Mercator, and geographic WGS 84 / JGD2000 /
JGD2011. `genegis-crs` now registers the same codes so workflow contracts
validate them. JGD2000/JGD2011/WGS 84 are treated as coincident (≤ ~1 m) and
the assumption is recorded whenever a transform crosses datum labels. The
Tokyo datum fails closed because it needs a grid shift. Tests pin the zone VII
origin, UTM false origin, millimetre round trips, and the projected/geodesic
scale factor.

### 3. Planner

`planner::plan` returns a `ToolkitPlan` (JSON steps over the catalog):

- the **rule planner** handles common Japanese/English questions offline —
  distance selection and counting, 「この点から…」 with a clicked point,
  per-polygon counts, population by area-weighted apportionment, density,
  nearest distance, clip, buffer, reprojection, measurement, grouping —
  and converts 「徒歩N分」 at 80 m/min with the assumption stated;
- the **LLM planner** (OpenAI-compatible, same `GENEGIS_LLM_*` variables as
  `genegis-ai`) receives the catalog and the loaded layers' schemas and may
  compose any graph. Its output is validated (known ops, exact input roles,
  existing layer IDs, no dangling steps); rejected plans are fed back for up
  to three attempts, and `auto` mode falls back to rules.

The planner never verifies: plans are validated structurally, then executed
through the Command bus where operation checks decide acceptance.

The rule planner inserts `make_valid` automatically when a source layer with
invalid polygons feeds a polygon operation, and says so in its rationale.

### 3a. MCP server and agent planners

`genegis-mcp` (a binary in `genegis-toolkit`, no new dependencies) exposes the
toolkit to MCP clients such as Claude Code over stdio JSON-RPC: `list_layers`,
`list_operations`, `load_sample_data`, `import_layer`, `describe_layer`,
`query_table`, `suggest_plan`, `run_plan`, `resolve_place`, `export_layer`,
`remove_layer`. The repository's `.mcp.json` registers it for Claude Code;
layers live in the same `.genegis/layers` store as the Workbench, so agent
results appear in the データ分析 view.

The agent composes `ToolkitPlan` JSON; `run_plan` validates it, executes it
through `RunWorkflow`, and returns step receipts, checks, units, and digests —
or a tool error with the reason (missing unit, unknown field, invalid
polygons, failed check) that the agent can act on. The agent is never its own
verifier.

### 3b. Planner evaluation

`crates/genegis-toolkit/examples/planner_eval.rs` defines 17 questions over
the Nagoya samples, each with a hand-written ground-truth plan. An answer is
correct only when the planner's *verified output layer* contains a numeric
field matching the ground truth within 0.5 %; two questions must be declined.
Five questions are held out: they were written after the rule planner was
tuned and are never used to tune it. `scripts/eval-mcp-planner.py` runs the
same questions through headless Claude Code (`claude -p`) with only the
genegis MCP server available and scores the store it wrote to; the agent's
prose is not scored.

Results (2026-09-25/26, [rule](../reports/rfc-0007-planner-eval-rule.json),
[Claude Code](../reports/rfc-0007-planner-eval-claude-code.json)):

| Planner | Correct | Held-out | Notes |
|---------|---------|----------|-------|
| Rule (offline) | 13/17 | 1/5 | All 12 calibration questions correct (CI gate). On held-out questions it answers confidently wrong rather than declining, e.g. it drops the negation in 「1km以内に**ない**店舗」. |
| Claude Code over MCP (claude-opus-5-5) | 15/17 | 3/5 | US$2.84 for 17 headless sessions, 3–8 turns each. After invalid flood-zone polygons began to be rejected, it read the error and added `make_valid` on its own. Both misses are ambiguous questions (店舗 read as supermarkets only; per-ward totals grouped by the attribute rather than by location) and are annotated, not re-scored. |

Findings: the rule planner is a reliable offline baseline for the question
shapes it knows but must not be trusted outside them; an agent planner covers
compositional questions, and the verification boundary — not the agent —
decides what counts as an answer.

### 4. Places

`place::resolve_place` resolves any name through OpenStreetMap Nominatim
(boundaries, ODbL) or 国土地理院 地名・住所検索 (points). The response bytes
are hashed into the source snapshot, all candidates are returned, boundary
candidates are preferred, and license/attribution travel with the layer.
Place data can then be combined with any imported statistics (for example
clip e-Stat mesh data to 札幌市).

### 5. Attributes

`expr.rs` is a small SQL-like language (`AND/OR/NOT`, comparisons, `IN`,
`LIKE`, `IS NULL`, arithmetic, `round/abs/lower/…`, Japanese field names)
shared by the table filter, `filter`, and `calculate`. `table.rs` provides
paged/sorted queries, field statistics, classification (equal interval,
quantile, Jenks natural breaks, categorical; colour-blind-safe palettes), and
picking features at a clicked location with a metric tolerance.

### 6. Export

`export.rs` writes RFC 7946 GeoJSON, CSV (UTF-8 BOM, WKT + CRS column,
re-importable), GeoPackage 1.4, GeoParquet 1.1 (with a
`genegis:provenance` key), and an A4 PDF map written without external
libraries: conformal display CRS, legend with class counts, scale bar, north
arrow, CRS, attribution, license, and the layer/workflow digests. Japanese
text uses the Adobe-Japan1 CID font by reference (not embedded). Every export
returns a receipt with the output SHA-256.

## Workbench

`apps/workbench/src/gis.rs` exposes `/api/gis/*` (layers, import, samples,
geojson, table, stats, classify, style, rename, crs, export, pick,
operations, plan, run, ask, place). Layers persist under `.genegis/layers`
(`GENEGIS_LAYER_DIR`) as lossless JSON and are re-verified against their
content ID on load; tampered files are refused.

The 「データ分析」 view adds file drop/CRS prompt, sample data, place search,
a layer list (visibility, colour classes, export, CRS confirmation), an SVG
map with optional 地理院タイル basemap, click-to-inspect and
click-to-set-point, a filterable/sortable attribute table, and a results pane
showing the executed graph, every check, units, digests, and sources.

## CLI and Claude Code

`genegis gis import <file> [--crs …] [--out …]` imports (and converts) any
supported format through `RunWorkflow`; `genegis gis ask "<question>" --layer
<file> …` plans, executes, verifies, and prints the run receipt; `genegis gis
ops` lists the catalog. CI runs an import → GeoPackage → re-import → ask →
PDF smoke test. CI also runs the rule planner against the 12 calibration questions
(`planner_eval rule --require-calibration`).

From Claude Code, the repository's `.mcp.json` starts `genegis-mcp` through
`cargo run`; approve the `genegis` server when prompted and ask spatial
questions in plain language.

## Non-goals and limits

- No raster analysis, network routing, or 3D in the toolkit (those remain in
  their existing crates).
- No automatic statistics download for arbitrary cities: e-Stat requires an
  application ID; users import statistics and combine them with places.
- CSV cannot carry types; use GeoPackage/GeoParquet for lossless exchange.
- The PDF references, and does not embed, a Japanese font; viewers substitute
  an installed Gothic face.
- The Tauri shell uses these features through the Workbench HTTP server, like
  the existing STAC and composer panels.

## Verification

`cargo test -p genegis-toolkit` covers projections, WKB/WKT, every importer
(including Shift_JIS DBF and projected data without a CRS), every operation
and its checks, plan validation and deterministic digests, the rule planner
end to end on the Nagoya sample data, LLM plan validation, place parsing,
the expression language, classification, picking, all exports, and store
persistence/tamper detection.

The new dependencies change `Cargo.lock`, which the Phase 14 M1 GPU receipt
binds by digest. The receipt was therefore re-measured on the reference GTX
1660 Ti (release build, Vulkan backend, 2026-09-25: first frame 1.18 s ≤ 2.0 s,
steady state 628 fps ≥ 30 fps) instead of re-stamping the old measurement; the
previous receipt is archived in
`docs/reports/gpu-audit/pre-rfc-0007/`. The Horizon 4 H4.6 performance-matrix
receipts still reference the pre-RFC 0006 lock digest; they are not checked by
tests and need the GDAL-backed testkit collector to regenerate.

