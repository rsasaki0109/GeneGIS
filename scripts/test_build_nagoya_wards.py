"""Offline regression tests for the Nagoya boundary builder."""

from __future__ import annotations

import importlib.util
import pathlib
import unittest


SCRIPT = pathlib.Path(__file__).with_name("build-nagoya-wards.py")
SPEC = importlib.util.spec_from_file_location("build_nagoya_wards", SCRIPT)
assert SPEC and SPEC.loader
BUILDER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BUILDER)


def ring(x0: float, y0: float, x1: float, y1: float) -> list[list[float]]:
    return [[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]]


class BuilderRegressionTests(unittest.TestCase):
    def ward(self) -> dict:
        return {
            "ward_code": "23111",
            "ward_name": "港区",
            "ward_name_en": "Minato",
            "population": 143715,
        }

    def population(self) -> dict:
        return {
            "census_year": 2020,
            "source": "test",
            "source_url": "https://example.test/page",
            "source_data_url": "https://example.test/data.xlsx",
            "license": "test",
            "retrieval_basis": "test fixture",
            "source_version": "test-v1",
        }

    def test_missing_features_rejected(self) -> None:
        with self.assertRaises(ValueError):
            BUILDER.build_ward_feature(self.ward(), {"features": []}, self.population())

    def test_mismatched_property_rejected(self) -> None:
        geo = {
            "features": [
                {
                    "properties": {"N03_007": "23110", "N03_004": "中川区"},
                    "geometry": {"type": "Polygon", "coordinates": [[ring(0, 0, 1, 1)]]},
                }
            ]
        }
        with self.assertRaisesRegex(ValueError, "mismatched ward code"):
            BUILDER.build_ward_feature(self.ward(), geo, self.population())

    def test_multipart_and_valid_hole_are_retained(self) -> None:
        geo = {
            "features": [
                {
                    "properties": {"N03_007": "23111", "N03_004": "港区"},
                    "geometry": {
                        "type": "MultiPolygon",
                        "coordinates": [
                            [ring(0, 0, 10, 10), ring(2, 2, 4, 4)],
                            [ring(20, 20, 21, 21)],
                        ],
                    },
                }
            ]
        }
        feature = BUILDER.build_ward_feature(self.ward(), geo, self.population())
        coordinates = feature["geometry"]["coordinates"]
        self.assertEqual(len(coordinates), 2)
        self.assertEqual(len(coordinates[0]), 2)
        self.assertEqual(len(coordinates[1]), 1)

    def test_malformed_hole_shell_is_split_without_loss(self) -> None:
        geo = {
            "features": [
                {
                    "properties": {"N03_007": "23111", "N03_004": "港区"},
                    "geometry": {
                        "type": "MultiPolygon",
                        "coordinates": [[ring(0, 0, 1, 1), ring(10, 10, 11, 11)]],
                    },
                }
            ]
        }
        feature = BUILDER.build_ward_feature(self.ward(), geo, self.population())
        self.assertEqual(len(feature["geometry"]["coordinates"]), 2)
        self.assertEqual(feature["properties"]["boundary_hole_count"], 0)


if __name__ == "__main__":
    unittest.main()
