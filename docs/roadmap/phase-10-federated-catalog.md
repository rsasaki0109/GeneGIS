# Phase 10: Federated Catalog Search

**Goal:** Search and bind datasets across federated STAC endpoints and cloud GeoParquet assets.

**Status:** Alpha in progress.

## Tracks

| Track | Focus |
|-------|--------|
| **Catalog** | Federated STAC search + merge into overlay |
| **Storage** | Cloud GeoParquet range-read execution |
| **Agent** | Multi-source workflow binding |
| **Workbench** | Federated discovery UI |

## Deliverables

### Phase 10 alpha — federated discovery

- [x] Define endpoint, search request, source attribution, and result summary models
- [x] Search multiple STAC endpoints with partial-failure isolation
- [x] Normalize and deduplicate STAC Items while retaining every source endpoint
- [x] Add offline FeatureCollection fixtures and catalog unit tests
- [x] Add `genegis catalog stac search` CLI entry point
- [ ] Persist named endpoint registry with authentication metadata
- [ ] Add workbench federated discovery panel

### Phase 10 beta — cloud execution

- [ ] Read remote GeoParquet metadata and selected row groups with HTTP range requests
- [ ] Bind a discovered asset through Command + GeoWorkflow IR
- [ ] Verify schema, CRS, units, license, and source coverage before execution
- [ ] Record source URL, STAC identity, retrieval time, and verification provenance

### Phase 10 gamma — agent and release hardening

- [ ] Let the planner compare compatible candidates and explain its selection
- [ ] Add allowlisted domains, response-size limits, timeouts, and redirect policy
- [ ] Add federated-search → bind → execute → verify E2E coverage
- [ ] Keep the offline Nagoya north-star workflow passing in CI

## Alpha CLI

```bash
genegis catalog stac search \
  --endpoint local=examples/stac/sample-search.json \
  --bbox 136.79,35.03,137.07,35.27 \
  --limit 10
```

HTTP endpoint URLs may be either a STAC API root or an explicit `/search` URL.
Local endpoints are STAC ItemCollection / GeoJSON FeatureCollection fixtures and
use the same request and result model as HTTP searches.

## North star (unchanged)

「名古屋市の人口密度を表示」 — offline rule planner + DuckDB verification must keep passing in CI.

See [`phase-9-external-data.md`](phase-9-external-data.md).
