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

### Phase 9 alpha (external STAC + GeoParquet smoke)

- [x] Phase 9 roadmap (this document)
- [x] `fetch_stac_collection` / `fetch_stac_item` + catalog overlay import
- [x] CLI `genegis catalog stac fetch|import URL`
- [x] Bundled Nagoya GeoParquet fixture writer + catalog entry
- [x] Workflow `nagoya-geoparquet` — offline feature-count verify via agent
- [x] CI smoke — local geoparquet agent + STAC fetch fixture

### Phase 9 beta (discovery in planner)

- [ ] Planner `stac_fetch` tool for external collection URLs
- [ ] Workbench external STAC import panel
- [ ] GeoParquet density pipeline (parquet → density choropleth)

## CLI (target)

```bash
genegis catalog stac fetch https://example.com/collection.json
genegis catalog stac import https://example.com/items/item-id.json
genegis vector geoparquet info examples/nagoya-population-density/data/nagoya-wards.parquet
genegis agent run "名古屋 wards GeoParquet を検証"
```

## North star (unchanged)

「名古屋市の人口密度を表示」 — offline rule planner + DuckDB verification must keep passing in CI.

See [`phase-8-intent-expansion.md`](phase-8-intent-expansion.md).
