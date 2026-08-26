# GeneGIS

**Verified spatial workflows from intent to map.**

AI-native · Cloud-native · GPU-native · Open source

## Feature showcase

<table>
  <tr>
    <td width="50%" align="center">
      <img src="docs/assets/feature-cloud.gif" alt="Measured multi-format cloud view" width="100%" /><br />
      <sub><strong>Cloud selected view</strong><br />GeoParquet · COG · COPC · PMTiles · wgpu</sub>
    </td>
    <td width="50%" align="center">
      <img src="docs/assets/feature-adapters.gif" alt="Cross-engine spatial verification" width="100%" /><br />
      <sub><strong>OSS engine verification</strong><br />PostGIS · GRASS · QGIS Processing</sub>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <img src="docs/assets/feature-trust.gif" alt="Map-linked Trust Debugger" width="100%" /><br />
      <sub><strong>Trust Debugger</strong><br />Evidence graph · semantic diff · offline capsule</sub>
    </td>
    <td width="50%" align="center">
      <img src="docs/assets/feature-collab.gif" alt="Live semantic map review" width="100%" /><br />
      <sub><strong>Spatial collaboration</strong><br />Comments · branches · Automerge CRDT</sub>
    </td>
  </tr>
</table>

Cloud metrics come from a reproducible
[`evidence artifact`](docs/reports/readme-cloud-selected-view.json), not a product claim.

### One prompt, inspectable end to end

<p align="center">
  <img src="docs/assets/workbench-hero.gif" alt="Intent to verified Nagoya population-density map" width="960" />
</p>

<p align="center">
  <a href="https://genegis-playground.rsasaki0109.chatgpt.site"><strong>Open the zero-install Playground →</strong></a>
</p>

## Five flagship use cases, one verified pipeline

Every RFC 0005 application case runs as an agent-executable workflow with a
named verifier and an audit receipt — offline, deterministic, fail-closed:

<p align="center">
  <img src="docs/assets/usecase-showcase.gif" alt="Five verified GeneGIS use cases: density, flood exposure, evacuation delay, 15-minute city, NDVI delta, point-cloud epochs" width="960" />
</p>

The same frames rendered on **real open data** (国交省 A31a flood polygons,
OSM walk network, 名古屋市指定避難所 — see below for the one-command setup):

<p align="center">
  <img src="docs/assets/usecase-showcase-real.gif" alt="The same verified workflows on real open data: census density, MLIT A31a flood exposure, real-shelter evacuation delays on the OSM graph, real-POI 15-minute city" width="960" />
</p>

| Prompt | Workflow | Verifier |
|---|---|---|
| 「名古屋市の人口密度を表示」 | `nagoya-density` | DuckDB re-aggregation |
| 「名古屋市の洪水浸水リスクと人口曝露を表示」 | `nagoya-flood-exposure` | `duckdb_verify` |
| 「名古屋市の洪水浸水リスクと避難所アクセシビリティを表示」 | `nagoya-evacuation-access` | `route_sanity_verify` |
| 「名古屋市の15分都市アクセシビリティを表示」 | `nagoya-xmin-city` | `route_sanity_verify` |
| 「名古屋周辺のNDVI時系列をSentinel-2から作成して検証」 | `sentinel-ndvi-timeseries` | `index_range_verify` |
| 「2時期の点群から建物・植生の変化を抽出して検証」 | `copc-change-detect` | `volume_delta_verify` |

Frames are synthetic offline fixtures, regenerated bit-stable by
`cargo run -p genegis-cli -- demo frames` + `scripts/build-readme-showcase.sh`
(spec: [`docs/rfcs/0005-application-use-cases.md`](docs/rfcs/0005-application-use-cases.md)).

### 3D district exploration

The Phase 14 district view combines point-cloud terrain, LOD1 buildings,
roads, POIs, and a linked dashboard in one deterministic orbit:

<p align="center">
  <img src="docs/assets/district3d.gif" alt="Verified offline 3D district fixture with point-cloud terrain, LOD1 buildings, roads, POIs, and dashboard" width="800" />
</p>

This is a verified offline fixture with a local metric CRS; its sources and
units remain explicit and the frames are reproducible from the workflow output.

## Same prompts, real open data

The UC-1 workflows run unchanged on licensed real data. Fetch, convert and
re-declare checksums (fail-closed receipts stay green only when the declared
sha256 matches the bytes actually read):

```bash
python3 scripts/fetch-real-data.py   # downloads + converts, prints sha256s

export GENEGIS_FLOOD_ZONES_PATH=examples/nagoya-population-density/data/real/nagoya-flood-zones-real.geojson
export GENEGIS_FLOOD_ZONES_SHA=<printed sha256>
export GENEGIS_SHELTERS_PATH=examples/nagoya-population-density/data/real/nagoya-shelters-real.geojson
export GENEGIS_SHELTERS_SHA=<printed sha256>
cargo run -p genegis-cli -- agent run "名古屋市の洪水浸水リスクと避難所アクセシビリティを表示"
```

Sources: 国土数値情報 洪水浸水想定区域（河川単位）A31a 愛知県 想定最大規模
(MLIT, CC-BY 4.0) clipped to the Nagoya bbox, and 名古屋市指定避難所
(令和7年8月時点, via BODIK CKAN, CC-BY 4.0). On the 2026-08 fetch this
yields 46,856 depth-band polygons (19.4% of street length flooded) and
803 real shelters — 中川区 +16 min, 南区 +11 min, 西区 +10 min around the
Shōnai/Shinkawa corridors on the real OSM graph. **Not a hazard-map
substitute**; always consult official 重ねるハザードマップ outputs for
life-safety decisions.

### Real OSM walk network (UC-1 + UC-4 fully real)

```bash
python3 scripts/fetch-osm-network.py   # Overpass API, © OpenStreetMap contributors (ODbL)

export GENEGIS_WALK_NETWORK_PATH=examples/nagoya-population-density/data/real/nagoya-walk-network-real.geojson
export GENEGIS_WALK_NETWORK_SHA=<printed sha256>
export GENEGIS_POIS_PATH=examples/nagoya-population-density/data/real/nagoya-pois-real.geojson
export GENEGIS_POIS_SHA=<printed sha256>
cargo run --release -p genegis-cli -- agent run "名古屋市の15分都市アクセシビリティを表示"
```

On the 2026-08 fetch: 650k graph nodes / 775k edges / 21,353 km of
walkable OSM highways, 4,218 real POIs (supermarket/clinic/school/park).
UC-4 verifies in ~80 s and UC-1 in ~35 s (`--release`); both keep the
same verifiers as the fixtures.

GeneGIS turns intent into a typed Workflow DAG, executes through open GIS
engines, and returns a map with CRS, units, sources, provenance, and independent
checks. It is a verification workbench—not a QGIS clone.

North-star prompt:

```text
名古屋市の人口密度を表示
```

## Why it is different

- Every operation flows through Command + Workflow Graph.
- GeoParquet, COG, COPC, PMTiles, and STAC use bounded cloud reads.
- PostGIS, GRASS, QGIS Processing, GDAL, and DuckDB remain interoperable workers.
- Trust fails closed and ships as an offline-verifiable evidence capsule.
- Map comments, branches, semantic diff, and provenance support review.

## Run

```bash
# Prompt → verified map
cargo run -p genegis-cli -- ask "名古屋市の人口密度を表示"

# Inspect without executing
cargo run -p genegis-cli -- ask "名古屋市の人口密度を表示" --plan-only

# Rebuild measured README evidence and GIFs
bash scripts/render-readme-hero.sh

# Rebuild the RFC 0005 use-case showcase GIF (synthetic fixtures)
cargo run -p genegis-cli -- demo frames && bash scripts/build-readme-showcase.sh

# Rebuild the Phase 14 M0 3D district orbit
cargo run -p genegis-cli -- demo frames-3d .genegis/frames-3d
bash scripts/build-district3d-gif.sh

# Real-data variant (after fetch-real-data.py / fetch-osm-network.py + env vars)
cargo run --release -p genegis-cli -- demo frames .genegis/frames-real
bash scripts/build-readme-showcase.sh .genegis/frames-real \
  docs/assets/usecase-showcase-real.gif density,flood,evacuation,xmin-city
```

## Architecture

```text
Intent → Command → GeoWorkflow → Verified Execution → Map / Report
                         ↓
       Rust core · OSS adapters · cloud formats · wgpu
```

Core: Rust · UI: TypeScript/Tauri · plugins: Python/WASM · analytics: DuckDB

## Read more

- [Architecture](docs/architecture/overview.md)
- [Master RFC](docs/rfcs/0001-master-architecture.md)
- [Proof-carrying analysis](docs/rfcs/0003-proof-carrying-spatial-analysis.md)
- [Roadmap](docs/roadmap/phase-11-proof-carrying-analysis.md)
- [Contributing](CONTRIBUTING.md)

Apache-2.0 OR MIT
