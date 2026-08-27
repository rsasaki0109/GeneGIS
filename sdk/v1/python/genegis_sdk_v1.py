"""Dependency-free GeneGIS SDK v1 manifest binding."""

from dataclasses import dataclass
import re
from typing import Any

PLUGIN_API_VERSION = "1.0.0"
CAPABILITIES = frozenset({
    "read_catalog", "read_storage", "analysis_step", "render_hook",
    "export_artifact", "publish_stac",
})


@dataclass(frozen=True)
class PluginManifestV1:
    id: str
    version: str
    api_version: str
    capabilities: tuple[str, ...]
    artifact_digest: str
    name: str = ""
    description: str = ""
    author: str = ""
    wasm_entry: str | None = None

    @classmethod
    def from_dict(cls, value: dict[str, Any]) -> "PluginManifestV1":
        allowed = {"id", "name", "version", "api_version", "description", "author", "capabilities", "artifact_digest", "wasm"}
        if set(value) - allowed:
            raise ValueError("unknown manifest field")
        capabilities = tuple(value.get("capabilities", ()))
        wasm = value.get("wasm")
        manifest = cls(
            id=value.get("id", ""), version=value.get("version", ""),
            api_version=value.get("api_version", ""), capabilities=capabilities,
            artifact_digest=value.get("artifact_digest", ""),
            name=value.get("name", ""), description=value.get("description", ""),
            author=value.get("author", ""),
            wasm_entry=wasm.get("entry") if isinstance(wasm, dict) else None,
        )
        manifest.validate()
        return manifest

    def validate(self) -> None:
        if not re.fullmatch(r"[a-z0-9-]+", self.id): raise ValueError("invalid plugin id")
        if not re.fullmatch(r"\d+\.\d+\.\d+", self.version): raise ValueError("invalid version")
        if not re.fullmatch(r"1\.\d+\.\d+", self.api_version): raise ValueError("unsupported API version")
        if not self.capabilities or len(set(self.capabilities)) != len(self.capabilities) or any(value not in CAPABILITIES for value in self.capabilities): raise ValueError("invalid capabilities")
        if not re.fullmatch(r"sha256:[0-9a-fA-F]{64}", self.artifact_digest): raise ValueError("invalid artifact digest")
        if self.wasm_entry is not None and not self.wasm_entry.endswith(".wasm"): raise ValueError("invalid WASM entry")
