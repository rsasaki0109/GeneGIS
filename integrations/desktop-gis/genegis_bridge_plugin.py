"""Thin provider-neutral desktop GIS client for GeneGIS bridge capsules."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any
from urllib.request import Request, urlopen

PROTOCOL_VERSION = "0.1.0"
ALLOWED_FORMATS = {"geo_json", "geo_parquet", "cog", "copc", "pm_tiles"}


class BridgeContractError(ValueError):
    """The desktop host supplied an unsafe or incomplete layer declaration."""


def sha256_file(path: Path) -> str:
    """Return a prefixed SHA-256 for exact exported layer bytes."""
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return f"sha256:{digest.hexdigest()}"


def prepare_request(document: dict[str, Any]) -> dict[str, Any]:
    """Validate selected layers and attach observed content checksums."""
    if set(document) != {"project_name", "desktop_host", "layers"}:
        raise BridgeContractError("request must contain project_name, desktop_host, and layers only")
    if not str(document["project_name"]).strip() or not str(document["desktop_host"]).strip():
        raise BridgeContractError("project_name and desktop_host are required")
    layers = document["layers"]
    if not isinstance(layers, list) or not layers:
        raise BridgeContractError("at least one explicitly selected layer is required")

    prepared = json.loads(json.dumps(document))
    for layer in prepared["layers"]:
        required = {
            "id", "name", "kind", "format", "source_path", "crs",
            "coordinate_unit", "license", "expected_checksum", "extent",
            "temporal_interval",
        }
        if set(layer) != required:
            raise BridgeContractError(f"layer must contain exactly: {sorted(required)}")
        if layer["format"] not in ALLOWED_FORMATS:
            raise BridgeContractError(f"unsupported layer format: {layer['format']}")
        if not str(layer["license"]).strip():
            raise BridgeContractError("every selected layer requires a license or attribution")
        source = Path(layer["source_path"]).expanduser().resolve(strict=True)
        if not source.is_file():
            raise BridgeContractError(f"selected layer is not a regular file: {source}")
        observed = sha256_file(source)
        expected = layer.get("expected_checksum")
        if expected and expected != observed:
            raise BridgeContractError(f"selected layer checksum mismatch: {layer['id']}")
        layer["source_path"] = str(source)
        layer["expected_checksum"] = observed
    return prepared


def push_selected_layers(
    document: dict[str, Any],
    endpoint: str = "http://127.0.0.1:7812/api/bridge/capsule",
    timeout_seconds: float = 30.0,
) -> dict[str, Any]:
    """Send only declared layers to the local GeneGIS workbench."""
    payload = json.dumps(prepare_request(document), separators=(",", ":")).encode("utf-8")
    request = Request(endpoint, data=payload, headers={"Content-Type": "application/json"}, method="POST")
    with urlopen(request, timeout=timeout_seconds) as response:
        result = json.load(response)
    if not result.get("ok"):
        raise BridgeContractError(result.get("error") or "GeneGIS rejected bridge request")
    return result


class GeneGISDesktopBridgePlugin:
    """Small host-facing adapter suitable for a desktop plugin action."""

    protocol_version = PROTOCOL_VERSION

    def __init__(self, endpoint: str = "http://127.0.0.1:7812/api/bridge/capsule") -> None:
        self.endpoint = endpoint

    def push_selected_layers(self, project_name: str, desktop_host: str, layers: list[dict[str, Any]]) -> dict[str, Any]:
        """Create one reviewable bridge capsule from explicit host selections."""
        return push_selected_layers(
            {"project_name": project_name, "desktop_host": desktop_host, "layers": layers},
            self.endpoint,
        )


def main() -> int:
    parser = argparse.ArgumentParser(description="Push selected desktop GIS layers into a GeneGIS bridge capsule")
    parser.add_argument("request", type=Path, help="Strict bridge request JSON")
    parser.add_argument("--endpoint", default="http://127.0.0.1:7812/api/bridge/capsule")
    args = parser.parse_args()
    document = json.loads(args.request.read_text(encoding="utf-8"))
    print(json.dumps(push_selected_layers(document, args.endpoint), ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
