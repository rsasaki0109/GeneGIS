#!/usr/bin/env python3
"""Generate the synthetic Nagoya evacuation-shelter fixture (UC-1 network part).

Two designated shelters per ward are placed deterministically near the ward
centroid, nudged outward until they fall OUTSIDE every bundled flood zone
polygon. Capacities follow a fixed per-ward table.

Output:
  examples/nagoya-population-density/data/nagoya-shelters.geojson

Pure function of this script and of nagoya-wards/nagoya-flood-zones;
no randomness, no downloads.
"""

import json
import pathlib

DATA = pathlib.Path(__file__).resolve().parent.parent / (
    "examples/nagoya-population-density/data"
)

# Fixed offsets from ward centroid in degrees (~100-300 m).
SHELTER_OFFSETS = [(0.0030, 0.0022), (-0.0026, -0.0018)]
# Nudge step when a candidate lands inside a flood zone polygon: fixed
# offsets first, then a deterministic outward ring scan (12 angles x 3 radii).
NUDGE_STEPS = [
    (0.0, 0.0), (0.0025, 0.0), (-0.0025, 0.0), (0.0, 0.0020),
    (0.0, -0.0020), (0.0025, 0.0020), (-0.0025, -0.0020),
    (0.0038, 0.0028),
]
import math

for radius_lon in (0.006, 0.010, 0.015):
    for angle in range(0, 360, 30):
        rad = math.radians(angle)
        NUDGE_STEPS.append((
            round(radius_lon * math.cos(rad), 6),
            round(radius_lon * 0.75 * math.sin(rad), 6),
        ))
CAPACITY_BY_WARD = {
    "中区": 4200, "東区": 2600, "西区": 2400, "中村区": 2800, "中川区": 3000,
    "港区": 3200, "南区": 2600, "北区": 2200, "千種区": 2600, "東山区": 0,
    "昭和区": 2400, "瑞穂区": 2300, "熱田区": 2100, "緑区": 3400,
    "名東区": 2400, "天白区": 2900,
}
DEFAULT_CAPACITY = 2500


def load_polygons(path):
    payload = json.loads(path.read_text())
    polygons = []
    for feature in payload["features"]:
        geometry = feature["geometry"]
        polys = (
            [geometry["coordinates"]]
            if geometry["type"] == "Polygon"
            else geometry["coordinates"]
        )
        for poly in polys:
            rings = [[(c[0], c[1]) for c in ring] for ring in poly]
            polygons.append(rings)
    return polygons


FLOOD_RINGS = load_polygons(DATA / "nagoya-flood-zones.geojson")


def point_in_ring(x, y, ring):
    """Even-odd ray cast; mirrors genegis-geometry::predicate."""
    inside = False
    n = len(ring)
    for i in range(n):
        x1, y1 = ring[i]
        x2, y2 = ring[(i + 1) % n]
        if (y1 > y) != (y2 > y):
            t = (y - y1) / (y2 - y1)
            if x < x1 + t * (x2 - x1):
                inside = not inside
    return inside


def point_in_polygons(x, y):
    for rings in FLOOD_RINGS:
        if point_in_ring(x, y, rings[0]) and not any(
            point_in_ring(x, y, hole) for hole in rings[1:]
        ):
            return True
    return False


def derive_ward_centroids():
    wards = json.loads((DATA / "nagoya-wards.geojson").read_text())
    centroids = []
    for feature in wards["features"]:
        name = feature["properties"]["ward_name"]
        geometry = feature["geometry"]
        polys = (
            [geometry["coordinates"]]
            if geometry["type"] == "Polygon"
            else geometry["coordinates"]
        )
        xs = [c[0] for poly in polys for ring in poly for c in ring]
        ys = [c[1] for poly in polys for ring in poly for c in ring]
        centroids.append((name, sum(xs) / len(xs), sum(ys) / len(ys)))
    return sorted(centroids)


def main():
    features = []
    shelter_id = 1
    for ward_name, clon, clat in derive_ward_centroids():
        for index, (dlon, dlat) in enumerate(SHELTER_OFFSETS):
            base_lon, base_lat = clon + dlon, clat + dlat
            placed = None
            for nudge_lon, nudge_lat in NUDGE_STEPS:
                candidate = (round(base_lon + nudge_lon, 6),
                             round(base_lat + nudge_lat, 6))
                if not point_in_polygons(*candidate):
                    placed = candidate
                    break
            if placed is None:
                raise SystemExit(
                    f"no safe shelter spot found for {ward_name} #{index + 1}"
                )
            capacity = CAPACITY_BY_WARD.get(ward_name, DEFAULT_CAPACITY)
            if capacity == 0:
                capacity = DEFAULT_CAPACITY
            features.append({
                "type": "Feature",
                "properties": {
                    "shelter_id": f"shelter-{shelter_id:03d}",
                    "name": f"{ward_name}指定避難所{index + 1}",
                    "ward_name": ward_name,
                    "kind": "designated_evacuation_shelter",
                    "capacity": capacity,
                },
                "geometry": {
                    "type": "Point",
                    "coordinates": list(placed),
                },
            })
            shelter_id += 1
    payload = {
        "type": "FeatureCollection",
        "name": "nagoya-shelters",
        "crs": "EPSG:4326",
        "description": "Synthetic designated evacuation shelters (2 per ward), "
        "placed outside bundled flood zones. Generated by "
        "scripts/build-nagoya-shelters.py; not GSI survey data.",
        "features": features,
    }
    path = DATA / "nagoya-shelters.geojson"
    path.write_text(json.dumps(payload, ensure_ascii=False, separators=(",", ":")))
    print(f"wrote {path} ({len(features)} shelters)")


if __name__ == "__main__":
    main()
