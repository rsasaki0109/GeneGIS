# RFC 0001: GeneGIS Master Architecture

- **Status:** Accepted (Phase 0)
- **Authors:** GeneGIS core team
- **Created:** 2026-06-12

## Summary

GeneGIS is a new category of geospatial platform: an **AI-native GIS workbench** with cloud-native data, GPU rendering, collaboration, and extensibility — explicitly **not** a QGIS or ArcGIS clone.

## Motivation

Existing GIS tools optimize for expert manual operation. GeoAI-era workflows need:

1. Natural-language intent → verified spatial workflows
2. Partial reads from cloud-optimized formats (COG, GeoParquet, COPC, PMTiles)
3. GPU-first rendering at scale
4. Reproducibility, provenance, and collaboration by default

## Key decisions

### D1: Rust core

GIS Core, engines, renderer, server, and CLI share a Rust codebase for safety, performance, and WASM portability.

### D2: Workflow graph as IR

All analysis — whether triggered by UI, AI, or CLI — is represented as a **GeoWorkflow** DAG before execution.

### D3: Command bus unification

UI clicks, AI actions, and CLI invocations emit the same `Command` type for undo, audit, and replay.

### D4: wgpu rendering

Primary graphics abstraction is **wgpu** (WebGPU-compatible), not raw Vulkan-first.

### D5: Tauri desktop + TS UI

Desktop shell uses Tauri; rich IDE-like UX uses TypeScript. egui reserved for internal tools.

### D6: Data stack

| Role | Technology |
|------|------------|
| Local analytics | DuckDB Spatial |
| Cloud/lake vectors | GeoParquet |
| Enterprise DB | PostGIS |
| Legacy interchange | Shapefile, GeoJSON (limited) |

### D7: Plugin sandbox

WASM plugins (default), TS UI extensions, Python analysis sandboxes. Capability-based permissions.

### D8: MVP north star

One-prompt demo: **「名古屋市の人口密度を表示」** — data discovery through choropleth with citations and workflow visibility.

## Non-goals

- QGIS UI / Processing Toolbox parity
- ArcGIS proprietary stack replication
- Full autonomous GIS in MVP
- Marketplace before SDK stabilizes

## Repository structure

See Section 10 of the master architecture document and `GeneGIS/crates/`.

## Phased delivery

| Phase | Focus |
|-------|-------|
| 0 | Manifesto, RFC, crate skeleton, canvas prototype |
| 1 | MVP Nagoya demo, DuckDB, basic render |
| 2 | COG, PMTiles, STAC, benchmarks |
| 3 | GPU massive rendering, COPC alpha |
| 4 | Plugin SDK + marketplace beta |
| 5 | Collaboration (CRDT), GeneGIS Server |
| 6 | Multi-agent autonomous GIS |

## Open questions

- CRDT choice: Automerge vs Yjs for project metadata sync
- Local LLM packaging for air-gapped deployments
- Native plugin trust model for enterprise hardware integrations

## References

- Master Architecture Design Document (2026-06-12)
- Autonomous GIS research (self-generating / verifying workflows)
- OGC GeoParquet, COG, COPC, PMTiles, STAC specifications
