# GeneGIS Workbench

Local web workbench — opens in your default browser with the same UI as the Tauri shell.

## Run

```bash
# Terminal 1 — shared collab store (optional but recommended)
cargo run -p genegis-server

# Terminal 2 — workbench UI
cargo run -p genegis-workbench
```

Opens `http://127.0.0.1:7812/` and auto-runs the North Star prompt.

On startup the workbench pulls collab state from GeneGIS Server (`http://127.0.0.1:7813` by default). If the server is down it falls back to `.genegis/collab.json`, then the Nagoya demo session.

## Environment

| Variable | Default | Purpose |
|----------|---------|---------|
| `GENEGIS_SERVER_URL` | `http://127.0.0.1:7813` | Collab pull/push target |
| `GENEGIS_COLLAB_PATH` | `.genegis/collab.json` | Local collab cache |

## API

- `POST /api/ask` — `{ "prompt": "名古屋市の人口密度を表示" }` → JSON includes `result.png_base64`, `result.dataset`, `result.stac_item`, and `result.summary`
- `POST /api/gpu-preview` — launches a native WebGPU view. Use `{ "workflow_id": "scene3d-copc-lod1", "copc_path": "...copc.laz", "buildings_path": "...json", "crs": "EPSG:6675" }` for the Command + Workflow-backed 3D scene; drag to orbit and scroll to zoom.
- `POST /api/ogc/execute` — executes typed WMS `GetCapabilities`/`GetMap` or WFS `GetFeature` JSON through adapter admission and `RunWorkflow`; returns response bytes plus the manifest and shared I/O receipts.
- `POST /api/publish` — `{ "capsule_path": "...", "slug": "nagoya-density" }` verifies a result capsule and writes an allowlisted share/embed bundle under `.genegis/publications/`.
- `POST /api/bridge/capsule` — accepts a provider-neutral desktop layer request and writes a content-addressed Command/Workflow bridge capsule under `.genegis/bridge-capsules/`.
- `GET /api/composer/templates` and `POST /api/composer/sessions` — list reviewed graphs and create an undoable no-code draft; session edit/run routes reject invalid contracts and unreviewed digests before execution.
- `GET /api/plugins` — lists capability-gated plugin manifests from `./plugins` (or repo `plugins/`)
- `GET /api/collab` — map-anchored review comments + branch summary + sync metadata
- `POST /api/collab/comment` — `{ "author": "reviewer", "body": "..." }` → adds comment, saves locally, pushes to server
- `POST /api/collab/sync` — pull latest session from GeneGIS Server
- `GET /api/agent/runs/latest` — latest agent trace from `.genegis/agent-run.json`
- `POST /api/agent/run` — `{ "prompt": "名古屋市の人口密度を表示" }` → plan → execute → verify trace
- Static UI from `../desktop/ui/` — **Download PNG** and **Open GPU Map** buttons; verified temporal slider; **Comments**, **Plugins**, and **Dataset** panels in the sidebar

The 3D building file is a strict measured-facts document. Heights and vertical
coordinates are metres; its CRS must equal the request CRS. POIs are included
in the same source snapshot so dashboard categories are covered by the scene
result digest:

```json
{
  "schema_version": "0.1.0",
  "crs": "EPSG:6675",
  "vertical_unit": "metres",
  "buildings": [
    {
      "id": "building-1",
      "footprint": [[0, 0], [10, 0], [10, 8], [0, 8]],
      "base_z": 0,
      "height": 12.4
    }
  ],
  "pois": [
    {
      "id": "poi-1",
      "position": [4, 4, 0],
      "category": "school"
    }
  ]
}
```

The response includes a verified live dashboard with KPI, building-height
histogram, and POI-category widgets bound to the workflow and scene result
digests.

## Multi-client demo

```bash
# Terminal 1
cargo run -p genegis-server

# Terminal 2 — browser workbench
cargo run -p genegis-workbench

# Terminal 3 — CLI adds a comment and pushes to server
genegis collab comment add "Verify ward boundary" --author cli
genegis collab push

# In the workbench sidebar, click Sync (or reload) to see the CLI comment
```

## Tauri shell

See [`../desktop/README.md`](../desktop/README.md). Tauri release build verified (`npm run build` → `.deb` / AppImage).
