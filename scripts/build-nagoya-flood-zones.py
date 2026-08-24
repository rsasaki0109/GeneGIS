#!/usr/bin/env python3
"""Generate the synthetic Nagoya flood inundation fixture.

The zones approximate 想定最大規模 river-corridor and coastal-lowland
inundation polygons described by 重ねるハザードマップ / 国土数値情報
for Nagoya, but the geometry is hand-authored for offline demos — it is
NOT survey data. Every feature carries a `depth_class` in metres matching
the official depth bands (0.5 / 3.0 / 5.0+).

Output: examples/nagoya-population-density/data/nagoya-flood-zones.geojson
Deterministic: no randomness, fixed vertex lists.
"""

import json
import pathlib

OUT = (
    pathlib.Path(__file__).resolve().parent.parent
    / "examples/nagoya-population-density/data/nagoya-flood-zones.geojson"
)


def corridor(centerline, half_width_deg):
    """Build a closed polygon strip around a lon/lat centerline."""
    left, right = [], []
    for index, (lon, lat) in enumerate(centerline):
        if index + 1 < len(centerline):
            nlon, nlat = centerline[index + 1]
        else:
            nlon, nlat = centerline[index - 1]
        dlon, dlat = nlon - lon, nlat - lat
        length = (dlon * dlon + dlat * dlat) ** 0.5 or 1.0
        # normal of the direction vector
        nx, ny = -dlat / length, dlon / length
        left.append((lon + nx * half_width_deg, lat + ny * half_width_deg))
        right.append((lon - nx * half_width_deg, lat - ny * half_width_deg))
    ring = left + right[::-1]
    ring.append(ring[0])
    return [ring]


ZONE_DEFS = [
    {
        "zone_id": "shonai-river-lower",
        "name": "庄内川下流域（想定最大規模）",
        "depth_class": 5.0,
        "half_width": 0.006,
        "centerline": [
            (136.905, 35.215), (136.892, 35.185), (136.880, 35.155),
            (136.872, 35.125), (136.867, 35.095), (136.862, 35.072),
        ],
    },
    {
        "zone_id": "shinkawa",
        "name": "新川流域（想定最大規模）",
        "depth_class": 3.0,
        "half_width": 0.004,
        "centerline": [
            (136.930, 35.190), (136.915, 35.160), (136.902, 35.130),
            (136.895, 35.100), (136.888, 35.080),
        ],
    },
    {
        "zone_id": "tempaku-river",
        "name": "天白川流域（想定最大規模）",
        "depth_class": 3.0,
        "half_width": 0.0035,
        "centerline": [
            (136.965, 35.150), (136.955, 35.120), (136.945, 35.090),
            (136.938, 35.065), (136.930, 35.048),
        ],
    },
    {
        "zone_id": "yamazaki-river",
        "name": "山崎川・戸田川流域（計画規模）",
        "depth_class": 0.5,
        "half_width": 0.0028,
        "centerline": [
            (136.960, 35.125), (136.950, 35.105), (136.940, 35.085),
            (136.925, 35.062),
        ],
    },
    {
        "zone_id": "minato-coastal",
        "name": "港区沿岸低地（高潮・内水氾濫）",
        "depth_class": 0.5,
        "polygon": [[
            [136.848, 35.115], [136.862, 35.118], [136.866, 35.095],
            [136.864, 35.070], [136.852, 35.052], [136.842, 35.060],
            [136.844, 35.090], [136.848, 35.115],
        ]],
    },
    {
        "zone_id": "nakagawa-inland",
        "name": "中川区内地（内水氾濫）",
        "depth_class": 0.5,
        "polygon": [[
            [136.878, 35.140], [136.895, 35.142], [136.900, 35.122],
            [136.894, 35.105], [136.882, 35.108], [136.876, 35.126],
            [136.878, 35.140],
        ]],
    },
]


def build_features():
    features = []
    for zone in ZONE_DEFS:
        if "polygon" in zone:
            rings = zone["polygon"]
        else:
            rings = corridor(zone["centerline"], zone["half_width"])
        features.append(
            {
                "type": "Feature",
                "properties": {
                    "zone_id": zone["zone_id"],
                    "name": zone["name"],
                    "depth_class_m": zone["depth_class"],
                    "assumed_scale": "想定最大規模" if zone["depth_class"] >= 3.0 else "計画規模",
                },
                "geometry": {"type": "Polygon", "coordinates": rings},
            }
        )
    return features


def main():
    collection = {
        "type": "FeatureCollection",
        "name": "nagoya-flood-zones",
        "crs": "EPSG:4326",
        "description": "Synthetic offline flood inundation fixture approximating "
        "重ねるハザードマップ depth-band corridors over Nagoya. Not survey data.",
        "reference_sources": [
            "https://disaportaldata.gsi.go.jp/raster/01_flood_l2_shinsuishin_Kuni_data/",
            "https://nlftp.mlit.go.jp/ksj/gml/datalist/KsjTmplt-A32.html",
        ],
        "depth_bands_m": [0.5, 3.0, 5.0],
        "features": build_features(),
    }
    OUT.write_text(json.dumps(collection, ensure_ascii=False, separators=(",", ":")))
    print(f"wrote {OUT} ({len(collection['features'])} zones)")


if __name__ == "__main__":
    main()
