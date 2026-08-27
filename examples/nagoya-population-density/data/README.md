# Nagoya population density demo data

## Files

| File | Description |
|------|-------------|
| `nagoya-population-2020.json` | 2020 census population by ward (16 wards) |
| `nagoya-wards.geojson` | Ward boundaries + population attributes |
| `nagoya-oracle-2020.json` | Immutable independent population/area oracle |
| `nagoya-source-manifest-2020.json` | Immutable source identities, licenses, and checksums |
| `nagoya-sentinel-{red,nir}-2025-{04,10}.tif` | Deterministic synthetic COG epochs for NDVI verification |
| `nagoya-change-epoch-{a,b}.las` | Deterministic synthetic LAS epochs for classified change verification |
| `nagoya-scene.copc.laz` | Deterministic EPSG:6675 COPC fixture for hardware-bound 3D acceptance |
| `nagoya-scene-lod1.json` | LOD1 buildings sharing the COPC fixture's CRS and metric axes |
| `nagoya-scene-fixture-manifest.json` | COPC/LOD1 hashes, provenance, toolchain, bounds, and acceptance budgets |

## Rebuild boundaries

Synthetic NDVI and point-cloud fixtures are committed so a clean offline clone
can run the verified template suite. Their exact generation paths are:

```bash
cargo run -p genegis-raster --example write_ndvi_fixture --offline
cargo run -p genegis-pointcloud --example write_change_fixture --offline
cd tools/pdal && pixi run --manifest-path pixi.toml -- pdal pipeline nagoya-scene-copc.pipeline.json
```

Run the COPC pipeline from `tools/pdal/` because its source and destination
paths are intentionally relative to that directory. The pinned PDAL writer
uses a fixed creation date, seed, and thread count; two consecutive generation
runs must produce the digest declared by `nagoya-scene-fixture-manifest.json`.

They are test evidence, not observations of real Sentinel-2 or survey data.

Ward polygons come from **国土数値情報 N03** via [JapanCityGeoJson](https://github.com/niiyz/JapanCityGeoJson):

```bash
python3 GeneGIS/scripts/build-nagoya-wards.py
```

This downloads `geojson/23/23101.json` … `23116.json`, validates every source
feature's ward properties, and losslessly merges every polygon part. A source
shell whose purported holes lie outside its exterior (the upstream Minato
snapshot) is normalized to independent polygon parts; valid holes remain
holes and are subtracted by the area engine.

## Sources

- Boundaries: [MLIT 国土数値情報 N03 行政区域](https://nlftp.mlit.go.jp/ksj/gml/datalist/KsjTmplt-N03.html),
  processed snapshot via [JapanCityGeoJson](https://github.com/niiyz/JapanCityGeoJson)
- Population: [名古屋市 令和2年国勢調査 確定値](https://www.city.nagoya.jp/shisei/toukei/1003703/1003773/1003809/1034253/1003818.html)
  and the [official Excel table](https://www.city.nagoya.jp/_res/projects/default_project/_page_/001/003/818/toukeihyo.xlsx)
- License: 名古屋市オープンデータ利用規約（政府標準利用規約2.0準拠） and
  国土数値情報の政府標準利用規約
- Independent oracle: `nagoya-oracle-2020.json` (名古屋市資料/GSI面積調)

The exact immutable identities and regeneration rationale are recorded in
[`nagoya-source-manifest-2020.json`](nagoya-source-manifest-2020.json).

## Note

Area/density uses `ellipsoidal_wgs84` (recorded in the verification panel),
with `EPSG:4326` coordinate units (`degrees`) and output area in `km²`.
