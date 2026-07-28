# Phase 10: Federated Catalog Search

**Goal:** Search and bind datasets across federated STAC endpoints and cloud GeoParquet assets.

**Status:** Complete.

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
- [x] Persist named endpoint registry with authentication metadata
- [x] Add workbench federated discovery panel

### Phase 10 beta — cloud execution

- [x] Read remote GeoParquet metadata and selected row groups with HTTP range requests
- [x] Bind a discovered asset through Command + GeoWorkflow IR
- [x] Verify schema, CRS, units, license, and source coverage across binding and execution
- [x] Record source URL, STAC identity, retrieval time, and verification provenance

### Phase 10 gamma — agent and release hardening

- [x] Let the planner compare compatible candidates and explain its selection
- [x] Add allowlisted domains, response-size limits, timeouts, and redirect policy
- [x] Add federated-search → bind → execute → verify E2E coverage
- [x] Keep the offline Nagoya north-star workflow passing in CI

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

The Workbench discovery panel shows the selected asset, verification count, and
score. For remote GeoParquet assets, **Range Read + verify** returns the Command,
GeoWorkflow, binding decision, execution report, and machine-readable checks in
one receipt.

External hosts are denied by default. Allow trusted STAC and asset hosts with:

```bash
GENEGIS_REMOTE_ALLOWED_HOSTS=earth-search.aws.element84.com,example-bucket.s3.amazonaws.com
```

Exact hostnames and `*.example.org` wildcard suffixes are supported. Loopback is
allowed for local fixtures. The default policy rejects URL credentials and
redirects, limits each response to 8 MiB, and applies a 15-second global timeout.

## North star (unchanged)

「名古屋市の人口密度を表示」 — offline rule planner + DuckDB verification must keep passing in CI.

See [`phase-9-external-data.md`](phase-9-external-data.md).
