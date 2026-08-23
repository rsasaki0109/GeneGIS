# Nagoya population density demo data

## Files

| File | Description |
|------|-------------|
| `nagoya-population-2020.json` | 2020 census population by ward (16 wards) |
| `nagoya-wards.geojson` | Ward boundaries + population attributes |
| `nagoya-oracle-2020.json` | Immutable independent population/area oracle |
| `nagoya-source-manifest-2020.json` | Immutable source identities, licenses, and checksums |

## Rebuild boundaries

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
