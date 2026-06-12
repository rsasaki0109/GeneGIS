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
- `POST /api/gpu-preview` — launches native WebGPU choropleth window (after pipeline success)
- `GET /api/plugins` — lists capability-gated plugin manifests from `./plugins` (or repo `plugins/`)
- `GET /api/collab` — map-anchored review comments + branch summary + sync metadata
- `POST /api/collab/comment` — `{ "author": "reviewer", "body": "..." }` → adds comment, saves locally, pushes to server
- `POST /api/collab/sync` — pull latest session from GeneGIS Server
- `GET /api/agent/runs/latest` — latest agent trace from `.genegis/agent-run.json`
- `POST /api/agent/run` — `{ "prompt": "名古屋市の人口密度を表示" }` → plan → execute → verify trace
- Static UI from `../desktop/ui/` — **Download PNG** and **Open GPU Map** buttons; **Comments**, **Plugins**, and **Dataset** panels in the sidebar

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
