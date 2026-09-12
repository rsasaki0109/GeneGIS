#!/usr/bin/env python3
"""Build the deterministic synthetic transit corridor fixture.

This mirrors the real-data schema produced by ``scripts/fetch-mlit-transit.py``
(N02 rail / N07 bus corridors) so the same ``load_transit_corridors`` path and
the same multimodal verifier cover both the offline fixture and the licensed
real transit data.

The fixture models three Nagoya rail corridors (名古屋駅 area → 金山 → 千種 →
池下, plus a south line 名古屋 → 熱田 and a north line 名古屋 → 大曽根) as
line corridors between stops. Consecutive stop pairs become ride edges in the
multimodal graph. The walk network and POI fixtures are unchanged, so the
walk-only and multimodal accessibility modes share one verifier vocabulary.

Output:
  examples/nagoya-population-density/data/nagoya-transit.geojson
"""

from __future__ import annotations

import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
DATA = ROOT / "examples" / "nagoya-population-density" / "data"
OUT_PATH = DATA / "nagoya-transit.geojson"

# Corridors: (line, mode, [(stop_id, lon, lat), ...]).
CORRIDORS = [
    (
        "meitetsu-nagoya-main",
        "rail",
        [
            ("kanayama", 136.9022, 35.1436),
            ("nagoya-station", 136.8815, 35.1707),
            ("jyoshi", 136.9058, 35.1764),
            ("chikusa", 136.9308, 35.1702),
            ("ikeshita", 136.9454, 35.1696),
        ],
    ),
    (
        "meijo-line",
        "rail",
        [
            ("nagoya-station", 136.8815, 35.1707),
            ("atsuta", 136.9104, 35.1192),
        ],
    ),
    (
        "hokusei-line",
        "rail",
        [
            ("nagoya-station", 136.8815, 35.1707),
            ("ozone", 136.9218, 35.1968),
        ],
    ),
]

STOPS: dict[str, tuple[float, float]] = {}
for _line, _mode, stops in CORRIDORS:
    for stop_id, lon, lat in stops:
        STOPS[stop_id] = (lon, lat)


def main() -> int:
    features = []
    for line, mode, stops in CORRIDORS:
        coords = [[STOPS[stop_id][0], STOPS[stop_id][1]] for stop_id, _, _ in stops]
        features.append(
            {
                "type": "Feature",
                "properties": {
                    "line": line,
                    "mode": mode,
                    "wait_minutes": 6.0,
                    "ride_minutes": 2.0,
                },
                "geometry": {"type": "LineString", "coordinates": coords},
            }
        )
    for stop_id, (lon, lat) in STOPS.items():
        lines = [
            line
            for line, _mode, stops in CORRIDORS
            if any(sid == stop_id for sid, _, _ in stops)
        ]
        features.append(
            {
                "type": "Feature",
                "properties": {
                    "kind": "stop",
                    "id": stop_id,
                    "lines": ",".join(lines),
                },
                "geometry": {"type": "Point", "coordinates": [lon, lat]},
            }
        )

    collection = {
        "type": "FeatureCollection",
        "name": "nagoya-transit",
        "crs": "EPSG:4326",
        "description": (
            "SYNTHETIC deterministic transit corridors over the Nagoya walk "
            "grid (rail mode, ~2 min between stops). NOT real MLIT N02/N07 data; "
            "use scripts/fetch-mlit-transit.py for the licensed real corridors."
        ),
        "features": features,
    }
    OUT_PATH.write_text(
        json.dumps(collection, ensure_ascii=False, separators=(",", ":")),
        encoding="utf-8",
    )
    print(f"Wrote {OUT_PATH} ({len(CORRIDORS)} corridors, {len(STOPS)} stops)")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        print(f"build-nagoya-transit: {error}", file=sys.stderr)
        sys.exit(1)