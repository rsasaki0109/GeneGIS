# Plugin Author Guide (Phase 4 alpha)

GeneGIS plugins are **WASM modules** shipped with a JSON manifest. The host loads manifests first, applies a capability allow-list (RFC D7), and only then compiles the `.wasm` file.

## Bundle layout

```
plugins/my-plugin/
├── genegis.plugin.json   # required manifest (see below)
└── my_plugin.wasm        # compiled module (relative path from manifest)
```

The repository includes a smoke bundle at `GeneGIS/plugins/demo-filter/`.

## Manifest (`genegis.plugin.json`)

| Field | Required | Description |
|-------|----------|-------------|
| `id` | yes | Stable kebab-case identifier (`demo-filter`) |
| `version` | yes | Semver `major.minor.patch` |
| `api_version` | yes | Must match host `PLUGIN_API_VERSION` (`0.1.x`) |
| `capabilities` | yes | Non-empty list of granted capabilities |
| `name` | no | Display name for workbench / CLI |
| `description` | no | Short summary |
| `author` | no | Author or organization |
| `wasm.entry` | no | Relative path to `.wasm` (required for load smoke) |

Example:

```json
{
  "id": "demo-filter",
  "name": "Demo Filter",
  "version": "0.1.0",
  "api_version": "0.1.0",
  "description": "Example analysis filter plugin",
  "author": "GeneGIS",
  "capabilities": ["analysis_step"],
  "wasm": { "entry": "demo_filter.wasm" }
}
```

Validate in Rust:

```rust
use genegis_plugin_api::{PluginManifest, PLUGIN_API_VERSION};

let manifest = PluginManifest::parse_and_validate(json)?;
assert_eq!(manifest.api_version, PLUGIN_API_VERSION);
```

## Capabilities

| Capability | String | Typical use |
|------------|--------|-------------|
| `ReadCatalog` | `read_catalog` | Read dataset metadata from `genegis-catalog` |
| `ReadStorage` | `read_storage` | HTTP range-read via `genegis-storage` |
| `AnalysisStep` | `analysis_step` | Register or run workflow analysis steps |
| `RenderHook` | `render_hook` | Choropleth / tile render hooks |
| `ExportArtifact` | `export_artifact` | Export maps or tabular outputs |
| `PublishStac` | `publish_stac` | Emit STAC items from catalog assets |

Request only what you need. The host intersects manifest capabilities with its policy:

```rust
use genegis_plugin_api::{CapabilityPolicy, PluginCapability};

let policy = CapabilityPolicy::read_only(); // catalog + storage only
policy.require_capability(&manifest, PluginCapability::ReadCatalog)?;
```

## Host smoke tests (CLI)

From the repository root:

```bash
genegis plugin list
genegis plugin info plugins/demo-filter
genegis plugin load plugins/demo-filter
```

`list` / `info` require a valid manifest. `load` additionally compiles the WASM module through Wasmtime after capability gating.

## Workbench listing

`cargo run -p genegis-workbench` serves `GET /api/plugins`, which the shared desktop UI renders in the **Plugins** sidebar. Tauri desktop exposes the same payload via `list_plugins`.

Default plugin root: `./plugins`, then the repository `plugins/` directory when running from a crate subdirectory.

## WASM authoring (alpha limits)

Phase 4 alpha validates **manifest + module load** only. There is no stable plugin export ABI yet — do not rely on host function imports beyond future SDK releases.

Recommended workflow for authors today:

1. Author and validate `genegis.plugin.json`.
2. Compile a minimal WASM module (smoke export optional).
3. Run `genegis plugin load` locally.
4. List the bundle in workbench to confirm discovery.

## Version contract

- Host SDK version: `genegis_plugin_api::PLUGIN_API_VERSION` (`0.1.0`).
- Compatible manifests: same `major.minor` (`0.1.x`).
- Manifest filename: `genegis.plugin.json`.

## Out of scope (Phase 4)

- Plugin marketplace or billing
- Native (non-WASM) plugins
- TypeScript UI extensions and Python sandboxes (future tracks)

## Related code

| Crate / path | Role |
|--------------|------|
| `crates/genegis-plugin-api` | Manifest schema, capabilities, policy |
| `crates/genegis-plugin-host` | Discovery + Wasmtime loader |
| `plugins/demo-filter/` | Reference bundle |
| `docs/roadmap/phase-4-plugins.md` | Phase 4 deliverables |

## Next steps for authors

- Watch for a stable WASM export surface (`plugin_info`, analysis hooks).
- Keep manifests minimal until the host ABI is frozen.
- Open issues with reproducible `genegis plugin load` logs when sandboxing blocks expected capabilities.
