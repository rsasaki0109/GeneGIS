# RFC 0005: Application Use Case Portfolio

- **Status:** Draft — **All five use cases landed** as of 2026-08-24 (offline fixture cores). UC-2 core (`genegis-tile` PMTiles writer + MVT encoder, `dashboard-export-demo`, `tile_roundtrip_verify`). UC-1 complete: overlay part (`nagoya-flood-exposure` grid-sampling workflow, `run_flood_exposure` + `duckdb_verify`) and network evacuation part (`nagoya-evacuation-access`, flood-penalized Dijkstra via `scale_edge_costs`, `run_evacuation_access` + `route_sanity_verify`, shelter fixture placed outside flood zones). UC-4 core (`genegis-network` walk-graph crate, `nagoya-xmin-city`, `run_accessibility_score`). UC-3 offline core (`sentinel-ndvi-timeseries`: STAC discovery → COG band reads → NDVI algebra in `genegis-raster::algebra` → per-ward zonal means, `run_index_timeseries` + `index_range_verify`; real Sentinel-2 HTTP scenes remain adapter scope). UC-5 core (`copc-change-detect`: `genegis-pointcloud` XYZ reader for LAS + COPC streaming, per-cell p90 nDSM diff with geometric thresholds, `run_copc_change` + `volume_delta_verify`, control-quadrant + exact-NN stability checks)
- **Date:** 2026-08-24
- **Scope:** Flagship application cases that exercise the platform end-to-end after Phase 9–10 (cloud-native formats, federated catalog) and during Phase 12–13 (operational verification workbench)

## Decision

GeneGIS will develop five flagship application cases, each shipped as an
agent-executable workflow with a named verifier and a recorded audit bundle,
extending the existing verification-matrix pattern of Phase 9:

| # | Use case | One-prompt demo | Primary crates |
|---|----------|-----------------|----------------|
| UC-1 | Disaster response overlay & evacuation accessibility | 「名古屋市の洪水浸水リスクと避難所アクセシビリティを表示」 | vector, raster, spatial-index |
| UC-2 | Municipal map dashboard with provenance receipt | 「名古屋市の人口密度ダッシュボードを監査証跡付きで公開用に書き出し」 | tile, catalog, capsule |
| UC-3 | Environmental monitoring time series (STAC → index) | 「名古屋周辺のNDVI時系列をSentinel-2から作成して検証」 | catalog, raster, storage |
| UC-4 | Urban accessibility (X-minute city) | 「名古屋駅から15分で到達できる施設を可視化してスコア化」 | geometry, spatial-index, adapter |
| UC-5 | Point cloud change detection (multi-epoch) | 「2時期の点群から建物・植生の変化を抽出して検証」 | pointcloud, render |

All cases remain subject to the platform invariants of RFC 0004: Command +
Workflow Graph only, CRS/provenance always recorded, fail-closed verification,
and the offline no-LLM Nagoya path kept green in CI.

## External landscape (research summary, 2026)

1. **AI geospatial agents are converging on orchestration + verification.**
   Microsoft's GeoFaham (AutoGen multi-agent over Planetary Computer/PostGIS)
   and HASTE show demand for natural-language disaster analysis; academic work
   (GeoContra, OpenEarthAgent, NORA, GeoAgent) converges on contract-checked,
   provenance-complete, replay-verifiable workflows. GeneGIS's GeoContract +
   Verification Graph + audit bundle is the same thesis applied natively in
   Rust; these systems validate the direction rather than compete with it.
2. **Disaster rapid mapping is a solved *pattern*, not a solved product.**
   DLR-ZKI publishes flood masks plus building footprints into a web viewer
   within hours. Japan exposes the raw inputs openly: 重ねるハザードマップ
   flood/sediment/tsunami raster tiles (`disaportaldata.gsi.go.jp`, GSI tile
   spec), GSI 指定緊急避難場所・指定避難所 CSV/GeoJSON, e-Stat small-area
   census polygons, 国土数値情報 flood inundation polygons. Published studies
   (Hiroshima evacuation-time hazard maps; Hamamatsu 2SFCA shelter equity)
   define exactly the analytics users ask for.
3. **Offline-first browser GIS is viable and wanted.** FloodGraph/NeerNet
   (FOSS Hack 2026) runs MapLibre + PMTiles + Pyodide/NetworkX fully client-side
   with service-worker region packs — confirming PMTiles as the right delivery
   format for UC-2.
4. **Accessibility analytics have converged on five measures** (accessX):
   cumulative opportunities, nearest-facility cost, Hansen potential, 2SFCA
   supply-demand, co-accessibility. Rust implementations exist (graphways via
   pyo3), showing Rust is a credible home for network analytics.
5. **Point cloud change detection is maturing** (Urb3DCD-v2 object-based
   methods ~82 mIoU; national ALS biomass time series; UAV-LiDAR restoration
   monitoring). Multi-epoch COPC + DSM differencing is an achievable v0;
   learned segmentation is out of scope for MVP.

## Use case specifications

### UC-1: Disaster response overlay & evacuation accessibility

- **Inputs:** 重ねるハザードマップ flood tiles (or 国土数値情報 polygon
  variant), GSI evacuation-point GeoJSON, e-Stat population grid/polygons,
  OSM road network extract for Nagoya.
- **Workflow:** import → reproject to JGD2011 / EPSG:6678 (Nagoya local) →
  raster zonal exposure join (population × inundation depth class) →
  network accessibility from population centroids to shelters with
  flood-penalized edge costs (speed × depth factor) → choropleth + isochrone
  render.
- **Verifier:** DuckDB re-aggregation of exposed population totals;
  feature-count verify per layer; isochrone area sanity bounds.
- **Platform gap:** network routing/isochrone primitive (see Platform gaps).

### UC-2: Municipal dashboard with provenance receipt

- **Inputs:** any verified analytical output (e.g., nagoya-density result).
- **Workflow:** style binding → PMTiles export → static viewer bundle →
  attach audit bundle v3 snapshot + source attribution manifest.
- **Verifier:** round-trip tile read-back equality on sampled tiles; audit
  bundle schema validation; attribution presence check.
- **Evidence:** Fukuoka/Kumamoto/Yamaguchi prefecture-city data platforms and
  Kobe Data Lab demonstrate municipal demand; none ship machine-checkable
  provenance receipts — this is GeneGIS's differentiator.

### UC-3: Environmental monitoring time series

- **Inputs:** Sentinel-2 L2A COGs discovered via federated STAC search
  (Phase 10 capability).
- **Workflow:** STAC item fetch → band stack (range-read, no whole-object
  download) → NDVI/NDBI computation → temporal aggregation → zonal stats per
  ward → chart + map render.
- **Verifier:** index value range checks ([-1,1]), pixel-count reconciliation
  against COG metadata, deterministic recomputation on a sampled window.
- **Note:** mirrors Project Atlantis (STAC/Zarr ML-ready flood archive) and
  GeoFaham index pipelines; our angle is range-read evidence per RFC 0004
  invariant 6.

### UC-4: Urban accessibility (X-minute city)

- **Inputs:** OSM POIs + road graph for Nagoya, e-Stat population for weights.
- **Workflow:** build routable graph → POI categorization → compute cumulative
  opportunity + nearest-cost (+ optional 2SFCA) per grid/hex cell → score
  surface render with threshold bands (5/10/15 min).
- **Verifier:** route-distance ≥ straight-line distance on samples;
  reachable-set monotonicity in threshold; total POI count reconciliation.
- **Evidence:** accessX measure set adopted as our analytic contract;
  Transportationer/Valhalla precompute-per-grid pattern adopted for scale.

### UC-5: Point cloud change detection (multi-epoch)

- **Inputs:** two COPC epochs over the same AOI (e.g., GSI airborne LiDAR).
- **Workflow:** load/tile COPC → per-cell height statistics → DSM/DTM grid
  derivation → epoch difference → classify change (building added/removed,
  vegetation growth/removal) by height + prior classification → 3D + 2D render.
- **Verifier:** volume-change sign consistency between epochs; sampled M3C2
  distance spot checks; unchanged-control-area delta ≈ 0.
- **Note:** geometric/threshold v0 per Urb3DCD literature; ML segmentation is
  a plugin-level extension, not core.

## Prioritization

| Wave | Cases | Rationale |
|------|-------|-----------|
| 1 | UC-2, UC-1 (overlay part) | Mostly existing capability; ships visible demos fast; strengthens north star |
| 2 | UC-3, UC-4 | Requires raster algebra + routing primitives; highest differentiation once landed |
| 3 | UC-1 (network evacuation), UC-5 | Depends on Wave 2 primitives; heaviest compute |

## Platform gaps (new work items)

1. ~~`genegis-network`: routable graph build, shortest path, isochrone~~
   **Fully landed 2026-08-24**: GeoJSON LineString graph loading with
   shared-vertex merge, nearest-node snapping, Dijkstra travel times,
   cumulative-opportunity counting, hazard-aware `scale_edge_costs`
   (speed × depth factor), native route-sanity checks and convex-hull
   isochrones (`isochrone()` + shoelace area, monotone in threshold; concave
   alpha-shapes deferred until a consumer needs them). OSM PBF ingestion
   remains open as gap #4.
2. Raster algebra + zonal statistics in `genegis-raster` — **partially
   landed 2026-08-24**: `algebra::{ndvi, mean}` clamped index math plus
   pixel-center zonal means in analysis; reclassify and general zonal agg
   remain open (UC-5 DSM differencing will need them).
3. ~~PMTiles writer + static bundle export in `genegis-tile`.~~ **Landed
   2026-08-24**: clustered single-root v3 writer, MVT polygon encoder with
   Sutherland–Hodgman clipping, gzip metadata/tiles verified interoperable with
   the official Python `pmtiles` reader.
4. OSM PBF/Overpass ingestion adapter producing typed GeoParquet with full
   provenance (source URL, extraction timestamp, license tag).
5. ~~COPC epoch diff operations in `genegis-pointcloud`.~~ **Landed
   2026-08-24**: uniform `read_point_cloud_path` over LAS and COPC-streaming
   sources, shared-grid height statistics, threshold classification and sign
   reconciliation. Learned segmentation remains plugin scope.

## Non-goals

- Real-time hazard simulation physics (hydrodynamic modeling) — delegate via
  adapters.
- Trained deep-learning segmentation/change models in core — plugin scope.
- Replacing municipal BI dashboards generally; we ship the map + receipt layer.

## Verification matrix additions (target)

| Workflow | Execute tool | Verifier | Offline? |
|----------|--------------|----------|----------|
| `nagoya-flood-exposure` | `run_flood_exposure` | `duckdb_verify` | Landed |
| `nagoya-evacuation-access` | `run_evacuation_access` | `route_sanity_verify` | Landed |
| `dashboard-export-demo` | `run_pmtiles_export` | `tile_roundtrip_verify` | Landed |
| `sentinel-ndvi-timeseries` | `run_index_timeseries` | `index_range_verify` | Yes (fixture COGs; real scenes via adapter) |
| `nagoya-xmin-city` | `run_accessibility_score` | `route_sanity_verify` | Landed |
| `copc-change-detect` | `run_copc_change` | `volume_delta_verify` | Landed |

## References

- External: GeoFaham, HASTE (Microsoft); DLR-ZKI Southern Germany 2024 flood
  rapid mapping (Applied Sciences record); Project Atlantis (ECMWF Code for
  Earth); GeoContra (arXiv 2605.00782); NORA (arXiv 2605.02092);
  OpenEarthAgent (arXiv 2602.17665); accessX; graphways; Urb3DCD-v2 studies;
  重ねるハザードマップ オープンデータ (disaportal.gsi.go.jp);
  GSI 指定緊急避難場所データ (gsi.go.jp/bousaichiri/hinanbasho.html);
  e-Stat; 国土数値情報.
- Internal: `/home/sasaki/workspace/GeneGIS/docs/rfcs/0003-proof-carrying-spatial-analysis.md`,
  `/home/sasaki/workspace/GeneGIS/docs/rfcs/0004-operational-verification-workbench.md`,
  `/home/sasaki/workspace/GeneGIS/docs/roadmap/phase-9-external-data.md`,
  `/home/sasaki/workspace/GeneGIS/docs/guides/agent-orchestration.md`.
