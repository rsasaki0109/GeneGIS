#!/usr/bin/env python3
"""Fetch the REAL 令和2年国勢調査 500m人口メッシュ for the Nagoya bbox.

This converts the licensed e-Stat 地域メッシュ統計 (人口総数) 500m mesh into the
same GeoJSON schema produced by ``scripts/build-nagoya-population-mesh.py``
(``mesh_id`` + ``population`` + ward polygon), so the offline fixture and the
real mesh share one density pipeline and one ward-oracle verifier.

Source:
  - e-Stat 統計地理情報システム 国勢調査 令和2年 4次メッシュ（500mメッシュ）
    https://www.e-stat.go.jp/gis/statmap-search (区域メッシュ, 人口総数)
  - License: e-Stat 利用規約 / 政府標準利用規約（統計データ）
  - The geographic CSV / Shapefile contains one row per 500m mesh cell with a
    population field.  Extraction requires either the interactive GIS download
    or the e-Stat API with an application ID.

Output:
  examples/nagoya-population-density/data/real/nagoya-population-mesh-real.geojson

Wards are assigned by point-in-polygon against the bundled N03 ward fixture.
Usage: python3 scripts/fetch-estat-mesh.py PATH_TO_MESH
"""

from __future__ import annotations

import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
DATA = ROOT / "examples/nagoya-population-density/data"
REAL = DATA / "real"
WARDS_PATH = DATA / "nagoya-wards.geojson"
OUT_PATH = REAL / "nagoya-population-mesh-real.geojson"

# Nagoya bbox (matches catalog records).
NAGOYA = (136.78, 35.02, 137.08, 35.28)


def point_in_ring(x, y, ring):
    inside = False
    for i in range(len(ring)):
        x1, y1 = ring[i]
        x2, y2 = ring[(i + 1) % len(ring)]
        if (y1 > y) != (y2 > y):
            at_x = (x2 - x1) * (y - y1) / (y2 - y1) + x1
            if x < at_x:
                inside = not inside
    return inside


def in_polygon(x, y, polygon):
    if not point_in_ring(x, y, polygon[0]):
        return False
    return not any(point_in_ring(x, y, hole) for hole in polygon[1:])


def ward_of(x, y, ward_rings):
    for code, name, parts in ward_rings:
        for polygon in parts:
            if in_polygon(x, y, polygon):
                return code, name
    return None


def load_ward_rings():
    wards = json.loads(WARDS_PATH.read_text(encoding="utf-8"))
    result = []
    for feature in wards["features"]:
        geom = feature["geometry"]
        polygons = (
            [geom["coordinates"]]
            if geom["type"] == "Polygon"
            else geom["coordinates"]
        )
        result.append(
            (
                feature["properties"]["ward_code"],
                feature["properties"]["ward_name"],
                polygons,
            )
        )
    return result


def main() -> int:
    if len(sys.argv) < 2:
        print(
            "usage: python3 scripts/fetch-estat-mesh.py PATH_TO_MESH\n\n"
            "PATH_TO_MESH is the e-Stat 500m mesh source (CSV with a "
            "KEY_CODE / geometry, or a GeoJSON). The population field and cell "
            "boundary parsing depend on the source format.",
            file=sys.stderr,
        )
        return 2

    source = pathlib.Path(sys.argv[1])
    if not source.is_file():
        print(f"mesh source not found: {source}", file=sys.stderr)
        return 1

    # The converter reads the population field per cell. This implementation
    # is intentionally format-specific: adapt to the actual e-Stat export.
    payload = json.loads(source.read_text(encoding="utf-8"))
    ward_rings = load_ward_rings()

    features = []
    total = 0
    unmatched = 0
    for feature in payload.get("features", []):
        props = feature.get("properties", {})
        population = int(props.get("population") or 0)
        geometry = feature.get("geometry") or {}
        if geometry.get("type") != "Polygon":
            continue
        ring = geometry["coordinates"][0]
        cx = sum(p[0] for p in ring) / len(ring)
        cy = sum(p[1] for p in ring) / len(ring)
        if not (NAGOYA[0] <= cx <= NAGOYA[2] and NAGOYA[1] <= cy <= NAGOYA[3]):
            continue
        ward = ward_of(cx, cy, ward_rings)
        if ward is None:
            unmatched += 1
            continue
        code, name = ward
        mesh_id = str(props.get("mesh_id") or props.get("KEY_CODE") or f"mesh-{len(features)}")
        features.append(
            {
                "type": "Feature",
                "properties": {
                    "mesh_id": mesh_id,
                    "ward_code": code,
                    "ward_name": name,
                    "population": population,
                },
                "geometry": geometry,
            }
        )
        total += population

    REAL.mkdir(parents=True, exist_ok=True)
    collection = {
        "type": "FeatureCollection",
        "name": "nagoya-population-mesh-real",
        "crs": "EPSG:4326",
        "description": (
            "REAL 令和2年国勢調査 500m人口メッシュ clipped to the Nagoya bbox, wards "
            "assigned by point-in-polygon against the N03 ward fixture. Source: "
            "e-Stat 統計地理情報システム 地域メッシュ統計 (政府標準利用規約)."
        ),
        "features": features,
    }
    OUT_PATH.write_text(
        json.dumps(collection, ensure_ascii=False, separators=(",", ":")),
        encoding="utf-8",
    )
    import hashlib

    digest = hashlib.sha256(OUT_PATH.read_bytes()).hexdigest()
    print(f"wrote {OUT_PATH} ({len(features)} cells, {unmatched} skipped)")
    print(f"sha256:{digest}  {OUT_PATH}")
    return 0


if __name__ == "__main__":
    sys.exit(main())