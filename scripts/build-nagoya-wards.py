#!/usr/bin/env python3
"""Build nagoya-wards.geojson from N03 ward boundaries + e-Stat population."""

from __future__ import annotations

import json
import sys
import urllib.request
from pathlib import Path

BASE = "https://raw.githubusercontent.com/niiyz/JapanCityGeoJson/master/geojson/23"
ROOT = Path(__file__).resolve().parents[1]
DATA = ROOT / "examples" / "nagoya-population-density" / "data"
POP_PATH = DATA / "nagoya-population-2020.json"
OUT_PATH = DATA / "nagoya-wards.geojson"


def fetch_ward(code: str) -> dict:
    url = f"{BASE}/{code}.json"
    with urllib.request.urlopen(url, timeout=60) as resp:
        return json.load(resp)


def main() -> int:
    population = json.loads(POP_PATH.read_text(encoding="utf-8"))
    pop_by_code = {w["ward_code"]: w for w in population["wards"]}

    features = []
    for ward in population["wards"]:
        code = ward["ward_code"]
        geo = fetch_ward(code)
        src_feature = geo["features"][0]
        props = src_feature.get("properties", {})
        features.append(
            {
                "type": "Feature",
                "properties": {
                    "ward_code": code,
                    "ward_name": ward["ward_name"],
                    "ward_name_en": ward["ward_name_en"],
                    "population": ward["population"],
                    "census_year": population["census_year"],
                    "prefecture": props.get("N03_001", "愛知県"),
                    "city": props.get("N03_003", "名古屋市"),
                    "boundary_source": "MLIT N03 via JapanCityGeoJson",
                    "boundary_url": f"{BASE}/{code}.json",
                    "population_source": population["source"],
                    "population_source_url": population["source_url"],
                },
                "geometry": src_feature["geometry"],
            }
        )

    collection = {
        "type": "FeatureCollection",
        "name": "nagoya-wards",
        "crs": "EPSG:4326",
        "features": features,
    }
    OUT_PATH.write_text(
        json.dumps(collection, ensure_ascii=False, separators=(",", ":")),
        encoding="utf-8",
    )
    print(f"Wrote {OUT_PATH} ({len(features)} wards)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
