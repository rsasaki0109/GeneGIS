# Phase 2: Alpha

**Goal:** Evolve the Phase 1 demo into a credible alpha — GPU map preview, optional LLM planning, richer data IO.

**Star target:** 1,000 → 5,000

## Tracks

| Track | Phase 2 focus |
|-------|----------------|
| **AI** | Model-agnostic LLM planner with rule-based fallback; keep GeoWorkflow IR + validators |
| **GPU** | WebGPU choropleth preview in `genegis-render` (Nagoya wards) |
| **Data** | GeoParquet read path + catalog metadata ✅ |
| **Desktop** | Tauri release build; workbench export buttons (PNG download) |

## Deliverables

- [x] Phase 2 roadmap (this document)
- [x] LLM planner backend (`genegis-ai`, OpenAI-compatible HTTP, env-configured)
- [x] CLI `--planner rule|llm` with automatic rule fallback
- [x] WebGPU choropleth renderer (`genegis-render`)
- [x] GeoParquet smoke read (`genegis-vector`)
- [x] Catalog metadata registry (`genegis-catalog`)
- [x] Workbench PNG download button
- [x] Tauri `npm run build` release artifact

## GeoParquet read (implemented alpha)

```rust
use genegis_vector::{read_geoparquet_bytes, read_geoparquet_path};

// From file
let dataset = read_geoparquet_path("data/wards.parquet")?;

// From in-memory bytes (cloud download)
let dataset = read_geoparquet_bytes(&parquet_bytes)?;
```

Reads GeoParquet metadata, decodes WKB geometry columns, and maps rows into the shared `VectorDataset` model used by GeoJSON.

## Catalog metadata (implemented alpha)

```rust
use genegis_catalog::{alpha_catalog, NAGOYA_WARDS_DENSITY_ID};

let catalog = alpha_catalog();
let dataset = catalog.require(NAGOYA_WARDS_DENSITY_ID)?;
```

The ask pipeline resolves the Nagoya north-star dataset through the catalog and returns `dataset` metadata in the API response / summary JSON.

## AI planner (implemented alpha)

```bash
# Default — rule-based (offline, deterministic)
genegis ask "名古屋市の人口密度を表示" --plan-only

# LLM backend (requires API key; falls back to rules on failure)
export GENEGIS_LLM_API_KEY=sk-...
genegis ask "Show Nagoya population density on a map" --planner llm --plan-only
```

Environment:

| Variable | Default | Purpose |
|----------|---------|---------|
| `GENEGIS_LLM_API_KEY` | — | Bearer token for OpenAI-compatible API |
| `GENEGIS_LLM_BASE_URL` | `https://api.openai.com/v1` | API base |
| `GENEGIS_LLM_MODEL` | `gpt-4o-mini` | Chat model |

## Out of scope

- Full agent autonomy without human approval
- Marketplace / billing
- Multi-tenant cloud deployment

## North star (unchanged)

「名古屋市の人口密度を表示」 — must keep working offline via rule planner.

## Next: Phase 3 Beta

See [phase-3-beta.md](phase-3-beta.md).
