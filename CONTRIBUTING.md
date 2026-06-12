# Contributing to GeneGIS

Thank you for helping build the next generation of GIS.

## Before you start

1. Read the [README](../README.md) and [architecture overview](architecture/overview.md)
2. GeneGIS is **not** a QGIS clone — please don't open PRs that recreate layer-tree desktop UX
3. Major changes need an RFC in `docs/rfcs/`

## Development setup

```bash
cargo build
cargo test
cargo run -p genegis-cli -- workflow run nagoya-density
```

## Code organization

- `crates/genegis-core` — project model, commands (stable-ish)
- `crates/genegis-workflow` — GeoWorkflow IR
- `crates/genegis-render` — wgpu rendering
- Other crates — engines (Phase 0+ skeletons)

## Pull request guidelines

- Keep PRs focused; prefer small incremental changes
- Include tests when adding logic to core crates
- Record CRS, units, and provenance for any spatial operation
- No secrets, credentials, or private dataset dumps

## RFC process

1. Copy `docs/rfcs/0001-master-architecture.md` as a template
2. Number sequentially (`0002-...`)
3. Discuss in issues before large implementations

## Code of conduct

Be respectful. We compete with ideas, not people. QGIS and ArcGIS deserve credit as inspirations we learn from — not targets to attack.
