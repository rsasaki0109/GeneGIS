# GeneGIS Workbench

Local web workbench — opens in your default browser with the same UI as the Tauri shell.

## Run

```bash
cargo run -p genegis-workbench
```

Opens `http://127.0.0.1:7812/` and auto-runs the North Star prompt.

## API

- `POST /api/ask` — `{ "prompt": "名古屋市の人口密度を表示" }` → JSON includes `result.png_base64`, `result.dataset`, `result.stac_item`, and `result.summary`
- `POST /api/gpu-preview` — launches native WebGPU choropleth window (after pipeline success)
- `GET /api/plugins` — lists capability-gated plugin manifests from `./plugins` (or repo `plugins/`)
- `GET /api/collab` — map-anchored review comments + branch summary
- Static UI from `../desktop/ui/` — **Download PNG** and **Open GPU Map** buttons; **Comments**, **Plugins**, and **Dataset** panels in the sidebar

## Tauri shell

See [`../desktop/README.md`](../desktop/README.md). Tauri release build verified (`npm run build` → `.deb` / AppImage).
