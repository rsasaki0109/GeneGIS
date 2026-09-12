#!/usr/bin/env python3
"""Fetch REAL 気象庁 AMeDAS observations for the Nagoya station (51106).

Source:
  - 気象庁 AMeDAS 観測データ JSON (10-min observations)
    https://www.jma.go.jp/bosai/amedas/data/point/51106/{yyyyMMddHHmmss}.json
  - License: 気象庁ホームページ利用規約（政府標準利用規約準拠）

This script resolves the latest observation time, downloads the AMeDAS point
JSON for 名古屋 (51106), and emits a deterministic GeoJSON observation snapshot
with temperature, humidity, precipitation, pressure, and wind (values carry the
observation quality flag from the source).

Output:
  examples/nagoya-population-density/data/real/nagoya-amedas-live.geojson

The snapshot schema (station_id, observed_at, values with unit + quality) fits
the H3.1 live-feed cursor/watermark contract; wiring a continuously refreshing
adapter remains adapter scope (see docs/rfcs/0006-open-data-expansion.md).

Usage: python3 scripts/fetch-jma-amedas.py
"""

from __future__ import annotations

import json
import pathlib
import sys
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
REAL = ROOT / "examples/nagoya-population-density/data/real"
CACHE = ROOT / ".genegis/real-data/jma"

NAGOYA_STATION = "51106"
NAGOYA_POINT = (136.97, 35.17)  # 名古屋 (lon, lat)

POINT_URL = "https://www.jma.go.jp/bosai/amedas/data/map/{time}.json"
LATEST_URL = "https://www.jma.go.jp/bosai/amedas/data/latest_time.txt"


def log(message):
    print(message, flush=True)


def fetch(url):
    request = urllib.request.Request(url, headers={"User-Agent": "GeneGIS-real-data/1.0"})
    with urllib.request.urlopen(request, timeout=60) as response:
        return response.read().decode("utf-8")


def main() -> int:
    latest = fetch(LATEST_URL).strip()
    # "2026-09-12T07:40:00+09:00" -> "20260912074000"
    import re

    compact = re.sub(r"[-:T+]", "", latest.split("+")[0])
    if len(compact) != 14:
        raise SystemExit(f"unexpected latest_time format: {latest!r}")
    payload = json.loads(fetch(POINT_URL.format(time=compact)))
    station = payload.get(NAGOYA_STATION)
    if station is None:
        raise SystemExit(f"station {NAGOYA_STATION} missing from the AMeDAS map payload")

    observed_at = latest
    # Key per-element values carrying a quality flag as the second element.
    mapped = {}
    for key in ("temp", "humidity", "precipitation1h", "pressure", "wind"):
        raw = station.get(key)
        if isinstance(raw, list) and len(raw) >= 1 and raw[0] is not None:
            mapped[key] = {"value": raw[0], "quality": raw[1] if len(raw) > 1 else 0}

    if not mapped:
        raise SystemExit("no usable AMeDAS observations in the point payload")

    features = [{
        "type": "Feature",
        "properties": {
            "station_id": NAGOYA_STATION,
            "station_name": "名古屋",
            "observed_at": observed_at,
            "values": mapped,
        },
        "geometry": {"type": "Point", "coordinates": [NAGOYA_POINT[0], NAGOYA_POINT[1]]},
    }]

    REAL.mkdir(parents=True, exist_ok=True)
    out = REAL / "nagoya-amedas-live.geojson"
    collection = {
        "type": "FeatureCollection",
        "name": "nagoya-amedas-live",
        "crs": "EPSG:4326",
        "description": (
            "REAL 気象庁 AMeDAS 名古屋 (51106) point observation at the latest "
            "available time. Source: 気象庁防災情報 JSON (気象庁ホームページ利用規約). "
            "Observations are weather data, not hazard decisions."
        ),
        "features": features,
    }
    out.write_text(json.dumps(collection, ensure_ascii=False, separators=(",", ":")), encoding="utf-8")
    import hashlib

    digest = hashlib.sha256(out.read_bytes()).hexdigest()
    log(f"wrote {out} ({observed_at})")
    log(f"sha256:{digest}  {out}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"fetch-jma-amedas: {error}", file=sys.stderr)
        sys.exit(1)