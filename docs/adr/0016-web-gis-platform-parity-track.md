# ADR 0016: Web GIS Platform Parity Track

- Status: Proposed
- Date: 2026-08-26

## Context

A leading commercial web-GIS SaaS demonstrates the user-facing capability set
the market now expects from a browser-first GIS platform:

1. **3D** — point-cloud terrain tiles, LOD1 extruded buildings with real
   heights, orbit/perspective camera, 3D feature analysis.
2. **Live dashboards** — charts and KPI widgets bound to map layers.
3. **External data connections** — WMS, WFS, and other OGC service layers.
4. **Layer compression and time slider** — temporal animation of layer state.
5. **Desktop-GIS bridge plugin** — export layers from a desktop GIS into the
   web platform for sharing.
6. **Publishing** — share links, embedded maps, story-map style presentation.
7. **Enterprise posture** — on-prem deployment, role-based access, unlimited
   projects.

GeneGIS today covers the data plane well (COG, COPC metadata read, GeoParquet,
PMTiles selection, STAC catalog, workflow graph, proof-carrying verification)
but has no 3D viewer, no live dashboard widgets, no WMS/WFS client, no time
slider widget, and no desktop-GIS bridge plugin. The existing dashboard
deliverable is a PMTiles bundle export, not an interactive analytics surface.

The platform goal is parity on this capability set while keeping the GeneGIS
differentiators: AI-native intent → workflow execution, provenance and
verification on every result, and cloud-native open standards over vendor
lock-in. Parity means matching user-facing capability breadth — not cloning
another product's UI or branding, which stays out of the repository entirely.

## Decision

Open a **platform parity track** (Phase 14) that grows GeneGIS to full
capability parity with the reference web-GIS platform, delivered as six
milestones behind the existing Workflow Graph + Command + provenance model:

| # | Milestone | Capability closed | Reuses |
| --- | --- | --- | --- |
| M0 | 3D district demo GIF | Deterministic offline point-cloud terrain + LOD1 buildings + roads + POI dashboard for a Japanese suburban district, animated as a README showcase | `genegis-analysis`, `resvg`, `build-district3d-gif.sh` |
| M1 | 3D viewer | WebGPU perspective/orbit camera, COPC point rendering, extruded polygon layers, height attribution | `genegis-render`, `apps/workbench` |
| M2 | Live dashboards | Chart widgets (histogram, category breakdown, KPI counters) bound to verified workflow results | `genegis-analysis::dashboard` |
| M3 | OGC service clients | WMS `GetMap`/`GetCapabilities` and WFS `GetFeature` as adapter-backed, receipted sources | `genegis-storage`, adapter manifest |
| M4 | Time slider + layer streaming | Temporal epoch playback over versioned layers; vector-tile streaming with per-layer encoding budgets | NDVI/change epochs, `genegis-tile` |
| M5 | Publishing + desktop bridge | Share-link/embeddable viewer; plugin that pushes desktop-GIS layers into a GeneGIS project | WASM/plugin host, capsule export |

Non-goals: multi-tenant SaaS billing, proprietary format lock-in, replicating
any vendor's visual identity or trademarks.

## Consequences

- The north-star prompt 「名古屋市の人口密度を表示」 and all existing
  verification gates remain unchanged; parity work adds capability breadth,
  never removes evidence requirements.
- Every new source type (WMS/WFS) must enter through the adapter manifest and
  produce I/O receipts like COG/COPC/GeoParquet do today.
- 3D rendering extends `genegis-render` rather than introducing a second
  graphics stack; headless frame capture must stay deterministic enough for
  CI-regenerable showcase assets.
- Repository copy must not name commercial products; docs refer to "the
  reference web-GIS platform".
