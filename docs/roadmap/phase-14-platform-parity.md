# Phase 14: Web GIS Platform Parity

**Status:** Proposed (ADR 0016).

**Goal:** Reach user-facing capability parity with the reference commercial
web-GIS platform — 3D, live dashboards, OGC service layers, time slider,
publishing, and a desktop-GIS bridge — while keeping every result
provenance-bound and verified.

Reference capability analysis and milestone table:
[`docs/adr/0016-web-gis-platform-parity-track.md`](../adr/0016-web-gis-platform-parity-track.md).

## Milestones

| ID | Deliverable | Exit evidence | Status |
| --- | --- | --- | --- |
| `M0` | 3D district showcase GIF | README GIF: point-cloud terrain + LOD1 buildings + roads + POI dashboard for a Japanese suburban district; frames regenerable via one script | Complete (2026-08-26) |
| `M1` | 3D viewer (WebGPU) | Orbit camera over COPC points + extruded buildings with real heights in the workbench; first-frame/FPS budgets under the shared I/O receipt model | Pending |
| `M2` | Live dashboards | Chart widgets driven by verified workflow results (building-height histogram, POI category breakdown, KPI counters); digest-bound to the executed workflow | Pending |
| `M3` | WMS/WFS clients | GetCapabilities/GetMap and GetFeature through adapter manifest with I/O receipts; positive + negative admission tests | Pending |
| `M4` | Time slider + layer streaming | Epoch playback widget over temporal layers; per-layer tile encoding budget evidence | Pending |
| `M5` | Publishing + desktop bridge | Share-link/embeddable viewer export; plugin pushing desktop-GIS layers into a GeneGIS project capsule | Pending |

## M0 first slice (complete)

Recreate the reference 3D district demo end to end as a deterministic,
offline-safe Japanese suburban fixture:

1. **Terrain** — seeded point-cloud grid in a local metric CRS.
2. **Buildings** — seeded LOD1 footprints with generated metric heights,
   extruded in renderer space.
3. **Roads** — deterministic local road grid.
4. **POI** — six deterministic categorized points.
5. **Dashboard strip** — height-band histogram, POI category counts, total
   building count rendered into each frame.
6. **GIF** — orbit-camera frame sequence assembled by
   `scripts/build-district3d-gif.sh` into `docs/assets/district3d.gif`.

Constraints:

- No competitor names anywhere in the repository or generated assets.
- All data sources recorded as citations/provenance in the result capsule.
- Frames must be regenerable offline from pinned fixtures (CI-safe), with an
  optional real-download path behind `GENEGIS_*_PATH` overrides like the
  existing real-data showcase.

The real-data override remains follow-up work and must identify every source,
CRS, unit, and checksum without changing the offline fixture path.

## Out of scope

- Multi-tenant SaaS, billing, per-seat licensing
- Vendor visual identities/trademarks
- Replacing the Workflow Graph execution path with direct-render shortcuts

## North star (unchanged)

「名古屋市の人口密度を表示」 keeps passing offline via rule planner + DuckDB
verification in CI.
