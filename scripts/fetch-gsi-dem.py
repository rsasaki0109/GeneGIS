#!/usr/bin/env python3
"""Fetch REAL GSI 標高タイル (5m DEM) over the Nagoya bbox.

Source:
  - GSI 標高タイル (DEM5A, 5m mesh), text format, one CSV tile per 256x256 px
    https://cyberjapandata.gsi.go.jp/xyz/dem5a/{z}/{x}/{y}.txt
  - License: 国土地理院コンテンツ利用規約（政府標準利用規約準拠）

The text tiles carry one elevation (m, two decimals) per pixel; missing pixels
are ``e``. This script downloads the tiles covering the Nagoya bbox at zoom 15
(equivalent to the 5m grid) and converts them to a deterministic GeoJSON of
labeled elevation points in EPSG:4326.

Output:
  examples/nagoya-population-density/data/real/nagoya-dem5a-real.geojson

The point schema (lon, lat, elevation_m) mirrors the 3D scene point vocabulary
so the real DEM can feed scene-level checks; converting to a projected COG for
the full GPU scene remains adapter scope (see docs/rfcs/0006-open-data-expansion.md).

Usage: python3 scripts/fetch-gsi-dem.py
"""

from __future__ import annotations

import json
import pathlib
import sys
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
REAL = ROOT / "examples/nagoya-population-density/data/real"
CACHE = ROOT / ".genegis/real-data/dem5a"

# Nagoya bbox (matches catalog records). Override with GENEGIS_DEM_BBOX for a
# lighter district-level fetch: "west,south,east,north".
import os as _os

_bbox_override = _os.environ.get("GENEGIS_DEM_BBOX")
if _bbox_override:
    west, south, east, north = (float(v) for v in _bbox_override.split(","))
    NAGOYA = (west, south, east, north)

ZOOM = 15
ORIGIN_URL = f"https://cyberjapandata.gsi.go.jp/xyz/dem5a/{ZOOM}"


def log(message):
    print(message, flush=True)


def lonlat_to_tile(lon, lat, z):
    n = 2 ** z
    x = int((lon + 180.0) / 360.0 * n)
    y = int((1.0 - __import__("math").asinh(__import__("math").tan(
        __import__("math").radians(lat))) / __import__("math").pi) / 2.0 * n)
    return x, y


def tile_to_lonlat(x, y, z):
    n = 2 ** z
    import math
    lon = x / n * 360.0 - 180.0
    lat = math.degrees(math.atan(math.sinh(math.pi * (1.0 - 2.0 * y / n))))
    return lon, lat


def download(url, target):
    if target.is_file() and target.stat().st_size > 0:
        log(f"cache hit: {target.name}")
        return
    target.parent.mkdir(parents=True, exist_ok=True)
    log(f"downloading {url}")
    request = urllib.request.Request(url, headers={"User-Agent": "GeneGIS-real-data/1.0"})
    with urllib.request.urlopen(request, timeout=60) as response, target.open("wb") as out:
        out.write(response.read())


def parse_tile(path):
    """Return rows of elevations (m or None) for one 256x256 text tile."""
    rows = []
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line:
            continue
        rows.append([None if cell == "e" else float(cell) for cell in line.split(",")])
    return rows


def main() -> int:
    x0, y0 = lonlat_to_tile(NAGOYA[0], NAGOYA[3], ZOOM)
    x1, y1 = lonlat_to_tile(NAGOYA[2], NAGOYA[1], ZOOM)
    # Validate tile range sanity (Nagoya at z15 is a small region).
    tile_count = (x1 - x0 + 1) * (y1 - y0 + 1)
    if tile_count > 2048:
        raise SystemExit(f"suspicious tile count {tile_count}; refusing to continue")
    if tile_count > 256:
        log(
            f"WARNING: {tile_count} tiles (~{tile_count * 340 // 1024} MB) will be "
            f"downloaded; consider a district bbox for a lighter fetch"
        )

    features = []
    total_points = 0
    for x in range(x0, x1 + 1):
        for y in range(y0, y1 + 1):
            tile_path = CACHE / f"{ZOOM}/{x}/{y}.txt"
            download(f"{ORIGIN_URL}/{x}/{y}.txt", tile_path)
            rows = parse_tile(tile_path)
            west, north = tile_to_lonlat(x, y, ZOOM)
            east, south = tile_to_lonlat(x + 1, y + 1, ZOOM)
            dlon = (east - west) / 256.0
            dlat = (north - south) / 256.0
            for row_index, row in enumerate(rows):
                for col_index, elevation in enumerate(row):
                    if elevation is None:
                        continue
                    lon = round(west + (col_index + 0.5) * dlon, 6)
                    lat = round(north - (row_index + 0.5) * dlat, 6)
                    if not (NAGOYA[0] <= lon <= NAGOYA[2] and NAGOYA[1] <= lat <= NAGOYA[3]):
                        continue
                    features.append({
                        "type": "Feature",
                        "properties": {"elevation_m": round(elevation, 2)},
                        "geometry": {"type": "Point", "coordinates": [lon, lat]},
                    })
                    total_points += 1

    if total_points == 0:
        raise SystemExit("no elevation points matched the Nagoya bbox")

    REAL.mkdir(parents=True, exist_ok=True)
    out = REAL / "nagoya-dem5a-real.geojson"
    collection = {
        "type": "FeatureCollection",
        "name": "nagoya-dem5a-real",
        "crs": "EPSG:4326",
        "description": (
            "REAL GSI 標高タイル DEM5A (5m) over the Nagoya bbox, text format, "
            "elevation in metres. Source: 国土地理院 標高タイル "
            "(https://cyberjapandata.gsi.go.jp/xyz/dem5a/, 国土地理院コンテンツ利用規約). "
            "Not a substitute for official terrain products."
        ),
        "zoom": ZOOM,
        "features": features,
    }
    out.write_text(json.dumps(collection, ensure_ascii=False, separators=(",", ":")), encoding="utf-8")
    import hashlib

    digest = hashlib.sha256(out.read_bytes()).hexdigest()
    log(f"wrote {out} ({total_points} points, {tile_count} tiles)")
    log(f"sha256:{digest}  {out}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"fetch-gsi-dem: {error}", file=sys.stderr)
        sys.exit(1)