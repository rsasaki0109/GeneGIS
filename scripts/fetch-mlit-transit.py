#!/usr/bin/env python3
"""Fetch REAL 国土数値情報 鉄道(N02)・バスルート(N07) over the Nagoya bbox.

Sources (both 国土数値情報, 政府標準利用規約):
  - 鉄道データ (N02, 2025年度): https://nlftp.mlit.go.jp/ksj/gml/data/N02/N02-25/N02-25_GML.zip
  - バスルート (N07, 2022年度): https://nlftp.mlit.go.jp/ksj/gml/data/N07/N07-22/N07-22_GML.zip

Both are national JPGIS GML bundles. This script downloads the national
bundle, parses the transit line geometry, and clips to the Nagoya bbox,
emitting a GeoJSON of LineString ride corridors with operator/line identity.

Output:
  examples/nagoya-population-density/data/real/nagoya-transit-real.geojson

The schema (line_id, operator, mode, is_line) feeds the multimodal walk+ride
accessibility path; converting ride corridors into a headway/transfer graph
remains adapter scope (see docs/rfcs/0006-open-data-expansion.md).

Usage: python3 scripts/fetch-mlit-transit.py [--rail] [--bus]
"""

from __future__ import annotations

import json
import pathlib
import sys
import urllib.request
import zipfile
import xml.etree.ElementTree as ET

ROOT = pathlib.Path(__file__).resolve().parent.parent
REAL = ROOT / "examples/nagoya-population-density/data/real"
CACHE = ROOT / ".genegis/real-data/transit"

NAGOYA = (136.78, 35.02, 137.08, 35.28)

RAIL_URL = "https://nlftp.mlit.go.jp/ksj/gml/data/N02/N02-25/N02-25_GML.zip"
BUS_URL = "https://nlftp.mlit.go.jp/ksj/gml/data/N07/N07-22/N07-22_GML.zip"

GML = "http://www.opengis.net/gml"


def log(message):
    print(message, flush=True)


def download(url, target, size_limit_mb=512):
    if target.is_file() and target.stat().st_size > 0:
        log(f"cache hit: {target.name}")
        return
    target.parent.mkdir(parents=True, exist_ok=True)
    log(f"downloading {url}")
    request = urllib.request.Request(url, headers={"User-Agent": "GeneGIS-real-data/1.0"})
    with urllib.request.urlopen(request, timeout=600) as response, target.open("wb") as out:
        out.write(response.read())
    if target.stat().st_size > size_limit_mb * 1024 * 1024:
        raise SystemExit(f"{target.name} exceeds {size_limit_mb} MB safety limit")


def bbox_overlaps(coords):
    xs = [p[0] for p in coords]
    ys = [p[1] for p in coords]
    return not (
        max(xs) < NAGOYA[0]
        or min(xs) > NAGOYA[2]
        or max(ys) < NAGOYA[1]
        or min(ys) > NAGOYA[3]
    )


def parse_curve_coordinates(curve):
    """Extract a coordinate list from a gml:LineString / gml:Curve posList."""
    for pos_list in curve.iter(f"{{{GML}}}posList"):
        values = pos_list.text.split()
        points = [(float(values[i]), float(values[i + 1])) for i in range(0, len(values), 2)]
        return points
    for line_string in curve.iter(f"{{{GML}}}LineString"):
        return parse_curve_coordinates(line_string)
    return []


def convert_gml(path, mode, operator_props):
    """Convert one JPGIS GML file into transit LineString features clipped to Nagoya."""
    features = []
    tree = ET.parse(path)
    root = tree.getroot()
    # JPGIS attributes live in a transaction/feature member block; we iterate
    # all elements named after the record type and read its properties.
    for member in root.iter():
        tag = member.tag.rsplit("}", 1)[-1]
        if tag not in ("N02RailroadSection", "N07Route", "N07BusRoute", "N07Service"):
            continue
        properties = {}
        for child in member:
            child_tag = child.tag.rsplit("}", 1)[-1]
            text = (child.text or "").strip()
            if child_tag in ("r:", "fid", "note"):
                continue
            properties[child_tag] = text
        operator = properties.get("r2mojname") or properties.get("r02_001") or "unknown"
        operator = operator_props.get(operator, operator)
        geometry = None
        for child in member.iter(f"{{{GML}}}LineString"):
            coords = parse_curve_coordinates(child)
            if coords and bbox_overlaps(coords):
                geometry = coords
                break
        if geometry is None:
            continue
        features.append({
            "type": "Feature",
            "properties": {
                "line_id": properties.get("fid", f"{mode}-{len(features)}"),
                "operator": operator,
                "mode": mode,
                "is_line": True,
            },
            "geometry": {"type": "LineString", "coordinates": geometry},
        })
    return features


def main() -> int:
    want_rail = "--rail" in sys.argv
    want_bus = "--bus" in sys.argv
    if not want_rail and not want_bus:
        want_rail = want_bus = True

    features = []
    if want_rail:
        bundle = CACHE / "N02-25_GML.zip"
        download(RAIL_URL, bundle)
        extract = CACHE / "n02"
        if not any(extract.rglob("*.xml")):
            with zipfile.ZipFile(bundle) as archive:
                archive.extractall(extract)
        for xml_path in sorted(extract.rglob("*.xml")):
            features.extend(convert_gml(xml_path, "rail", {}))
        log(f"rail: {len(features)} features")

    if want_bus:
        bundle = CACHE / "N07-22_GML.zip"
        download(BUS_URL, bundle)
        extract = CACHE / "n07"
        if not any(extract.rglob("*.xml")):
            with zipfile.ZipFile(bundle) as archive:
                archive.extractall(extract)
        for xml_path in sorted(extract.rglob("*.xml")):
            features.extend(convert_gml(xml_path, "bus", {}))
        log(f"bus: {len(features)} features (combined)")

    if not features:
        raise SystemExit("no transit lines matched the Nagoya bbox")

    REAL.mkdir(parents=True, exist_ok=True)
    out = REAL / "nagoya-transit-real.geojson"
    collection = {
        "type": "FeatureCollection",
        "name": "nagoya-transit-real",
        "crs": "EPSG:4326",
        "description": (
            "REAL 国土数値情報 鉄道(N02 2025) + バスルート(N07 2022) clipped to the "
            "Nagoya bbox as ride corridors. Source: 国土数値情報 (政府標準利用規約). "
            "Headway/transfer graph conversion is adapter scope."
        ),
        "features": features,
    }
    out.write_text(json.dumps(collection, ensure_ascii=False, separators=(",", ":")), encoding="utf-8")
    import hashlib

    digest = hashlib.sha256(out.read_bytes()).hexdigest()
    log(f"wrote {out} ({len(features)} features)")
    log(f"sha256:{digest}  {out}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, json.JSONDecodeError, ET.ParseError) as error:
        print(f"fetch-mlit-transit: {error}", file=sys.stderr)
        sys.exit(1)