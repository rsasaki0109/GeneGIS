# Nagoya population density demo data

## Files

| File | Description |
|------|-------------|
| `nagoya-population-2020.json` | 2020 census population by ward (16 wards) |
| `nagoya-wards.geojson` | Ward boundaries + population attributes |

## Rebuild boundaries

Ward polygons come from **国土数値情報 N03** via [JapanCityGeoJson](https://github.com/niiyz/JapanCityGeoJson):

```bash
python3 GeneGIS/scripts/build-nagoya-wards.py
```

This downloads `geojson/23/23101.json` … `23116.json` and merges population attributes.

## Sources

- Boundaries: MLIT 国土数値情報 N03 行政区域
- Population: e-Stat 2020年国勢調査
- Reference: 名古屋市オープンデータカタログ

## Note

Area/density uses `planar_wgs84_approx` (recorded in verification panel). Not geodesic.
