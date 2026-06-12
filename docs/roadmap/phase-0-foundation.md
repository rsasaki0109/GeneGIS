# Phase 0: Foundation / Manifesto

**Goal:** Establish GeneGIS vision, OSS expectations, and initial codebase skeleton.

**Star target:** 0 → 300

## Deliverables

- [x] Public repository structure
- [x] README manifesto ("Not a QGIS clone")
- [x] Architecture RFC ([0001](../rfcs/0001-master-architecture.md))
- [x] Initial Cargo workspace + crate skeletons
- [x] `genegis-core` project/command model
- [x] `genegis-workflow` GeoWorkflow IR + Nagoya template
- [x] WebGPU canvas prototype (`genegis-render`)
- [x] CLI: `genegis workflow run nagoya-density`
- [ ] GeoJSON viewer demo (Phase 1)
- [ ] Roadmap GitHub issue board

## Key message

> If GIS were invented in 2026, it would not look like a 2000s desktop app.

## Next: Phase 1 MVP Demo

- Tauri desktop alpha
- GeoJSON / Shapefile / GeoParquet read
- DuckDB query panel
- Choropleth rendering
- AI prompt → workflow graph execution
- PNG / HTML export

See [Phase 1 draft](phase-1-mvp.md).
