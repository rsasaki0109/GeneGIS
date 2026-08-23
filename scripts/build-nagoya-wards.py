#!/usr/bin/env python3
"""Build the lossless Nagoya ward GeoJSON fixture.

The upstream N03 snapshots are not guaranteed to have one feature per ward,
and a few snapshots contain several polygon parts in one ``Polygon`` shell
(the Minato ward snapshot is one example).  The old builder selected
``features[0]`` and then retained only the first ring in the vector reader,
which silently changed the ward area.  This builder validates the ward join,
retains every source feature/polygon/ring, and normalizes malformed
multi-part shells into separate polygons without dropping coordinates.
"""

from __future__ import annotations

import json
import sys
import urllib.request
from pathlib import Path
from typing import Any

BASE = "https://raw.githubusercontent.com/niiyz/JapanCityGeoJson/master/geojson/23"
ROOT = Path(__file__).resolve().parents[1]
DATA = ROOT / "examples" / "nagoya-population-density" / "data"
POP_PATH = DATA / "nagoya-population-2020.json"
OUT_PATH = DATA / "nagoya-wards.geojson"


def fetch_ward(code: str) -> dict[str, Any]:
    """Fetch one N03-derived ward snapshot from the pinned public source."""

    url = f"{BASE}/{code}.json"
    with urllib.request.urlopen(url, timeout=60) as resp:
        return json.load(resp)


def _signed_ring_area(ring: list[list[float]]) -> float:
    return sum(
        ring[i][0] * ring[(i + 1) % len(ring)][1]
        - ring[(i + 1) % len(ring)][0] * ring[i][1]
        for i in range(len(ring))
    ) * 0.5


def _point_in_ring(point: list[float], ring: list[list[float]]) -> bool:
    """Return whether a point is inside a lon/lat ring (boundary inclusive)."""

    x, y = point
    inside = False
    for i in range(len(ring)):
        x1, y1 = ring[i]
        x2, y2 = ring[(i + 1) % len(ring)]
        crosses = (y1 > y) != (y2 > y)
        if crosses:
            at_x = (x2 - x1) * (y - y1) / (y2 - y1) + x1
            if x < at_x:
                inside = not inside
    return inside


def _validate_ring(ring: Any, context: str) -> list[list[float]]:
    if not isinstance(ring, list) or len(ring) < 4:
        raise ValueError(f"{context}: polygon ring needs at least four positions")
    normalized: list[list[float]] = []
    for position in ring:
        if not isinstance(position, list) or len(position) < 2:
            raise ValueError(f"{context}: position needs longitude/latitude")
        x, y = float(position[0]), float(position[1])
        if not (-180 <= x <= 180 and -90 <= y <= 90):
            raise ValueError(f"{context}: coordinate out of WGS84 range: {x},{y}")
        normalized.append([x, y])
    if normalized[0] != normalized[-1]:
        raise ValueError(f"{context}: polygon ring is not closed")
    if abs(_signed_ring_area(normalized)) == 0:
        raise ValueError(f"{context}: polygon ring has zero area")
    return normalized


def _polygon_parts(geometry: dict[str, Any], context: str) -> list[list[list[list[float]]]]:
    """Return all polygon parts while preserving valid holes.

    ``Polygon`` follows GeoJSON's ``[exterior, hole, ...]`` convention.  A
    malformed source shell whose purported holes lie outside the exterior is
    split into independent polygon parts.  This is the representation used
    by the upstream N03 conversion for Minato; splitting is lossless and
    yields the intended area while still preserving every coordinate.
    """

    geometry_type = geometry.get("type")
    coordinates = geometry.get("coordinates")
    if geometry_type == "Polygon":
        raw_polygons = [coordinates]
    elif geometry_type == "MultiPolygon":
        raw_polygons = coordinates
    else:
        raise ValueError(f"{context}: unsupported geometry type {geometry_type!r}")
    if not isinstance(raw_polygons, list) or not raw_polygons:
        raise ValueError(f"{context}: empty polygon geometry")

    parts: list[list[list[list[float]]]] = []
    for polygon_index, raw_polygon in enumerate(raw_polygons):
        if not isinstance(raw_polygon, list) or not raw_polygon:
            raise ValueError(f"{context} polygon {polygon_index}: empty polygon")
        rings = [
            _validate_ring(ring, f"{context} polygon {polygon_index} ring {ring_index}")
            for ring_index, ring in enumerate(raw_polygon)
        ]
        exterior = rings[0]
        holes = rings[1:]
        # Valid holes must be inside their shell.  If they are not, this is a
        # source conversion's multi-part shell, not a topological hole.  Keep
        # every ring as an independent polygon so no area is silently lost.
        holes_are_contained = all(_point_in_ring(hole[0], exterior) for hole in holes)
        if holes and not holes_are_contained:
            parts.extend([[ring] for ring in rings])
        else:
            parts.append(rings)
    return parts


def build_ward_feature(ward: dict[str, Any], geo: dict[str, Any], population: dict[str, Any]) -> dict[str, Any]:
    code = ward["ward_code"]
    source_features = geo.get("features")
    if not isinstance(source_features, list) or not source_features:
        raise ValueError(f"{code}: boundary source has no features")

    polygons: list[list[list[list[float]]]] = []
    hole_count = 0
    prefectures: set[str] = set()
    cities: set[str] = set()
    for feature_index, source_feature in enumerate(source_features):
        props = source_feature.get("properties") or {}
        source_code = str(props.get("N03_007") or props.get("ward_code") or "")
        source_name = str(props.get("N03_004") or props.get("ward_name") or "")
        if source_code and source_code != code:
            raise ValueError(
                f"{code}: boundary feature {feature_index} has mismatched ward code {source_code}"
            )
        if source_name and source_name != ward["ward_name"]:
            raise ValueError(
                f"{code}: boundary feature {feature_index} has mismatched ward name {source_name}"
            )
        if props.get("N03_001"):
            prefectures.add(str(props["N03_001"]))
        if props.get("N03_003"):
            cities.add(str(props["N03_003"]))
        geometry = source_feature.get("geometry")
        if not isinstance(geometry, dict):
            raise ValueError(f"{code}: boundary feature {feature_index} has no geometry")
        feature_parts = _polygon_parts(geometry, f"{code} feature {feature_index}")
        polygons.extend(feature_parts)
        hole_count += sum(max(len(part) - 1, 0) for part in feature_parts)

    if not polygons:
        raise ValueError(f"{code}: boundary source has no polygon parts")
    if len(prefectures) > 1 or len(cities) > 1:
        raise ValueError(f"{code}: boundary features disagree on prefecture/city properties")
    source_url = f"{BASE}/{code}.json"
    return {
        "type": "Feature",
        "properties": {
            "ward_code": code,
            "ward_name": ward["ward_name"],
            "ward_name_en": ward["ward_name_en"],
            "population": ward["population"],
            "census_year": population["census_year"],
            "prefecture": next(iter(prefectures), "愛知県"),
            "city": next(iter(cities), "名古屋市"),
            "boundary_source": "MLIT N03 via JapanCityGeoJson",
            "boundary_url": source_url,
            "boundary_feature_count": len(source_features),
            "boundary_polygon_part_count": len(polygons),
            "boundary_hole_count": hole_count,
            "population_source": population["source"],
            "population_source_url": population["source_url"],
            "population_source_data_url": population["source_data_url"],
            "population_source_version": population["source_version"],
            "population_license": population["license"],
            "population_retrieval_basis": population["retrieval_basis"],
        },
        "geometry": {
            "type": "MultiPolygon",
            "coordinates": polygons,
        },
    }


def main() -> int:
    population = json.loads(POP_PATH.read_text(encoding="utf-8"))
    wards = population.get("wards")
    if not isinstance(wards, list) or len(wards) != 16:
        raise ValueError("population source must contain exactly 16 Nagoya wards")
    expected_total = population.get("population_total")
    actual_total = sum(int(ward["population"]) for ward in wards)
    if expected_total != actual_total:
        raise ValueError(
            f"population total mismatch: manifest={expected_total}, rows={actual_total}"
        )

    features = []
    for ward in wards:
        code = ward["ward_code"]
        features.append(build_ward_feature(ward, fetch_ward(code), population))

    collection = {
        "type": "FeatureCollection",
        "name": "nagoya-wards",
        "crs": "EPSG:4326",
        "population_source": population["source"],
        "population_source_url": population["source_url"],
        "population_source_data_url": population["source_data_url"],
        "population_source_version": population["source_version"],
        "population_license": population["license"],
        "population_retrieval_basis": population["retrieval_basis"],
        "population_total": actual_total,
        "boundary_source": "MLIT N03 via JapanCityGeoJson",
        "boundary_source_base_url": BASE,
        "features": features,
    }
    OUT_PATH.write_text(
        json.dumps(collection, ensure_ascii=False, separators=(",", ":")),
        encoding="utf-8",
    )
    print(f"Wrote {OUT_PATH} ({len(features)} wards; {actual_total} people)")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        print(f"build-nagoya-wards: {error}", file=sys.stderr)
        sys.exit(1)
