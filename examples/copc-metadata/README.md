# COPC metadata example

Smoke-read COPC header metadata using `genegis-pointcloud`.

## Run (local fixture)

```bash
cargo run -p copc-metadata
```

Uses the bundled PDAL `lone-star.copc.laz` fixture under `crates/genegis-pointcloud/testdata/`.

## Run (HTTP range-read)

```bash
cargo run -p copc-metadata -- "https://example.com/points.copc.laz"
```

Remote URIs use HTTP range-read (`read_mode: "http_range"`).

## CLI equivalent

```bash
genegis pointcloud info crates/genegis-pointcloud/testdata/lone-star.copc.laz
```

See [`docs/roadmap/phase-4-plugins.md`](../../docs/roadmap/phase-4-plugins.md).
