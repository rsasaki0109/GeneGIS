#!/usr/bin/env python3
"""Fetch REAL PLATEAU 名古屋市 3D都市モデル buildings (LOD1) and convert them.

Source:
  - PLATEAU 名古屋市 3D都市モデル 2022 (Project PLATEAU, MLIT)
    https://www.geospatial.jp/ckan/dataset/plateau-23100-nagoya-shi-2022
  - License: CC BY 4.0 (PLATEAU オープンデータ)

This script resolves the building dataset through the geospatial.jp CKAN API,
downloads the CityGML bundle, and converts LOD1 ``bldg:Building`` footprints
plus ``bldg:measuredHeight`` into a GeoJSON of building polygons. Coordinates
are kept in EPSG:6676 (JGD2011 / Japan Plane Rectangular CS VII) when the GML
declares it, or EPSG:4326 otherwise.

Output:
  examples/nagoya-population-density/data/real/nagoya-buildings-real.geojson

The schema (building_id, height_m, footprint) mirrors the scene LOD1 fixture so
the real buildings can feed scene-level checks. Converting to the projected
scene mesh for the full GPU path remains adapter scope.

Usage: python3 scripts/fetch-plateau.py
"""

from __future__ import annotations

import json
import pathlib
import sys
import urllib.parse
import urllib.request
import zipfile
import xml.etree.ElementTree as ET

ROOT = pathlib.Path(__file__).resolve().parent.parent
REAL = ROOT / "examples/nagoya-population-density/data/real"
CACHE = ROOT / ".genegis/real-data/plateau"

SEARCH_URL = (
    "https://www.geospatial.jp/ckan/api/3/action/package_search?q=plateau-23100-nagoya-shi-2022"
)
NAMESPACES = {
    "bldg": "http://www.opengis.net/citygml/building/2.0",
    "gml": "http://www.opengis.net/gml",
    "core": "http://www.opengis.net/citygml/2.0",
    "gen": "http://www.opengis.net/citygml/generics/2.0",
    "uro": "https://www.geospatial.jp/iur/uro/3.0",
}

# JGD2011 / Japan Plane Rectangular CS VII (Aichi, Mie).
EPSG_6676 = "EPSG:6676"


def log(message):
    print(message, flush=True)


def download(url, target, size_limit_mb=512):
    if target.is_file() and target.stat().st_size > 0:
        log(f"cache hit: {target.name}")
        return
    target.parent.mkdir(parents=True, exist_ok=True)
    log(f"downloading {url}")
    request = urllib.request.Request(url, headers={"User-Agent": "GeneGIS-real-data/1.0"})
    with urllib.request.urlopen(request, timeout=300) as response, target.open("wb") as out:
        out.write(response.read())
    if target.stat().st_size > size_limit_mb * 1024 * 1024:
        raise SystemExit(f"{target.name} exceeds {size_limit_mb} MB safety limit")


def resolve_building_zip() -> pathlib.Path:
    """Resolve the bldg CityGML bundle URL from the CKAN package metadata."""
    with urllib.request.urlopen(SEARCH_URL, timeout=60) as response:
        payload = json.load(response)
    results = payload.get("result", {}).get("results", [])
    for package in results:
        for resource in package.get("resources", []):
            name = str(resource.get("name", ""))
            if "bldg" in name.lower() and (
                resource.get("format", "").lower() == "zip" or name.lower().endswith(".zip")
            ):
                url = resource["url"]
                log(f"resolved {name} -> {url}")
                target = CACHE / "nagoya-bldg-citygml.zip"
                download(url, target)
                return target
    raise SystemExit("no building CityGML bundle found in the PLATEAU package")


def parse_ring_coordinates(pos_list, crs_axis):
    """Parse a gml:posList into a list of (x, y) coordinates."""
    values = [float(v) for v in pos_list.split()]
    points = []
    step = len(crs_axis)
    for i in range(0, len(values) - step + 1, step):
        if step >= 2:
            points.append((values[i], values[i + 1]))
    return points


def convert_building(path):
    """Convert one bldg CityGML file into building features."""
    features = []
    tree = ET.parse(path)
    root = tree.getroot()
    crs = root.attrib.get("srsName", "")
    crs = EPSG_6676 if "6676" in crs else ("EPSG:4326" if "4326" in crs else crs)

    for building in root.iter(f"{{{NAMESPACES['bldg']}}}Building"):
        gml_id = building.attrib.get(f"{{{NAMESPACES['gml']}}}id", f"bldg-{len(features)}")
        height = None
        for height_el in building.iter(f"{{{NAMESPACES['bldg']}}}measuredHeight"):
            try:
                height = float(height_el.text.strip())
            except (ValueError, AttributeError):
                pass
            break

        footprint = None
        # LOD1 footprint is typically a gml:Polygon under bldg:lod1Solid or
        # bldg:lod1Geometry (MultiSurface -> Polygon).
        for polygon in building.iter(f"{{{NAMESPACES['gml']}}}Polygon"):
            pos_lists = [
                el
                for el in polygon.iter(f"{{{NAMESPACES['gml']}}}posList")
            ]
            for pos_list in pos_lists:
                ring = parse_ring_coordinates(pos_list.text.strip(), ("x", "y", "z"))
                if len(ring) >= 3:
                    footprint = ring
                    break
            if footprint is not None:
                break

        if footprint is None or height is None or height <= 0.0:
            continue
        # Close the ring.
        if footprint[0] != footprint[-1]:
            footprint.append(footprint[0])
        features.append({
            "type": "Feature",
            "properties": {
                "building_id": gml_id,
                "height_m": round(height, 3),
            },
            "geometry": {
                "type": "Polygon",
                "coordinates": [[[round(x, 3), round(y, 3)] for x, y in footprint]],
            },
        })
    return features, crs


def main() -> int:
    bundle = resolve_building_zip()
    extract_dir = CACHE / "citygml"
    if not any(extract_dir.rglob("*.gml")):
        extract_dir.mkdir(parents=True, exist_ok=True)
        with zipfile.ZipFile(bundle) as archive:
            archive.extractall(extract_dir)

    features = []
    crs_used = None
    gml_files = sorted(extract_dir.rglob("*.gml"))
    if not gml_files:
        raise SystemExit("no CityGML files in the bundle")
    for gml in gml_files:
        file_features, crs = convert_building(gml)
        crs_used = crs
        features.extend(file_features)
        log(f"converted {gml.name}: {len(file_features)} buildings")

    if not features:
        raise SystemExit("no LOD1 buildings with measured height found")

    REAL.mkdir(parents=True, exist_ok=True)
    out = REAL / "nagoya-buildings-real.geojson"
    collection = {
        "type": "FeatureCollection",
        "name": "nagoya-buildings-real",
        "crs": crs_used or EPSG_6676,
        "description": (
            "REAL PLATEAU 名古屋市 3D都市モデル 2022 LOD1 buildings (footprint + "
            "measured height). Source: Project PLATEAU (MLIT) via geospatial.jp "
            "(CC BY 4.0). Not a substitute for official cadastral or building data."
        ),
        "features": features,
    }
    out.write_text(json.dumps(collection, ensure_ascii=False, separators=(",", ":")), encoding="utf-8")
    import hashlib

    digest = hashlib.sha256(out.read_bytes()).hexdigest()
    log(f"wrote {out} ({len(features)} buildings)")
    log(f"sha256:{digest}  {out}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, json.JSONDecodeError, ET.ParseError) as error:
        print(f"fetch-plateau: {error}", file=sys.stderr)
        sys.exit(1)