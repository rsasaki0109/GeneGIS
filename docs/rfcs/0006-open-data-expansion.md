# RFC 0006: Open-Data Expansion

- **Status:** Accepted for implementation — **OD-1 landed** (2026-09-12);
  **OD-2–OD-4 real-data fetchers landed** (2026-09-12); **OD-2 scene override
  plumbing, OD-3 multimodal graph, OD-4 JMA live-feed adapter landed**
  (2026-09-12).
- **Date:** 2026-09-11
- **Scope:** Replace synthetic fixtures with licensed Japanese open data across
  population density, 3D terrain/buildings, transit accessibility, and live
  weather, without weakening the fail-closed verification boundary.

## Decision

GeneGIS extends its real-data program from the four landed sources — MLIT A31a
flood zones, 名古屋市指定避難所, the OSM walk network/POIs, and 令和2年国勢調査
ward population — to four additional open-data domains. Every domain ships with
the same contract as the landed sources:

1. a deterministic `scripts/fetch-*.py` fetch+convert path;
2. a catalog record in `crates/genegis-catalog/src/catalog.rs` with license,
   CRS, bbox, checksum, and `source_version`;
3. an immutable source manifest (source URL, license, `sha256`, retrieval
   basis, scope statement);
4. an env-var `*_PATH` / `*_SHA` override so the workflow runs unchanged on real
   data while receipts stay fail-closed;
5. a named verifier and negative/mutation tests;
6. a `docs/reports/` evidence JSON recording measured real-data numbers.

The offline synthetic fixtures remain the CI baseline. Real data is an
opt-in, checksum-declared path, exactly as RFC 0005 §"Same prompts, real open
data" already establishes.

## Problems addressed

The north star and the 3D/live horizons still depend on synthetic inputs in
four places:

- population density is ward-level (16 units), too coarse for a real choropleth;
- the 3D district and city-scale scene use a synthetic point cloud and generated
  LOD1 heights rather than observed terrain and buildings;
- the 15-minute-city score is walk-only, ignoring the rail/bus network that
  actually defines accessibility in Nagoya;
- the live-feed adapter (H3.1) has no real, continuously updating provider
  behind it, so freshness/cursor semantics are exercised only on fixtures.

## Data sources

| ID | Domain | Source | License |
|----|--------|--------|---------|
| OD-1 | Population mesh | [e-Stat 国勢調査 地域メッシュ統計 (500m)](https://www.e-stat.go.jp/gis) | 政府標準利用規約（e-Stat利用規約） |
| OD-2a | Terrain | [GSI 基盤地図情報 標高 / 標高タイル](https://cyberjapandata.gsi.go.jp/xyz/dem5a_png/) | 国土地理院コンテンツ利用規約（政府標準利用規約準拠） |
| OD-2b | Buildings | [PLATEAU 名古屋市 3D都市モデル 2022](https://www.geospatial.jp/ckan/dataset/plateau-23100-nagoya-shi-2022) | CC BY 4.0（PLATEAU オープンデータ） |
| OD-3 | Transit | [国土数値情報 N02 鉄道 / N07 バス](https://nlftp.mlit.go.jp/ksj/) | 国土数値情報（政府標準利用規約） |
| OD-4 | Live weather | [気象庁 AMeDAS / 気象警報・注意報](https://www.jma.go.jp/bosai/) | 気象庁ホームページ利用規約（政府標準利用規約準拠） |

All sources are reachable without credentials; e-Stat and GSI were verified
reachable on 2026-09-11. Where a source requires a stable snapshot for
verification, the converter pins a named release and records its `sha256`.

## Specifications

### OD-1: 500m population mesh

- **Fetch:** `scripts/fetch-estat-mesh.py` downloads the 令和2年国勢調査 500m
  地域メッシュ (population total) for the Nagoya bbox and emits
  `examples/nagoya-population-density/data/real/nagoya-population-mesh-real.geojson`
  with `mesh_id`, `population`, and polygon geometry in `EPSG:4326`.
- **Aggregation:** `crates/genegis-analysis/src/nagoya.rs` gains a mesh input
  path that aggregates mesh population onto N03 ward polygons by area-weighted
  point-in-polygon, then runs the existing density pipeline unchanged.
- **Verifier:** the immutable `nagoya-oracle-2020.json` ward totals and
  `density_oracle` check are reused; mesh→ward totals must match within the
  existing 0.5% ward-area tolerance. Missing or duplicate mesh cells fail closed.
- **Env vars:** `GENEGIS_POPULATION_MESH_PATH`, `GENEGIS_POPULATION_MESH_SHA`.
- **Acceptance:** mesh aggregation reproduces the ward oracle within tolerance;
  dropping a mesh cell changes the total and fails the oracle check.

### OD-2: Real DEM and PLATEAU buildings

- **Fetch:** `scripts/fetch-gsi-dem.py` mosaics GSI 5m/10m DEM tiles over the
  Nagoya district bbox into `nagoya-dem-real.tif` (COG, `EPSG:6675`); and
  `scripts/fetch-plateau.py` converts PLATEAU 名古屋市 LOD1/LOD2 building
  footprints+heights into `nagoya-buildings-real.geojson` in the same local
  metric CRS as the existing scene fixture.
- **Scene:** `crates/genegis-render/src/scene3d.rs` and the city-scene planner
  gain `GENEGIS_DEM_PATH` / `GENEGIS_BUILDINGS_PATH` overrides. Terrain height
  sampling and building extrusion read the real assets; the synthetic
  `nagoya-scene.copc.laz` / `nagoya-scene-lod1.json` remain the CI default.
- **Verifier:** CRS/units/source-snapshot checks are unchanged; the existing
  GPU first-frame/FPS budgets (Phase 14 M1) must still pass on the real assets.
- **Acceptance:** the same frame-plan workflow produces a digest-bound scene
  from real DEM+buildings, and the M1 hardware receipt is regenerated.

### OD-3: Transit accessibility (N02/N07)

- **Fetch:** `scripts/fetch-mlit-transit.py` converts N02 rail lines/stations
  and N07 bus routes/stops to `nagoya-transit-real.geojson` (stops as nodes,
  lines as ride edges, with operator and line identity).
- **Graph:** `crates/genegis-network` gains a multimodal layer: walk edges plus
  ride edges with per-line headway/wait and transfer penalties. The existing
  walk-only graph remains the default.
- **Analysis:** `crates/genegis-analysis/src/accessibility.rs` adds a transit
  mode that composes walk-access → ride → egress, reusing the existing
  cumulative-opportunity and nearest-cost measures.
- **Verifier:** `route_sanity_verify` (triangle inequality sampling, threshold
  monotonicity) is reused; transit rides must not be faster than straight-line
  distance allows, and a missing transfer penalty fails closed.
- **Acceptance:** walk mode is backward compatible; transit mode changes the
  15-minute reachable set and passes the same route-sanity checks.

### OD-4: Live weather (JMA)

- **Fetch/adapter:** a JMA provider behind the existing
  `genegis-adapter::LiveFeedAdapter` contract reads AMeDAS observation JSON
  (temperature, precipitation, wind) and 気象警報・注意報, with an explicit
  provider revision and evaluation time.
- **Live path:** `crates/genegis-analysis/src/live_feed.rs` ingestion and
  `verified_alert.rs` evaluation run on real observations with cursor/watermark
  and freshness policy; the deterministic offline provider stays the CI default.
- **Verifier:** existing cursor/watermark monotonicity, staleness, and
  threshold/z-score rules are unchanged. An LLM judgement can never emit an
  alert (H3 gate).
- **Acceptance:** a real AMeDAS page ingests to an immutable snapshot, advances
  the watermark, and can trigger a deterministic threshold alert; stale or
  watermark-regressing pages fail closed.

## Platform gaps (new work items)

1. e-Stat/GSI/PLATEAU/JMA converters currently do not exist; each needs a
   pinned-release fetch path and a deterministic output digest. — **Landed
   2026-09-12**: `fetch-estat-mesh.py`, `fetch-gsi-dem.py`, `fetch-plateau.py`,
   `fetch-mlit-transit.py`, `fetch-jma-amedas.py` all run against real sources.
   OD-1 also lands the offline mesh fixture, catalog record, mesh→ward density
   executor (`run_nagoya_population_density_mesh`), CLI command
   (`genegis workflow run nagoya-population-mesh`), and the
   `population_mesh_conserves_the_ward_oracle` test with all four oracle checks
   passing.
2. `genegis-network` has no ride-edge/transfer model; multimodal Dijkstra is new.
   — **Landed 2026-09-12**: `TransitGraph` (walk + stops + ride corridors with
   headway wait and transfer penalty), `load_transit_corridors`,
   `run_nagoya_accessibility_with_transit`, synthetic `nagoya-transit` fixture +
   manifest + catalog record, CLI `genegis workflow run nagoya-xmin-city-transit`.
   Route sanity uses the physical floor (straight-line / 30 km/h) since transit
   legitimately beats walk speed. Real N02/N07 fetch landed in `fetch-mlit-transit.py`.
3. `genegis-render` scene inputs are compiled-in fixture paths; env-var override
   plumbing is new. — **Landed 2026-09-12**: `nagoya_scene_copc_path` /
   `nagoya_scene_lod1_path` in `genegis-catalog` honour `GENEGIS_COPC_PATH` /
   `GENEGIS_BUILDINGS_PATH`, and the `gpu_scene_acceptance` binary resolves the
   scene paths from those env vars before falling back to the fixture manifest.
   Digests must be re-declared so receipts stay fail-closed. The real DEM/buildings
   fetch landed in `fetch-gsi-dem.py` / `fetch-plateau.py`.
4. The live-feed adapter is provider-neutral but has no real HTTP provider wired
   into the Workbench/Server surfaces. — **Landed 2026-09-12**: `LiveFeedAdapter`
   gains a GET path (`execute_get`) and a dedicated AMeDAS path
   (`execute_amedas`) that converts the real 気象庁 `map/{time}.json` payload via
   `amedas_map_to_page` into the shared cursor/watermark/freshness contract.
   `execute_jma_live_feed_workflow` runs it through Command + Workflow, and
   `genegis live amedas` fetched a live Nagoya observation (26.2°C) with
   `fresh: true` on 2026-09-12.

## Non-goals

- Real-time hydrodynamic or dispersion simulation (adapter scope).
- Learned building/land-cover segmentation (plugin scope).
- Shipping licensed source bytes into the repository; large real assets stay in
  the gitignored `.genegis/real-data` cache with declared checksums.
- Replacing official hazard or evacuation guidance.

## Verification matrix additions (target)

| Workflow | Execute | Verifier | Offline? |
|----------|---------|----------|----------|
| `nagoya-density` (mesh) | `run_nagoya_population_density` | `density_oracle` | Fixture yes; mesh via env vars |
| city-scale 3D / district | scene frame plan | source snapshot + GPU budget | Fixture yes; DEM/PLATEAU via env vars |
| `nagoya-xmin-city` (transit) | `run_accessibility_score` | `route_sanity_verify` | Fixture yes; transit via env vars |
| live weather | `live_feed_ingest` | cursor/watermark + alert rules | Fixture yes; JMA via adapter |

## References

- Internal: `docs/rfcs/0005-application-use-cases.md`,
  `docs/roadmap/long-term-product-roadmap.md`,
  `examples/nagoya-population-density/data/README.md`,
  `scripts/fetch-real-data.py`, `scripts/fetch-osm-network.py`.
- External: e-Stat GIS (地域メッシュ統計), GSI 基盤地図情報/標高タイル,
  PLATEAU (国土交通省 Project PLATEAU), 国土数値情報 N02/N07, 気象庁防災情報 JSON.
