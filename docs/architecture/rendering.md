# GeneGIS Rendering Architecture

GPU-first rendering via **wgpu** (WebGPU-compatible).

## Phase 0

- `genegis-render`: clear-color canvas prototype
- Target: 60fps interactive scenes with progressive loading

## Phase 2 (alpha)

- `genegis-render`: WebGPU choropleth for Nagoya wards (earcut triangulation + per-feature colors)
- Example: `cargo run -p genegis-render --example choropleth_nagoya` (requires GPU + display)

## Phase 3 (beta)

- `genegis-render`: tiled 2×2 grid + 3 LOD levels; mouse-wheel zoom selects simplification
- GPU preview window uses multi-batch tile draws (`ChoroplethTiledGpu`)

## Pipeline (target)

```
Frame → visible layers → camera → chunk/tile/LOD requests
     → async decode → GPU upload → style → cull → draw → labels → present
```

## Memory budget

Each layer gets a GPU budget. Eviction by visible priority; refine LOD when idle.

## Modes

| Mode | Priority |
|------|----------|
| Interactive | Speed |
| Inspect | Picking accuracy |
| Presentation | Visual quality |
| Export | Deterministic high-res |
| Benchmark | Reproducible perf |

See master RFC for vector/raster/point cloud/tile sub-pipelines.
