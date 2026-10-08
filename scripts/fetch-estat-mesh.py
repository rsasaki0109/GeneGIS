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
  - The statistical GIS publishes one zipped CP932 CSV per first-level mesh
    without an application ID; ``--download`` fetches it, and the cell squares
    are computed from the standard grid-square codes (same parser as
    ``crates/genegis-toolkit/src/estat.rs``).

Output:
  examples/nagoya-population-density/data/real/nagoya-population-mesh-real.geojson

Wards are assigned by point-in-polygon against the bundled N03 ward fixture.
Usage:
  python3 scripts/fetch-estat-mesh.py --download [--level 500m|250m]
  python3 scripts/fetch-estat-mesh.py PATH_TO_MESH_GEOJSON
"""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import pathlib
import sys
import urllib.request
import zipfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
DATA = ROOT / "examples/nagoya-population-density/data"
REAL = DATA / "real"
WARDS_PATH = DATA / "nagoya-wards.geojson"
OUT_PATH = REAL / "nagoya-population-mesh-real.geojson"

# Nagoya bbox (matches catalog records).
NAGOYA = (136.78, 35.02, 137.08, 35.28)

# 令和2年国勢調査 人口等基本集計 statsId and mesh-code length per level.
LEVELS = {"1km": ("T001140", 8), "500m": ("T001141", 9), "250m": ("T001142", 10)}
DOWNLOAD_URL = (
    "https://www.e-stat.go.jp/gis/statmap-search/data"
    "?statsId={stats_id}&code={first_mesh}&downloadType=2"
)


def mesh_bounds(code):
    """[min_lon, min_lat, max_lon, max_lat] of a standard grid square (JGD2011)."""
    lat = int(code[0:2]) / 1.5
    lon = int(code[2:4]) + 100.0
    dlat, dlon = 2.0 / 3.0, 1.0
    if len(code) >= 6:
        dlat, dlon = dlat / 8.0, dlon / 8.0
        lat += int(code[4]) * dlat
        lon += int(code[5]) * dlon
    if len(code) >= 8:
        dlat, dlon = dlat / 10.0, dlon / 10.0
        lat += int(code[6]) * dlat
        lon += int(code[7]) * dlon
    for quadrant in code[8:]:
        # Quadrants: 1 SW, 2 SE, 3 NW, 4 NE.
        q = int(quadrant)
        if q not in (1, 2, 3, 4):
            raise ValueError(f"invalid quadrant in mesh code {code}")
        dlat, dlon = dlat / 2.0, dlon / 2.0
        if q >= 3:
            lat += dlat
        if q in (2, 4):
            lon += dlon
    return [lon, lat, lon + dlon, lat + dlat]


def first_meshes(bbox):
    out = []
    for p in range(int(bbox[1] * 1.5), int(bbox[3] * 1.5) + 1):
        for u in range(int(bbox[0]) - 100, int(bbox[2]) - 100 + 1):
            out.append(f"{p:02d}{u:02d}")
    return out


def parse_mesh_zip(data, level):
    """Rows of (mesh_code, population) from one zipped e-Stat CSV.

    人口（総数） is published for every cell; secrecy only suppresses the
    breakdown columns, so each cell keeps its own total.
    """
    stats_id, digits = LEVELS[level]
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        text = archive.read(archive.namelist()[0]).decode("cp932")
    lines = text.splitlines()
    header = [h.strip() for h in lines[0].split(",")]
    key = header.index("KEY_CODE") if "KEY_CODE" in header else 0
    total = header.index(f"{stats_id}001")
    rows = []
    for line in lines[2:]:  # second header row: Japanese labels
        cells = line.split(",")
        code = cells[key].strip() if key < len(cells) else ""
        if len(code) != digits:
            continue
        value = cells[total].strip() if total < len(cells) else ""
        population = 0 if value == "-" else (int(value) if value.isdigit() else None)
        if population is None:
            raise ValueError(f"mesh {code}: population total is not published ({value!r})")
        rows.append((code, population))
    if not rows:
        raise ValueError("no mesh rows at the requested level")
    return rows


def download_payload(level):
    """Download the first-level mesh files covering Nagoya as a GeoJSON payload."""
    stats_id, _ = LEVELS[level]
    REAL.mkdir(parents=True, exist_ok=True)
    features = []
    sources = []
    for first_mesh in first_meshes(NAGOYA):
        cache = REAL / f"estat_{stats_id}_{first_mesh}.zip"
        url = DOWNLOAD_URL.format(stats_id=stats_id, first_mesh=first_mesh)
        if cache.is_file():
            data = cache.read_bytes()
        else:
            with urllib.request.urlopen(url, timeout=120) as response:
                data = response.read()
            if not data.startswith(b"PK"):
                raise ValueError(f"{url} did not return a zip archive")
            cache.write_bytes(data)
        sources.append(f"{url} sha256:{hashlib.sha256(data).hexdigest()}")
        for code, population in parse_mesh_zip(data, level):
            b = mesh_bounds(code)
            ring = [[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]], [b[0], b[1]]]
            features.append(
                {
                    "type": "Feature",
                    "properties": {"mesh_id": code, "population": population},
                    "geometry": {"type": "Polygon", "coordinates": [ring]},
                }
            )
    for line in sources:
        print(f"source {line}")
    return {"type": "FeatureCollection", "features": features}


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
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("source", nargs="?", help="mesh GeoJSON with population per cell")
    parser.add_argument("--download", action="store_true", help="fetch from e-Stat 統計GIS")
    parser.add_argument("--level", choices=sorted(LEVELS), default="500m")
    args = parser.parse_args()
    if args.download:
        payload = download_payload(args.level)
    elif args.source:
        source = pathlib.Path(args.source)
        if not source.is_file():
            print(f"mesh source not found: {source}", file=sys.stderr)
            return 1
        payload = json.loads(source.read_text(encoding="utf-8"))
    else:
        parser.print_usage(sys.stderr)
        return 2

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
            f"REAL 令和2年国勢調査 {args.level if args.download else '500m'}人口メッシュ clipped to the Nagoya bbox, wards "
            "assigned by point-in-polygon against the N03 ward fixture. Source: "
            "e-Stat 統計地理情報システム 地域メッシュ統計 (政府標準利用規約)."
        ),
        "features": features,
    }
    OUT_PATH.write_text(
        json.dumps(collection, ensure_ascii=False, separators=(",", ":")),
        encoding="utf-8",
    )
    digest = hashlib.sha256(OUT_PATH.read_bytes()).hexdigest()
    print(f"wrote {OUT_PATH} ({len(features)} cells, {unmatched} skipped)")
    print(f"sha256:{digest}  {OUT_PATH}")
    return 0


if __name__ == "__main__":
    sys.exit(main())