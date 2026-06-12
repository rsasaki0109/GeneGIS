# GeneGIS Architecture Overview

GeneGIS is an AI-native, cloud-native, GPU-native geospatial workbench — not a desktop GIS reimplementation.

## Core principle

**The center of GeneGIS is Workflow, not Layer.**

```
Traditional:  Data → Layer → Manual Operation → Map
GeneGIS:      Intent → Discovery → Workflow Graph → Verified Execution → Map
```

## Platform layers

```
Apps (Desktop / Browser / Server / CLI)
  └── UX (Canvas, Command Palette, AI Chat, Workflow Graph)
        └── AI Native Layer (Intent, Planner, Verifier, Provenance)
              └── GIS Kernel
                    ├── Core (project, layers, commands)
                    ├── Spatial / Vector / Raster / PointCloud / Tile
                    ├── Query (DuckDB, PostGIS)
                    ├── Analysis (workflow DAG)
                    └── Rendering (wgpu)
                          └── Data / Storage / Plugin / Collaboration
```

## GIS Core responsibilities

- Project model, layer graph, CRS metadata, units
- Command bus (UI, AI, CLI, plugins → same path)
- Undo/redo, provenance, permissions
- Workflow graph schema

Implemented in `crates/genegis-core` and `crates/genegis-workflow`.

## Data philosophy

| Rule | Meaning |
|------|---------|
| Original data is immutable | Sources are referenced, not silently mutated |
| Derived data is reproducible | Outputs trace back to workflow graphs |
| Cache is disposable | Rebuild from workflow + source |
| Provenance is append-only | Audit trail for AI and humans |

## Cloud-native formats (first-class)

- **Vector:** GeoParquet
- **Raster:** COG (Cloud Optimized GeoTIFF)
- **Point cloud:** COPC
- **Tiles:** PMTiles, MBTiles, MVT
- **Catalog:** STAC, OGC API

## AI native design

AI generates **GeoWorkflow IR** first — not raw code. Every step carries:

- Operation name and parameters
- Expected schema and validation rules
- CRS / unit requirements
- Citations and review status

See [`ai-native.md`](ai-native.md).

## Rendering

wgpu-based pipeline targeting 60fps interactive scenes with:

- Progressive loading for huge datasets
- GPU memory budgets per layer
- LOD for vector, raster tiles, and point clouds

See [`rendering.md`](rendering.md).

## Related documents

- [RFC 0001: Master Architecture](../rfcs/0001-master-architecture.md)
- [Roadmap](../roadmap/phase-0-foundation.md)
- [Plugins](plugins.md) (planned)
- [Cloud](cloud.md) (planned)
