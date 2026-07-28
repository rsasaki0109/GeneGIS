# GeneGIS

<p align="center">
  <img src="docs/assets/workbench-hero.gif" alt="GeneGIS workbench — natural language intent to verified Nagoya population density map" width="960" />
</p>

<p align="center"><sub>North-star prompt <code>名古屋市の人口密度を表示</code> · real pipeline output · DuckDB verified</sub></p>

**AI-native · Cloud-native · GPU-native open geospatial workbench**

> If GIS were invented in 2026, it would not look like a 2000s desktop app.

GeneGIS is **not a QGIS clone**. It is a next-generation GIS platform built around workflow graphs, AI agents, cloud-optimized formats, and GPU rendering — designed for spatial intelligence in the GeoAI era.

<p align="center">
  <a href="PLAYGROUND_URL"><strong>Open the zero-install Playground →</strong></a>
</p>

No install and no API key: run the North Star prompt, inspect all 14 workflow
operations, verify CRS and units, follow the sources, and share a reproducible
result URL. The public alpha replays a committed artifact generated and verified
by the Rust core.

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

## Try it in 30 seconds

The fastest path is the [public Playground](PLAYGROUND_URL). For local execution:

```bash
# North-star one-liner (Intent → Workflow → Map)
cargo run -p genegis-cli -- ask "名古屋市の人口密度を表示"

# Inspect the workflow without executing it
cargo run -p genegis-cli -- ask "名古屋市の人口密度を表示" --plan-only
```

### Develop the public Playground

```bash
cargo run -p genegis-analysis --example export_playground -- public/demo
npm ci
npm run check
npm run dev
```

See [`docs/adr/0005-public-playground.md`](docs/adr/0005-public-playground.md)
for the verified-replay architecture.

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

## Roadmap

| Phase | Theme |
|-------|-------|
| 4 | [Plugins & COPC — SDK, WASM host, point cloud alpha](docs/roadmap/phase-4-plugins.md) |
| 5 | [Figma for GIS — comments, branches, collab sync](docs/roadmap/phase-5-collab.md) |
| 6 | [Autonomous GIS platform — multi-agent orchestration](docs/roadmap/phase-6-autonomous.md) |
| 7 | [Audit trail & release workbench — run history + provenance UI](docs/roadmap/phase-7-release.md) |
| 8 | [Intent expansion — multi-workflow agent verify beyond Nagoya](docs/roadmap/phase-8-intent-expansion.md) |
| 10 | [Federated catalog search and cloud execution](docs/roadmap/phase-10-federated-catalog.md) |

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
