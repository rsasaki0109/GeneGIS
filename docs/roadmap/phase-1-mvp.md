# Phase 1: MVP Demo

**Goal:** One-prompt Nagoya population density experience.

**Star target:** 300 → 1,000

## Demo promise

Input: `名古屋市の人口密度を表示`

Output: choropleth map + legend + sources + workflow graph + CRS/units verification.

## Deliverables

- [x] GeoJSON read (`genegis-vector`)
- [x] Nagoya ward demo dataset (simplified geometry, 2020 census population)
- [x] Workflow execution (`genegis-analysis`)
- [x] DuckDB density verification (`genegis-query`)
- [x] Choropleth HTML export
- [x] CLI `--execute` mode
- [x] Real N03 admin boundaries (JapanCityGeoJson / 国土数値情報)
- [x] Local web workbench (`apps/workbench` — browser launcher, verified at `127.0.0.1:7812`)
- [x] Tauri desktop app alpha (`apps/desktop` — builds and `npm run dev` launch verified on Rust 1.94 via `third-party/` patches)
- [x] PNG export

## Out of scope

- QGIS feature parity
- Full topology editing
- Full 3D city models
- Marketplace
- Enterprise auth
