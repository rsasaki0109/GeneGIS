# Phase 4: Plugins & COPC

**Goal:** Open the platform to extensions and cloud-native point clouds — WASM plugin SDK, host sandbox, and COPC alpha read path.

**Star target:** 5,000 → 7,500

## Tracks

| Track | Phase 4 focus |
|-------|-----------------|
| **Data** | COPC smoke read (`genegis-pointcloud`); COPC over HTTP range-read |
| **Plugins** | Capability-based plugin API; WASM host loader; workbench extension hook |
| **Catalog** | Second discoverable dataset + planner tag wiring (beyond Nagoya) |
| **Cloud** | GeoParquet URI streaming polish (reuse `genegis-storage` + `genegis-vector`) |
| **Docs** | Plugin author guide; COPC example under `examples/` |

## Deliverables

- [x] Phase 4 roadmap (this document)
- [x] COPC metadata smoke read (`genegis-pointcloud`, `copc-streaming`, CLI `genegis pointcloud info`)
- [x] COPC HTTP range-read (`read_copc_uri` + `HttpByteSource`, `read_mode: "http_range"`)
- [x] Plugin capability model (`genegis-plugin-api`: `PluginManifest`, `PluginCapability`, `CapabilityPolicy`)
- [x] WASM plugin host prototype (`genegis-plugin-host`, Wasmtime loader, CLI `genegis plugin list|info|load`)
- [x] Workbench plugin panel stub (`apps/workbench` `/api/plugins`, shared desktop UI sidebar)
- [ ] Second catalog dataset + planner workflow (optional stretch)

## Recommended order

1. **COPC smoke read** — mirror Phase 3 COG path (`read_cop_path`, `read_cop_uri`, CLI `genegis pointcloud info`)
2. **Plugin API** — define `PluginManifest`, capabilities (`ReadCatalog`, `AnalysisStep`, …), version contract
3. **WASM host** — load `.wasm` plugin, capability gate, smoke invoke from CLI
4. **Workbench hook** — list loaded plugins; no marketplace yet
5. **Second dataset** — e.g. prefecture-level density or remote COG demo entry in catalog

## COPC read (target)

```bash
genegis pointcloud info PATH
genegis pointcloud info https://example.com/lidar.copc.laz
```

```rust
use genegis_pointcloud::{read_copc_path, read_copc_uri, CopcInfo};

let info = read_copc_uri("https://example.com/lidar.copc.laz")?;
assert!(info.point_count > 0);
```

HTTP range-read should reuse `genegis-storage` probes and chunked fetch patterns established for COG (`read_mode: "http_range"`).

## Plugin SDK (Phase 4 alpha)

```rust
use genegis_plugin_api::{
    CapabilityPolicy, PluginCapability, PluginManifest, PLUGIN_API_VERSION,
};

let manifest = PluginManifest {
    id: "demo-filter".into(),
    version: "0.1.0".into(),
    api_version: PLUGIN_API_VERSION.into(),
    capabilities: vec![PluginCapability::AnalysisStep],
    ..Default::default()
};
manifest.validate()?;

let policy = CapabilityPolicy::read_only();
policy.validate_manifest(&manifest)?;
```

Manifest files use `genegis.plugin.json` beside the plugin bundle:

```json
{
  "id": "demo-filter",
  "name": "Demo Filter",
  "version": "0.1.0",
  "api_version": "0.1.0",
  "capabilities": ["analysis_step"],
  "wasm": { "entry": "demo_filter.wasm" }
}
```

```bash
genegis plugin list
genegis plugin info plugins/demo-filter
genegis plugin load plugins/demo-filter
```

Host loads WASM modules with an explicit capability allow-list (RFC D7). TS UI extensions and Python sandboxes remain out of scope for Phase 4 alpha.

## Catalog expansion (stretch)

Keep the north-star Nagoya workflow unchanged. Add a second `DatasetRecord` and `WorkflowId` only when planner tag matching and verification story are ready — avoid hard-coding another one-off path.

## Out of scope

- Plugin marketplace / billing
- Multi-tenant cloud deployment
- Full STAC API server
- Native (non-WASM) plugins
- Autonomous multi-agent GIS (Phase 6)

## North star (unchanged)

「名古屋市の人口密度を表示」 — must keep working offline via rule planner.

## Prerequisites (Phase 3 complete)

- STAC export + catalog registry (`genegis-catalog`)
- COG + HTTP range-read (`genegis-raster`, `genegis-storage`)
- GPU workbench preview + tiled choropleth (`genegis-render`, `apps/workbench`)
- Planner catalog lookup + benchmarks (`genegis-ai`, `genegis-testkit`)

See [`phase-3-beta.md`](phase-3-beta.md).
