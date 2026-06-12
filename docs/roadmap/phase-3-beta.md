# Phase 3: Beta

**Goal:** Scale beyond the alpha demo — STAC-aware discovery, cloud raster IO, and GPU rendering integrated into the workbench.

**Star target:** 2,500 → 5,000

## Tracks

| Track | Phase 3 focus |
|-------|-----------------|
| **GPU** | Workbench GPU preview hook; tiled/LOD choropleth path in `genegis-render` ✅ |
| **Data** | STAC Item export; COG smoke read (`genegis-raster`); catalog → planner wiring |
| **Cloud** | HTTP range-read prototype for catalog assets ✅ |
| **AI** | Planner resolves datasets from catalog/STAC metadata |
| **Perf** | Reproducible render + pipeline benchmarks (`genegis-testkit`) |

## Deliverables

- [x] Phase 3 roadmap (this document)
- [x] STAC Item export from catalog (`genegis-catalog`)
- [x] Workbench dataset + catalog panel
- [x] COG smoke read (`genegis-raster`)
- [x] Workbench GPU preview launcher (native WebGPU choropleth window)
- [x] Planner catalog lookup (beyond hard-coded Nagoya id)
- [x] Pipeline/render benchmark harness
- [x] HTTP range-read prototype (`genegis-storage`)
- [x] Tiled/LOD choropleth path (`genegis-render`)

## STAC export (implemented)

```rust
use genegis_catalog::{alpha_catalog, NAGOYA_WARDS_DENSITY_ID};

let item = alpha_catalog()
    .require(NAGOYA_WARDS_DENSITY_ID)?
    .to_stac_item();
```

The ask pipeline also returns `stac_item` on `AskPipelineResult`.

## COG read (implemented)

```bash
genegis raster info /path/to/file.tif
```

```rust
use genegis_raster::{read_cog_bytes, read_cog_path, read_cog_window_u8};

let info = read_cog_path("dem.tif")?;
let window = read_cog_window_u8("dem.tif", 0, 0, 256, 256)?;
```

Reads GeoTIFF metadata (EPSG, bounds, tiling, overviews) and supports partial window decode for cloud-native range-read workflows.

## GPU preview (implemented)

After a successful ask pipeline run, click **Open GPU Map** in the workbench. This launches the native WebGPU choropleth window (`genegis-render`) on a background thread.

API:

```bash
curl -X POST http://127.0.0.1:7812/api/gpu-preview
```

```rust
use genegis_analysis::spawn_nagoya_gpu_preview;

spawn_nagoya_gpu_preview()?;
```

Requires a local GPU and display (same as `cargo run -p genegis-render --example choropleth_nagoya`).

## Planner catalog lookup (implemented)

The rule-based and LLM planners bind workflows to catalog datasets via tag matching (`genegis-catalog::Catalog::match_dataset`). `ResolvedWorkflow.dataset_id` flows into the ask pipeline instead of a hard-coded Nagoya id.

```rust
use genegis_ai::plan_from_prompt;
use genegis_catalog::NAGOYA_WARDS_DENSITY_ID;

let plan = plan_from_prompt("名古屋市の人口密度を表示")?;
assert_eq!(plan.resolved.dataset_id, NAGOYA_WARDS_DENSITY_ID);
```

## Benchmark harness (implemented)

Reproducible north-star benchmarks live in `genegis-testkit`:

```bash
genegis bench
genegis bench pipeline --iterations 20
genegis bench render --json
```

```rust
use genegis_testkit::{benchmark_pipeline, benchmark_render_mesh, run_all_benchmarks};

let pipeline = benchmark_pipeline(2, 10)?;
let render = benchmark_render_mesh(2, 10)?;
let report = run_all_benchmarks(2, 10)?;
println!("pipeline median: {:.2} ms", pipeline.median_ms());
```

Targets:

| Sample | Measures |
|--------|----------|
| `pipeline` | Full ask pipeline (plan → analyze → DuckDB verify → HTML/PNG export) |
| `render_mesh` | CPU choropleth triangulation (`ChoroplethMesh::build`, 1280×720) |

## HTTP range-read (implemented)

Cloud-native partial reads for catalog assets live in `genegis-storage`. Remote COG metadata uses streaming HTTP range reads via `geotiff-reader` (`read_mode: "http_range"`) — no full-file download.

```bash
genegis storage fetch https://example.com/data.tif --range 0-65535 --json
genegis raster info https://example.com/dem.tif
```

```rust
use genegis_storage::{fetch_http_range, probe_http_content_length, ByteRange, COG_HEADER_PREFIX_BYTES};
use genegis_raster::{read_cog_uri, read_cog_window_uri, HttpOpenOptions};

let bytes = probe_http_content_length(url)?;
let info = read_cog_uri(url)?; // http_range — chunked IFD reads
let window = read_cog_window_uri(url, 0, 0, 256, 256)?;
let header = fetch_http_range(url, &ByteRange::prefix(COG_HEADER_PREFIX_BYTES - 1)?)?;
```

Local paths continue to work unchanged (`read_mode: "local"`). Generic asset fetch still uses `ureq`; COG decode uses `geotiff-reader`'s cached range source.

## Tiled / LOD choropleth (implemented)

The GPU preview uses a tiled mesh path with zoom-driven LOD (mouse wheel in the choropleth window):

```rust
use genegis_render::{
    lod_for_zoom, ChoroplethMap, ChoroplethTiledLodMap, TiledLodConfig,
};

let tiled = ChoroplethTiledLodMap::prepare(&map, TiledLodConfig::default());
let lod = lod_for_zoom(1.0, tiled.lod_levels());
let mesh = tiled.build_merged_mesh(1280.0, 720.0, lod);
```

Default grid: 2×2 tiles, 3 LOD levels. Scroll to zoom out and simplify geometry; zoom in for full-detail ward boundaries.

## Out of scope

- Full STAC API server / OGC API Features
- COPC point cloud (Phase 4 candidate per RFC)
- Multi-tenant cloud deployment
- Plugin marketplace

## North star (unchanged)

「名古屋市の人口密度を表示」 — must keep working offline via rule planner.

## Prerequisites (Phase 2 complete)

- WebGPU choropleth (`genegis-render`)
- GeoParquet smoke read (`genegis-vector`)
- Catalog metadata registry (`genegis-catalog`)
- LLM planner + Tauri release build

## Next phase

Phase 3 is complete. Continue with [`phase-4-plugins.md`](phase-4-plugins.md) (COPC alpha + WASM plugin SDK).
