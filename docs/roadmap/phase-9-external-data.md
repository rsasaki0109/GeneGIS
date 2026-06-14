# Phase 9: External STAC & GeoParquet Workflows

**Goal:** Discover datasets beyond the bundled alpha catalog — fetch external STAC collections/items and verify GeoParquet-backed workflows through the agent graph.

**Star target:** 30,000 → 40,000

## Tracks

| Track | Phase 9 focus |
|-------|----------------|
| **Catalog** | External STAC fetch + overlay import |
| **Vector** | GeoParquet read + feature-count verification |
| **Agent** | Fourth workflow (`nagoya-geoparquet`) |
| **CLI / Workbench** | `stac fetch|import`, geoparquet fixture + info |
| **Docs** | Phase 9 roadmap + orchestration guide update |

## Deliverables

### Phase 9 alpha (external STAC + GeoParquet smoke) — complete

- [x] Phase 9 roadmap (this document)
- [x] `fetch_stac_collection` / `fetch_stac_item` + catalog overlay import
- [x] CLI `genegis catalog stac fetch|import URL`
- [x] Bundled Nagoya GeoParquet fixture writer + catalog entry
- [x] Workflow `nagoya-geoparquet` — offline feature-count verify via agent
- [x] CI smoke — local geoparquet agent + STAC fetch fixture

### Phase 9 beta (discovery in planner) — complete

- [x] Planner `stac_fetch` tool for external collection URLs
- [x] Workbench external STAC import panel (`/api/stac/fetch|import|overlay`)
- [x] GeoParquet density pipeline (`nagoya-geoparquet-density` + DuckDB verify)
- [x] Workflow `external-stac-demo` — fetch bundled sample collection via agent
- [x] CI smoke — external STAC agent, GeoParquet density agent, overlay import

### Phase 9 gamma (release hardening) — complete

- [x] CI six-workflow agent matrix (north star + 5 Phase 9 workflows)
- [x] Audit bundle v3 — alpha + overlay + merged STAC snapshots
- [x] Ask pipeline smoke for GeoParquet density
- [x] Overlay path isolation (`GENEGIS_CATALOG_OVERLAY_PATH`) for tests
- [x] Orchestration guide + verification matrix update
- [x] GitHub Release v0.9.0

## Verification matrix

| Workflow | Execute tool | Verifier | Offline? |
|----------|--------------|----------|----------|
| `nagoya-density` | `run_nagoya_density` | `duckdb_verify` | Yes |
| `remote-cog-demo` | `run_remote_cog_metadata` | `cog_metadata_verify` | Needs HTTP |
| `local-cog-demo` | `run_local_cog_metadata` | `cog_metadata_verify` | Yes |
| `nagoya-geoparquet` | `run_geoparquet_read` | `geoparquet_feature_verify` | Yes |
| `nagoya-geoparquet-density` | `run_geoparquet_density` | `duckdb_verify` | Yes |
| `external-stac-demo` | `run_stac_fetch` | `stac_collection_verify` | Yes (sample JSON) |

## CLI (target)

```bash
genegis catalog stac fetch https://example.com/collection.json
genegis catalog stac import https://example.com/items/item-id.json
genegis vector geoparquet info examples/nagoya-population-density/data/nagoya-wards.parquet
genegis agent run "名古屋 wards GeoParquet を検証"
genegis agent run "名古屋 GeoParquet 人口密度を表示"
genegis agent run "外部STAC examples/stac/sample-collection.json を fetch"
genegis workflow run nagoya-geoparquet-density --execute
genegis ask "名古屋 GeoParquet 人口密度を表示" --no-html
genegis agent export-audit -o .genegis/audit-bundle.json
```

## North star (unchanged)

「名古屋市の人口密度を表示」 — offline rule planner + DuckDB verification must keep passing in CI.

## Next

Phase 10 — federated catalog search and cloud GeoParquet execution (TBD): draft `docs/roadmap/phase-10-federated-catalog.md`.

See [`phase-8-intent-expansion.md`](phase-8-intent-expansion.md) and [`docs/guides/agent-orchestration.md`](../guides/agent-orchestration.md).
