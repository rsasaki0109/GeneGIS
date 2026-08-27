# Plugin Author Guide (SDK v1)

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
| `api_version` | yes | Must share host `PLUGIN_API_VERSION` major (`1.x`) |
| `artifact_digest` | yes | SHA-256 identity of the distributed artifact |
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
  "api_version": "1.0.0",
  "description": "Example analysis filter plugin",
  "author": "GeneGIS",
  "capabilities": ["analysis_step"],
  "artifact_digest": "sha256:93a44bbb96c751218e4c00d479e4c14358122a389acca16205b1e4d0dc5f9476",
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

## WASM authoring

The stable component boundary is `sdk/v1/wasm/genegis-plugin-v1.wit`. Plugins
may read declared inputs and emit Commands; they do not mutate projects directly.

Recommended workflow for authors today:

1. Author and validate `genegis.plugin.json`.
2. Compile a minimal WASM module (smoke export optional).
3. Run `genegis plugin load` locally.
4. List the bundle in workbench to confirm discovery.

## Version contract

- Host SDK version: `genegis_plugin_api::PLUGIN_API_VERSION` (`1.0.0`).
- Compatible manifests: same non-zero major (`1.x`).
- Manifest filename: `genegis.plugin.json`.

## Signed registry

Registry publication requires an Ed25519 signature from a configured trusted
key. The host re-verifies manifest signature and artifact SHA-256 at resolution.
A signed revocation immediately prevents resolution of that exact release.

TypeScript and Python bindings plus the shared conformance fixture are under
`sdk/v1/`; Rust remains the reference implementation.

## Out of scope

- Native in-process plugins
- Billing and marketplace economics

## Related code

| Crate / path | Role |
|--------------|------|
| `crates/genegis-plugin-api` | Manifest schema, capabilities, policy |
| `crates/genegis-plugin-host` | Discovery + Wasmtime loader |
| `plugins/demo-filter/` | Reference bundle |
| `docs/roadmap/phase-4-plugins.md` | Phase 4 deliverables |

## Next steps for authors

- Implement the SDK v1 WIT exports and request only required capabilities.
- Run the shared Rust/Python conformance corpus before publication.
- Open issues with reproducible `genegis plugin load` logs when sandboxing blocks expected capabilities.
