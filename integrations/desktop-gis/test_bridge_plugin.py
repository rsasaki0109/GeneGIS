import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

MODULE_PATH = Path(__file__).with_name("genegis_bridge_plugin.py")
SPEC = importlib.util.spec_from_file_location("genegis_bridge_plugin", MODULE_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(MODULE)


class BridgePluginTests(unittest.TestCase):
    def request(self, source: Path):
        return {
            "project_name": "Desktop transfer",
            "desktop_host": "desktop-gis-test/1.0",
            "layers": [{
                "id": "wards",
                "name": "Wards",
                "kind": "vector",
                "format": "geo_json",
                "source_path": str(source),
                "crs": "EPSG:4326",
                "coordinate_unit": "degrees",
                "license": "CC BY 4.0",
                "expected_checksum": None,
                "extent": ["136", "35", "137", "36"],
                "temporal_interval": None,
            }],
        }

    def test_prepares_only_selected_file_with_observed_digest(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "wards.geojson"
            source.write_text(json.dumps({"type": "FeatureCollection", "features": []}), encoding="utf-8")
            prepared = MODULE.prepare_request(self.request(source))
            layer = prepared["layers"][0]
            self.assertTrue(layer["expected_checksum"].startswith("sha256:"))
            self.assertEqual(Path(layer["source_path"]), source.resolve())

    def test_rejects_checksum_drift_and_undeclared_fields(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "wards.geojson"
            source.write_text("{}", encoding="utf-8")
            request = self.request(source)
            request["layers"][0]["expected_checksum"] = "sha256:" + "0" * 64
            with self.assertRaises(MODULE.BridgeContractError):
                MODULE.prepare_request(request)
            request = self.request(source)
            request["credentials"] = "forbidden"
            with self.assertRaises(MODULE.BridgeContractError):
                MODULE.prepare_request(request)


if __name__ == "__main__":
    unittest.main()
