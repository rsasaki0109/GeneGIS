#!/usr/bin/env python3
"""Fetch the REAL OSM walk network and POIs for Nagoya (platform gap #4).

Sources:
  - OpenStreetMap via Overpass API (mirrors tried in order). Highways
    restricted to walkable classes; geometry via `out geom`. ODbL license —
    attribution required: "© OpenStreetMap contributors".
  - POIs: amenity/leisure nodes mapped onto the four fixture categories
    (supermarket / clinic / school / park).

Outputs:
  examples/nagoya-population-density/data/real/nagoya-walk-network-real.geojson
  examples/nagoya-population-density/data/real/nagoya-pois-real.geojson

Both are direct inputs for GENEGIS_WALK_NETWORK_PATH / GENEGIS_POIS_PATH.
Usage: python3 scripts/fetch-osm-network.py
"""

import json
import pathlib
import sys
import time
import urllib.parse
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
DATA = ROOT / "examples/nagoya-population-density/data"
REAL = DATA / "real"
CACHE = ROOT / ".genegis/real-data"

# Nagoya bbox (matches the catalog records).
BBOX = "35.02,136.78,35.28,137.08"

WALK_HIGHWAYS = (
    "residential|living_street|unclassified|service|footway|path|pedestrian|"
    "steps|track|tertiary|secondary"
)

NETWORK_QUERY = f"""[out:json][timeout:300];
way["highway"~"^({WALK_HIGHWAYS})$"]({BBOX});
out geom;"""

POI_QUERY = f"""[out:json][timeout:180];
(
  node["amenity"~"^(supermarket|clinic|doctors|school|kindergarten)$"]({BBOX});
  node["shop"~"^(supermarket|convenience)$"]({BBOX});
  node["leisure"="park"]({BBOX});
  way["leisure"="park"]({BBOX});
);
out geom;"""

MIRRORS = [
    "https://maps.mail.ru/osm/tools/overpass/api/interpreter",
    "https://overpass.kumi.systems/api/interpreter",
    "https://overpass-api.de/api/interpreter",
]

CATEGORY_MAP = {
    "supermarket": "supermarket",
    "clinic": "clinic",
    "doctors": "clinic",
    "school": "school",
    "kindergarten": "school",
    "park": "park",
}

WALK_SPEED_M_PER_MIN = 80.0
METERS_PER_DEG_LAT = 111_320.0


def log(message):
    print(message, flush=True)


def overpass(query):
    body = urllib.parse.urlencode({"data": query}).encode()
    last_error = None
    for mirror in MIRRORS:
        for attempt in range(2):
            try:
                request = urllib.request.Request(
                    mirror,
                    data=body,
                    headers={"User-Agent": "GeneGIS-real-data/1.0 (OSM walk network)"},
                )
                log(f"GET {mirror} (attempt {attempt + 1})")
                with urllib.request.urlopen(request, timeout=600) as response:
                    return json.loads(response.read())
            except Exception as error:  # noqa: BLE001 — mirror fallback by design
                last_error = error
                log(f"  failed: {error}")
                time.sleep(5 * (attempt + 1))
    raise SystemExit(f"all Overpass mirrors failed: {last_error}")


def haversine_m(a, b):
    import math

    dlon = (b[0] - a[0]) * METERS_PER_DEG_LAT * math.cos(math.radians((a[1] + b[1]) / 2))
    dlat = (b[1] - a[1]) * METERS_PER_DEG_LAT
    return (dlon * dlon + dlat * dlat) ** 0.5


def build_network(elements):
    features = []
    total_km = 0.0
    for element in elements:
        if element.get("type") != "way":
            continue
        tags = element.get("tags", {})
        kind = tags.get("highway")
        geometry = element.get("geometry") or []
        coords = [
            [round(point["lon"], 6), round(point["lat"], 6)]
            for point in geometry
            if "lon" in point and "lat" in point
        ]
        if len(coords) < 2:
            continue
        length_m = sum(
            haversine_m(coords[i], coords[i + 1]) for i in range(len(coords) - 1)
        )
        if length_m <= 0.0:
            continue
        total_km += length_m / 1000.0
        features.append({
            "type": "Feature",
            "properties": {
                "kind": kind,
                "osm_way_id": element.get("id"),
                "length_m": round(length_m, 1),
            },
            "geometry": {"type": "LineString", "coordinates": coords},
        })
    payload = {
        "type": "FeatureCollection",
        "name": "nagoya-walk-network-real",
        "crs": "EPSG:4326",
        "description": (
            "REAL OSM walkable highways over the Nagoya bbox "
            "(residential/living_street/unclassified/service/footway/path/"
            "pedestrian/steps/track/tertiary/secondary). Source: "
            "© OpenStreetMap contributors (ODbL) via Overpass API. "
            "Walking speed assumed 80 m/min; not a routing-grade extract."
        ),
        "walk_speed_m_per_min": WALK_SPEED_M_PER_MIN,
        "features": features,
    }
    REAL.mkdir(parents=True, exist_ok=True)
    out = REAL / "nagoya-walk-network-real.geojson"
    out.write_text(json.dumps(payload, ensure_ascii=False, separators=(",", ":")))
    log(f"wrote {out} ({len(features)} ways, {total_km:.0f} km, {out.stat().st_size/1e6:.1f} MB)")


def build_pois(elements):
    features = []
    seen = set()
    for element in elements:
        tags = element.get("tags", {})
        raw = tags.get("amenity") or tags.get("shop") or (
            "park" if tags.get("leisure") == "park" else None
        )
        category = CATEGORY_MAP.get(raw)
        if category is None:
            continue
        if element.get("type") == "node":
            lon, lat = element.get("lon"), element.get("lat")
        elif element.get("type") == "way":
            geometry = [p for p in (element.get("geometry") or []) if "lon" in p]
            if not geometry:
                continue
            lon = sum(p["lon"] for p in geometry) / len(geometry)
            lat = sum(p["lat"] for p in geometry) / len(geometry)
        else:
            continue
        if lon is None or lat is not None and not (35.0 <= lat <= 35.3):
            continue
        lon = round(lon, 6)
        lat = round(lat, 6)
        key = (category, lon, lat)
        if key in seen:
            continue
        seen.add(key)
        name = tags.get("name") or f"{category}-{element.get('id')}"
        features.append({
            "type": "Feature",
            "properties": {
                "poi_id": f"osm-{element.get('type', 'n')}-{element.get('id')}",
                "category": category,
                "name": name[:60],
            },
            "geometry": {"type": "Point", "coordinates": [lon, lat]},
        })
    payload = {
        "type": "FeatureCollection",
        "name": "nagoya-pois-real",
        "crs": "EPSG:4326",
        "description": (
            "REAL OSM amenity/leisure nodes mapped to fixture categories "
            "(supermarket, clinic←clinic/doctors, school←school/kindergarten, "
            "park←leisure=park). Source: © OpenStreetMap contributors (ODbL)."
        ),
        "categories": ["supermarket", "clinic", "school", "park"],
        "features": features,
    }
    REAL.mkdir(parents=True, exist_ok=True)
    out = REAL / "nagoya-pois-real.geojson"
    out.write_text(json.dumps(payload, ensure_ascii=False, separators=(",", ":")))
    log(f"wrote {out} ({len(features)} POIs, {out.stat().st_size/1e6:.1f} MB)")


def main():
    CACHE.mkdir(parents=True, exist_ok=True)

    network_payload = overpass(NETWORK_QUERY)
    build_network(network_payload.get("elements", []))

    poi_payload = overpass(POI_QUERY)
    build_pois(poi_payload.get("elements", []))

    import hashlib

    for name in ("nagoya-walk-network-real.geojson", "nagoya-pois-real.geojson"):
        path = REAL / name
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        log(f"sha256:{digest}  {path}")
    log(
        "\nRun verified workflows on the REAL network:\n"
        "  export GENEGIS_WALK_NETWORK_PATH=examples/nagoya-population-density/data/real/nagoya-walk-network-real.geojson\n"
        "  export GENEGIS_WALK_NETWORK_SHA=<printed sha256>\n"
        "  export GENEGIS_POIS_PATH=examples/nagoya-population-density/data/real/nagoya-pois-real.geojson\n"
        "  export GENEGIS_POIS_SHA=<printed sha256>"
    )


if __name__ == "__main__":
    sys.exit(main())
