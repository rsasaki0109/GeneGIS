# AGENTS — GeneGIS

## Purpose

GeneGIS is an AI-native, cloud-native, GPU-native open geospatial workbench.
This repository implements the GeneGIS platform — not a QGIS clone.

## Scope

- Rust core engines, TypeScript UI, Python analysis plugins
- Workflow-graph-first GIS architecture
- Cloud-native formats: GeoParquet, COG, COPC, PMTiles, STAC
- AI agent orchestration with verification and provenance

## Operating rules

- Keep the core small; extend via plugins and SDK
- All operations flow through Command + Workflow Graph
- Always record CRS, units, sources, and provenance
- Do not replicate QGIS desktop UI patterns
- Prefer open standards over vendor lock-in
- Minimize scope in each change; match existing conventions

## Delivery standards

- Architecture decisions go in `docs/adr/` or `docs/rfcs/`
- MVP north star: 「名古屋市の人口密度を表示」 one-prompt demo
- Use absolute paths when referencing files in notes
