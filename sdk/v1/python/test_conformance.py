import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).parent))
from genegis_sdk_v1 import PluginManifestV1


class ConformanceTest(unittest.TestCase):
    def test_shared_fixture(self) -> None:
        fixture = Path(__file__).parents[1] / "conformance" / "valid-plugin.json"
        manifest = PluginManifestV1.from_dict(json.loads(fixture.read_text(encoding="utf-8")))
        self.assertEqual(manifest.api_version, "1.0.0")

    def test_unknown_field_and_capability_fail_closed(self) -> None:
        base = {
            "id": "plugin", "version": "1.0.0", "api_version": "1.0.0",
            "capabilities": ["analysis_step"], "artifact_digest": "sha256:" + "0" * 64,
        }
        with self.assertRaises(ValueError):
            PluginManifestV1.from_dict({**base, "secret": "leak"})
        with self.assertRaises(ValueError):
            PluginManifestV1.from_dict({**base, "capabilities": ["admin"]})


if __name__ == "__main__":
    unittest.main()
