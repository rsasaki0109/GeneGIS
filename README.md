# GeneGIS

**AI-native · Cloud-native · GPU-native open geospatial workbench**

> If GIS were invented in 2026, it would not look like a 2000s desktop app.

GeneGIS is **not a QGIS clone**. It is a next-generation GIS platform built around workflow graphs, AI agents, cloud-optimized formats, and GPU rendering — designed for spatial intelligence in the GeoAI era.

## Why GeneGIS exists

Traditional GIS asks you to find data, fix CRS, wire geoprocessing by hand, validate results yourself, and export maps elsewhere. GeneGIS inverts that:

**Intent → Data Discovery → Workflow Graph → Verified Execution → Map / Insight / Report**

Example north-star prompt:

```text
名古屋市の人口密度を表示
```

GeneGIS resolves the place, discovers datasets, normalizes CRS, computes density, renders a choropleth, and shows sources + workflow graph + verification — not just a chat reply.

## Five differentiators

| Pillar | What it means |
|--------|----------------|
| **AI Agent Native** | Agents plan and verify spatial workflows, not just chat |
| **Cloud Native Data First** | GeoParquet, COG, COPC, PMTiles, STAC as first-class citizens |
| **GPU First** | LOD, tiles, range reads — never load billions of features wholesale |
| **Figma for GIS** | Collaboration, comments, branches, style systems at the center |
| **VSCode for GIS** | WASM / TS / Rust / Python SDK + marketplace extensibility |

## Quick start (Phase 0–1)

```bash
# Build the workspace
cargo build --workspace

# Print the MVP workflow graph (IR only)
cargo run -p genegis-cli -- workflow run nagoya-density

# North-star one-liner (Intent → Workflow → Map)
cargo run -p genegis-cli -- ask "名古屋市の人口密度を表示"

# Plan only (human-in-the-loop / Strict mode preview)
cargo run -p genegis-cli -- ask "名古屋市の人口密度を表示" --plan-only

# Execute analysis + DuckDB verification + summary JSON
cargo run -p genegis-cli -- workflow run nagoya-density --execute

# Execute and export choropleth HTML map
cargo run -p genegis-cli -- workflow run nagoya-density -x --html -o nagoya-density.html

# Execute and export choropleth PNG map
cargo run -p genegis-cli -- ask "名古屋市の人口密度を表示" --png --no-html

# Rebuild ward boundaries from 国土数値情報 N03 (optional)
python3 scripts/build-nagoya-wards.py

# Full example (writes examples/nagoya-population-density/output/)
cargo run -p nagoya-population-density

# COPC metadata smoke (local PDAL fixture)
cargo run -p copc-metadata

# Plugin discovery smoke
genegis plugin list

# Collaboration smoke
genegis collab comment list
genegis collab export -o .genegis/collab.json

# Desktop workbench (Tauri — requires extra patches on Rust 1.94)
cd apps/desktop && npm install && npm run dev

# Local web workbench (recommended MVP launcher; no Tauri deps)
cargo run -p genegis-workbench

# WebGPU canvas prototype (requires GPU + display)
cargo run -p genegis-render --example canvas_prototype

# WebGPU choropleth — Nagoya population density (Phase 2 alpha)
cargo run -p genegis-render --example choropleth_nagoya
```

## Architecture at a glance

```
Intent → GeoWorkflow IR → Verified Execution → Map
              ↑
         AI + CLI + UI (all emit Commands)
              ↓
    GIS Core (Rust) + DuckDB + wgpu + Cloud formats
```

See [`docs/architecture/overview.md`](docs/architecture/overview.md) and [`docs/rfcs/0001-master-architecture.md`](docs/rfcs/0001-master-architecture.md).

## Repository layout

```
crates/     Rust engines (core, render, workflow, ai, …)
apps/       Desktop (Tauri), web, server, CLI shells
plugins/    Official and community extensions
sdk/        Rust, TypeScript, Python SDK
docs/       Architecture, ADRs, RFCs, roadmap
examples/   Reproducible demos (Nagoya density, COG, COPC, …)
```

## Roadmap → GitHub Stars

| Phase | Theme | Star target |
|-------|-------|-------------|
| 0 | Foundation / Manifesto | 0 → 300 |
| 1 | MVP: Nagoya density demo | 300 → 1,000 |
| 2 | Alpha: GPU choropleth, GeoParquet, catalog | 1,000 → 2,500 |
| 3 | Beta: STAC, COG, GPU workbench integration | 2,500 → 5,000 |
| 4 | Plugins & COPC — SDK, WASM host, point cloud alpha | 5,000 → 7,500 | [`docs/roadmap/phase-4-plugins.md`](docs/roadmap/phase-4-plugins.md) |
| 5 | Figma for GIS — comments, branches, collab sync | 7,500 → 10,000 | [`docs/roadmap/phase-5-collab.md`](docs/roadmap/phase-5-collab.md) |
| 6 | Autonomous GIS platform | 10,000+ |

## Tech stack (decisions)

- **Core:** Rust
- **Rendering:** wgpu / WebGPU
- **Desktop:** Tauri + TypeScript UI
- **Local analytics:** DuckDB Spatial
- **Cloud vectors:** GeoParquet
- **Enterprise DB:** PostGIS

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). We use RFC culture for major design changes.

## License

Licensed under Apache-2.0 OR MIT at your option.

---

**GeneGIS is not a GIS with AI. GeneGIS is a GIS designed for AI agents and humans to collaborate.**
