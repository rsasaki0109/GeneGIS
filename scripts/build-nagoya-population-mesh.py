#!/usr/bin/env python3
"""Build the deterministic synthetic 500m population-mesh fixture.

This mirrors the real-data schema produced by ``scripts/fetch-estat-mesh.py``
so the same ``run_nagoya_population_density_mesh`` path and the same ward
oracle verifier cover both the offline fixture and the licensed real mesh.

The fixture is NOT real census data.  It is a deterministic offline artifact
that conserves the official 令和2年国勢調査 ward populations (the immutable
``nagoya-oracle-2020.json`` totals): each N03 ward polygon is tiled by an
aligned ~500m square grid, cells overlapping the ward are kept, and the ward
population is distributed across those cells by area (fraction of the cell
inside the ward).  Ward totals therefore reproduce the oracle by construction,
which is exactly what the mesh->ward verifier checks on the real path too.

Output:
  examples/nagoya-population-density/data/nagoya-population-mesh.geojson
"""

from __future__ import annotations

import json
import math
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
DATA = ROOT / "examples" / "nagoya-population-density" / "data"
WARDS_PATH = DATA / "nagoya-wards.geojson"
ORACLE_PATH = DATA / "nagoya-oracle-2020.json"
OUT_PATH = DATA / "nagoya-population-mesh.geojson"

# ~500m at Nagoya latitude.  A 1/240 degree grid step (0.0041666..) is about
# 463m in x and 463m in y at 35.2N; using an aligned grid keeps meshes stable
# and reproducible without needing a projection.
MESH_DEG = 1.0 / 240.0

# Nagoya bbox (matches catalog records, padded slightly).
NAGOYA = (136.78, 35.02, 137.08, 35.28)


def ring_area(ring: list[list[float]]) -> float:
    """Signed area of a lon/lat ring (planar shoelace, good enough for tiling)."""
    return sum(
        ring[i][0] * ring[(i + 1) % len(ring)][1]
        - ring[(i + 1) % len(ring)][0] * ring[i][1]
        for i in range(len(ring))
    ) * 0.5


def point_in_ring(x: float, y: float, ring: list[list[float]]) -> bool:
    inside = False
    for i in range(len(ring)):
        x1, y1 = ring[i]
        x2, y2 = ring[(i + 1) % len(ring)]
        if (y1 > y) != (y2 > y):
            at_x = (x2 - x1) * (y - y1) / (y2 - y1) + x1
            if x < at_x:
                inside = not inside
    return inside


def in_polygon(x: float, y: float, polygon: list[list[list[float]]]) -> bool:
    """Point-in-polygon respecting holes."""
    exterior = polygon[0]
    if not point_in_ring(x, y, exterior):
        return False
    return not any(point_in_ring(x, y, hole) for hole in polygon[1:])


def cell_inside_fraction(
    cx: float, cy: float, half: float, polygon: list[list[list[float]]]
) -> float:
    """Fraction of a cell inside a ward polygon, estimated by 2x2 centroid grid.

    All sample points are relative offsets from the cell southwest corner
    ``(cx, cy)``; ``half`` is the cell extent. A 2x2 grid keeps the synthetic
    mesh a smooth gradient while keeping generation fast for the full bbox.
    """
    inside = 0
    samples = 2
    for i in range(samples):
        for j in range(samples):
            sx = cx + half * (i + 0.5) / samples
            sy = cy + half * (j + 0.5) / samples
            if in_polygon(sx, sy, polygon):
                inside += 1
    return inside / (samples * samples)


def polygon_bbox(polygon: list[list[list[float]]]) -> tuple[float, float, float, float]:
    """Axis-aligned bbox (min_x, min_y, max_x, max_y) of a polygon part."""
    xs = [p[0] for ring in polygon for p in ring]
    ys = [p[1] for ring in polygon for p in ring]
    return min(xs), min(ys), max(xs), max(ys)


def main() -> int:
    wards = json.loads(WARDS_PATH.read_text(encoding="utf-8"))
    oracle = json.loads(ORACLE_PATH.read_text(encoding="utf-8"))
    oracle_by_code = {w["ward_code"]: w for w in oracle["wards"]}

    features: list[dict[str, Any]] = []
    total = 0
    for feature in wards["features"]:
        props = feature["properties"]
        code = props["ward_code"]
        name = props["ward_name"]
        ward_pop = int(props["population"])
        oracle_ward = oracle_by_code[code]
        if ward_pop != oracle_ward["population"]:
            raise ValueError(
                f"{code} population mismatch: fixture={ward_pop} oracle={oracle_ward['population']}"
            )

        polygons = feature["geometry"]["coordinates"]

        # Aligned grid origin snapped to the Nagoya bbox southwest corner.
        origin_x = NAGOYA[0]
        origin_y = NAGOYA[1]
        n_cols = int(math.floor((NAGOYA[2] - NAGOYA[0]) / MESH_DEG)) + 1
        n_rows = int(math.floor((NAGOYA[3] - NAGOYA[1]) / MESH_DEG)) + 1

        # Candidate-cell range from the union of polygon bboxes so we do not
        # scan the whole bbox grid for every ward.
        min_x = min(x for (x, _, _, _) in map(polygon_bbox, polygons))
        min_y = min(y for (_, y, _, _) in map(polygon_bbox, polygons))
        max_x = max(x for (_, _, x, _) in map(polygon_bbox, polygons))
        max_y = max(y for (_, _, _, y) in map(polygon_bbox, polygons))
        col0 = int(math.floor((min_x - NAGOYA[0]) / MESH_DEG)) - 1
        col1 = int(math.floor((max_x - NAGOYA[0]) / MESH_DEG)) + 1
        row0 = int(math.floor((min_y - NAGOYA[1]) / MESH_DEG)) - 1
        row1 = int(math.floor((max_y - NAGOYA[1]) / MESH_DEG)) + 1
        col0 = max(col0, 0)
        col1 = min(col1, n_cols)
        row0 = max(row0, 0)
        row1 = min(row1, n_rows)

        weights: list[tuple[float, float, float]] = []  # (cx, cy, weight)
        for row in range(row0, row1):
            for col in range(col0, col1):
                cx = origin_x + col * MESH_DEG
                cy = origin_y + row * MESH_DEG
                weight = 0.0
                for polygon in polygons:
                    weight = max(weight, cell_inside_fraction(cx, cy, MESH_DEG, polygon))
                if weight > 0.0:
                    weights.append((cx, cy, weight))

        if not weights:
            raise ValueError(f"{code}: no mesh cell overlaps the ward polygon")

        weight_sum = sum(w for (_, _, w) in weights)
        # Distribute ward population by fractional-area weight; a final carry
        # assigns the rounding remainder to the largest cell so the ward total
        # is conserved exactly (this is what the oracle verifier requires).
        allocated = [ward_pop * w / weight_sum for (_, _, w) in weights]
        rounded = [int(math.floor(p)) for p in allocated]
        remainder = ward_pop - sum(rounded)
        order = sorted(range(len(weights)), key=lambda i: allocated[i] - rounded[i], reverse=True)
        for i in order[:remainder]:
            rounded[i] += 1

        for (cx, cy, _w), pop in zip(weights, rounded):
            if pop <= 0:
                continue
            col = int(round((cx - NAGOYA[0]) / MESH_DEG))
            row = int(round((cy - NAGOYA[1]) / MESH_DEG))
            mesh_id = f"mesh-{col:04d}-{row:04d}"
            ring = [
                [round(cx, 6), round(cy, 6)],
                [round(cx + MESH_DEG, 6), round(cy, 6)],
                [round(cx + MESH_DEG, 6), round(cy + MESH_DEG, 6)],
                [round(cx, 6), round(cy + MESH_DEG, 6)],
                [round(cx, 6), round(cy, 6)],
            ]
            features.append(
                {
                    "type": "Feature",
                    "properties": {
                        "mesh_id": mesh_id,
                        "ward_code": code,
                        "ward_name": name,
                        "population": pop,
                        "cell_fraction": round(_w, 4),
                    },
                    "geometry": {"type": "Polygon", "coordinates": [ring]},
                }
            )
            total += pop

    # Conservation check against the oracle.
    if total != oracle["population_total"]:
        raise ValueError(f"mesh total {total} != oracle total {oracle['population_total']}")

    collection = {
        "type": "FeatureCollection",
        "name": "nagoya-population-mesh",
        "crs": "EPSG:4326",
        "description": (
            "SYNTHETIC deterministic 500m population-mesh fixture. Conserves the "
            "official 2020 ward totals by area-weighted cell allocation; NOT real "
            "census mesh data. Use scripts/fetch-estat-mesh.py for the licensed "
            "real 500m mesh."
        ),
        "mesh_step_deg": MESH_DEG,
        "features": features,
    }
    OUT_PATH.write_text(
        json.dumps(collection, ensure_ascii=False, separators=(",", ":")),
        encoding="utf-8",
    )
    print(f"Wrote {OUT_PATH} ({len(features)} mesh cells; {total} people)")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        print(f"build-nagoya-population-mesh: {error}", file=sys.stderr)
        sys.exit(1)