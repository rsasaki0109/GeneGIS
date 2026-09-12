# GeneGIS scripts

Deterministic builders for offline fixtures and fetchers for licensed real
open data. Real fetchers write to `examples/nagoya-population-density/data/real/`
(regenerable, git-ignored) and print the `sha256` of each output so receipts can
re-declare the checksum.

## Offline fixture builders (committed, CI-safe)

| Script | Output | Source |
|---|---|---|
| `build-nagoya-wards.py` | `data/nagoya-wards.geojson` | MLIT N03 via JapanCityGeoJson |
| `build-nagoya-population-mesh.py` | `data/nagoya-population-mesh.geojson` | Synthetic, conserves 2020 ward totals |
| `build-nagoya-flood-zones.py` | `data/nagoya-flood-zones.geojson` | Synthetic hazard fixture |
| `build-nagoya-shelters.py` | `data/nagoya-shelters.geojson` | Synthetic shelter fixture |
| `build-nagoya-walk-network.py` | `data/nagoya-walk-network.geojson` | Synthetic walk grid |
| `build-nagoya-transit.py` | `data/nagoya-transit.geojson` | Synthetic rail corridors (multimodal) |

## Real open-data fetchers (opt-in, regenerable)

| Script | Output (in `data/real/`) | Source | License |
|---|---|---|---|
| `fetch-real-data.py` | `nagoya-flood-zones-real.geojson`, `nagoya-shelters-real.geojson` | 国土数値情報 A31a, BODIK CKAN | CC BY 4.0 / 政府標準利用規約 |
| `fetch-osm-network.py` | `nagoya-walk-network-real.geojson`, `nagoya-pois-real.geojson` | OpenStreetMap / Overpass | ODbL |
| `fetch-estat-mesh.py` | `nagoya-population-mesh-real.geojson` | e-Stat 地域メッシュ統計 500m | 政府標準利用規約 |
| `fetch-gsi-dem.py` | `nagoya-dem5a-real.geojson` | GSI 標高タイル DEM5A | 国土地理院コンテンツ利用規約 |
| `fetch-plateau.py` | `nagoya-buildings-real.geojson` | PLATEAU 名古屋市 3D都市モデル | CC BY 4.0 |
| `fetch-mlit-transit.py` | `nagoya-transit-real.geojson` | 国土数値情報 N02 鉄道 / N07 バスルート | 政府標準利用規約 |
| `fetch-jma-amedas.py` | `nagoya-amedas-live.geojson` | 気象庁 AMeDAS | 気象庁利用規約 |

`fetch-gsi-dem.py` accepts `GENEGIS_DEM_BBOX=west,south,east,north` to fetch a
district-level subset (the full Nagoya bbox at z15 is ~840 tiles / ~280 MB).

Each fetcher prints `sha256:<digest>  <path>`. Run the corresponding workflow
with the matching `GENEGIS_*_PATH` / `GENEGIS_*_SHA` env vars so receipts stay
fail-closed (see the project README and RFC 0006).

For the live weather path, the adapter is wired directly:

```bash
GENEGIS_REMOTE_ALLOWED_HOSTS=www.jma.go.jp genegis live amedas
```