#!/usr/bin/env python3
"""Fetch REAL open data for the UC-1 workflows and convert it to fixture schemas.

Sources (both CC-BY 4.0):
  1. 国土数値情報 洪水浸水想定区域（河川単位） A31a, 愛知県, 想定最大規模
     https://nlftp.mlit.go.jp/ksj/gml/data/A31a/A31a-24/A31a-24_23_10_GEOJSON.zip
     Category dir 20 = 想定最大規模; A31a_205 = 浸水深ランク (1:0.5未満,
     2:0.5-3, 3:3-5, 4:5以上, 5:範囲外) mapped onto our depth bands.
  2. 名古屋市 指定避難所 (令和7年8月時点) via BODIK CKAN
     https://data.bodik.jp/dataset/231002_0200030000_27
     DBF carries 経度/緯度 in degrees; wards assigned by point-in-polygon.

Outputs (deterministic given the same upstream bytes):
  examples/nagoya-population-density/data/real/nagoya-flood-zones-real.geojson
  examples/nagoya-population-depth/data/real/nagoya-shelters-real.geojson  (see REAL_SHELTERS_PATH below)

Usage: python3 scripts/fetch-real-data.py
"""

import json
import pathlib
import sys
import urllib.request
import zipfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
DATA = ROOT / "examples/nagoya-population-density/data"
REAL = DATA / "real"
CACHE = ROOT / ".genegis/real-data"

A31A_URL = "https://nlftp.mlit.go.jp/ksj/gml/data/A31a/A31a-24/A31a-24_23_10_GEOJSON.zip"
SHELTER_URL = (
    "https://data.bodik.jp/dataset/5eda2cde-8fe9-485b-ab16-8e8bb55dbd46/resource/"
    "09d6b037-e48b-4e09-9bb0-07b94bdaa834/download/designated_evacuation_shelters-as_of_august_2025.zip"
)

# Nagoya wards bbox (matches catalog records, slightly padded).
NAGOYA = (136.78, 35.02, 137.08, 35.28)

# 浸水深ランク → our depth_class_m bands {0.5, 3.0, 5.0}.
RANK_TO_DEPTH = {1: 0.5, 2: 3.0, 3: 5.0, 4: 5.0}


def log(message):
    print(message, flush=True)


def download(url, target):
    if target.is_file() and target.stat().st_size > 0:
        log(f"cache hit: {target.name}")
        return target
    target.parent.mkdir(parents=True, exist_ok=True)
    log(f"downloading {url}")
    request = urllib.request.Request(url, headers={"User-Agent": "GeneGIS-real-data/1.0"})
    with urllib.request.urlopen(request, timeout=120) as response, target.open("wb") as out:
        out.write(response.read())
    return target


def unzip(archive, destination):
    destination.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(archive) as bundle:
        bundle.extractall(destination)


def iter_points(coordinates):
    if isinstance(coordinates[0], (int, float)):
        yield coordinates
        return
    for child in coordinates:
        yield from iter_points(child)


def round_ring(ring):
    return [[round(x, 6), round(y, 6)] for x, y in ring]


def convert_flood(extract_dir):
    """Category-20 (想定最大規模) polygons intersecting the Nagoya bbox."""
    category_dirs = [
        path for path in extract_dir.iterdir() if path.is_dir() and path.name.startswith("20_")
    ]
    if len(category_dirs) != 1:
        raise SystemExit(f"expected one 想定最大規模 dir, found {category_dirs}")

    features = []
    for path in sorted(category_dirs[0].glob("A31a-20-*.geojson")):
        payload = json.loads(path.read_text(encoding="utf-8"))
        for feature in payload.get("features", []):
            props = feature.get("properties", {})
            depth = RANK_TO_DEPTH.get(props.get("A31a_205"))
            if depth is None:
                continue  # rank 5 (範囲外) or missing
            geometry = feature.get("geometry") or {}
            if geometry.get("type") != "Polygon":
                continue
            rings = geometry["coordinates"]
            xs = [p[0] for ring in rings for p in iter_points(ring)]
            ys = [p[1] for ring in rings for p in iter_points(ring)]
            if not xs or max(xs) < NAGOYA[0] or min(xs) > NAGOYA[2]:
                continue
            if max(ys) < NAGOYA[1] or min(ys) > NAGOYA[3]:
                continue
            river = str(props.get("A31a_202", "河川")).strip()
            code = str(props.get("A31a_201", "river"))
            features.append({
                "type": "Feature",
                "properties": {
                    "zone_id": f"a31a-{code}-{len(features):05d}",
                    "name": f"{river}（想定最大規模 {depth}m帯）",
                    "depth_class_m": depth,
                    "source_rank": props.get("A31a_205"),
                },
                "geometry": {"type": "Polygon", "coordinates": [round_ring(r) for r in rings]},
            })

    payload = {
        "type": "FeatureCollection",
        "name": "nagoya-flood-zones-real",
        "crs": "EPSG:4326",
        "description": (
            "REAL 洪水浸水想定区域（想定最大規模） clipped to the Nagoya bbox. "
            "Source: 国土数値情報 洪水浸水想定区域（河川単位）A31a 愛知県 2024年度 "
            "(https://nlftp.mlit.go.jp/ksj/, CC-BY 4.0). Depth ranks mapped 1→0.5m, "
            "2→3m, 3/4→5m; rank 5 (範囲外) dropped. NOT a hazard-map substitute."
        ),
        "features": features,
    }
    REAL.mkdir(parents=True, exist_ok=True)
    out = REAL / "nagoya-flood-zones-real.geojson"
    out.write_text(json.dumps(payload, ensure_ascii=False, separators=(",", ":")))
    log(f"wrote {out} ({len(features)} zones, {out.stat().st_size/1e6:.1f} MB)")


def parse_dbf(path):
    data = path.read_bytes()
    count = int.from_bytes(data[4:8], "little")
    header_len = int.from_bytes(data[8:10], "little")
    record_len = int.from_bytes(data[10:12], "little")
    offset = 32
    fields = []
    while data[offset:offset + 1] != b"\r":
        name = data[offset:offset + 11].split(b"\0")[0].decode("cp932", "replace")
        fields.append((name, data[offset + 16]))
        offset += 32
    for index in range(count):
        record = data[header_len + index * record_len:header_len + (index + 1) * record_len]
        cursor = 1
        values = {}
        for name, width in fields:
            values[name] = record[cursor:cursor + width].decode("cp932", "replace").strip()
            cursor += width
        yield values


def load_ward_rings():
    wards = json.loads((DATA / "nagoya-wards.geojson").read_text())
    rings = []
    for feature in wards["features"]:
        geometry = feature["geometry"]
        polygons = (
            [geometry["coordinates"]]
            if geometry["type"] == "Polygon"
            else geometry["coordinates"]
        )
        parsed = [
            [[(c[0], c[1]) for c in ring] for ring in polygon]
            for polygon in polygons
        ]
        rings.append((feature["properties"]["ward_name"], parsed))
    return rings


def point_in_ring(x, y, ring):
    inside = False
    for i in range(len(ring)):
        x1, y1 = ring[i]
        x2, y2 = ring[(i + 1) % len(ring)]
        if (y1 > y) != (y2 > y):
            t = (y - y1) / (y2 - y1)
            if x < x1 + t * (x2 - x1):
                inside = not inside
    return inside


def ward_of(x, y, ward_rings):
    for name, polygons in ward_rings:
        for polygon in polygons:
            if point_in_ring(x, y, polygon[0]) and not any(
                point_in_ring(x, y, hole) for hole in polygon[1:]
            ):
                return name
    return None


def convert_shelters(extract_dir):
    dbf_files = list(extract_dir.rglob("指定避難所.dbf"))
    if len(dbf_files) != 1:
        raise SystemExit(f"expected one 指定避難所.dbf, found {dbf_files}")
    ward_rings = load_ward_rings()

    features = []
    unmatched = 0
    for record in parse_dbf(dbf_files[0]):
        try:
            lon = float(record["経度"])
            lat = float(record["緯度"])
        except (KeyError, ValueError):
            continue
        if not (NAGOYA[0] <= lon <= NAGOYA[2] and NAGOYA[1] <= lat <= NAGOYA[3]):
            unmatched += 1
            continue
        ward = ward_of(lon, lat, ward_rings)
        if ward is None:
            unmatched += 1
            continue
        name = record.get("施設名2") or f"指定避難所{record.get('修正通し番', '')}"
        features.append({
            "type": "Feature",
            "properties": {
                "shelter_id": f"real-{record.get('修正通し番', len(features)):0>4}",
                "name": f"{ward}{name}"[:60],
                "ward_name": ward,
                "kind": "designated_evacuation_shelter",
                "address": record.get("所在地2", ""),
            },
            "geometry": {"type": "Point", "coordinates": [round(lon, 6), round(lat, 6)]},
        })

    payload = {
        "type": "FeatureCollection",
        "name": "nagoya-shelters-real",
        "crs": "EPSG:4326",
        "description": (
            "REAL 名古屋市指定避難所（令和7年8月時点, 804 records） with wards "
            "assigned by point-in-polygon against the bundled N03 ward fixture. "
            "Source: 名古屋市 via BODIK CKAN (CC-BY 4.0)."
        ),
        "features": features,
    }
    REAL.mkdir(parents=True, exist_ok=True)
    out = REAL / "nagoya-shelters-real.geojson"
    out.write_text(json.dumps(payload, ensure_ascii=False, separators=(",", ":")))
    log(f"wrote {out} ({len(features)} shelters, {unmatched} skipped)")


def main():
    a31a_zip = download(A31A_URL, CACHE / "a31a-24-23.zip")
    shelter_zip = download(SHELTER_URL, CACHE / "nagoya-shelters.zip")
    a31a_dir = CACHE / "a31a"
    shelter_dir = CACHE / "shelters"
    if not (a31a_dir / "A31a-24_23_10_GeoJSON").is_dir():
        unzip(a31a_zip, a31a_dir)
    if not any(shelter_dir.rglob("指定避難所.dbf")):
        unzip(shelter_zip, shelter_dir)

    convert_flood(a31a_dir / "A31a-24_23_10_GeoJSON")
    convert_shelters(shelter_dir)

    import hashlib

    for name in ("nagoya-flood-zones-real.geojson", "nagoya-shelters-real.geojson"):
        path = REAL / name
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        log(f"sha256:{digest}  {path}")
    log(
        "\nRun the verified workflows on REAL data:\n"
        "  GENEGIS_FLOOD_ZONES_PATH=examples/nagoya-population-density/data/real/nagoya-flood-zones-real.geojson \\\n"
        "  GENEGIS_SHELTERS_PATH=examples/nagoya-population-density/data/real/nagoya-shelters-real.geojson \\\n"
        "  cargo run -p genegis-cli -- agent run \"名古屋市の洪水浸水リスクと避難所アクセシビリティを表示\""
    )


if __name__ == "__main__":
    sys.exit(main())
