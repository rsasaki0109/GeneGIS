# GeneGIS Desktop Workbench (Tauri alpha)

Phase 1 desktop shell: natural language prompt → choropleth map + verification panel.

> **Rust 1.94:** This crate depends on vendored patches under `GeneGIS/third-party/` (`cookie`, `tauri`, `tauri-utils`). See `third-party/README.md`.
>
> **Alternative:** `cargo run -p genegis-workbench` serves the same UI in your browser without Tauri.

## Prerequisites (Linux)

```bash
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
  librsvg2-dev patchelf
```

## Run

```bash
cd GeneGIS/apps/desktop
npm install
npm run dev
```

On launch, the North Star prompt runs automatically and renders the map in the embedded viewer.

**Dev launch verified:** `npm run dev` compiles and starts `genegis-desktop` on Linux (Rust 1.94 + GTK/WebKit).

## Architecture

Shared UI lives in `apps/desktop/ui/` (`index.html`, `app.js`, `styles.css`). The same `app.js` calls Tauri `invoke('run_ask')` in the desktop shell and `POST /api/ask` when served by `genegis-workbench`.

```
UI (HTML/JS) --invoke--> Tauri `run_ask` --> genegis-analysis::run_ask_pipeline
                                              ├── genegis-ai (intent)
                                              ├── DuckDB verify
                                              └── HTML choropleth export
```

## Build release

```bash
npm run build
```

Binary output: `target/release/genegis-desktop` (workspace root)  
Bundled packages: `target/release/bundle/` (`.deb`, AppImage on Linux)
