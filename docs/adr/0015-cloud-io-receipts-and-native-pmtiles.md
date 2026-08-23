# ADR 0015: Shared cloud I/O receipts and a narrow native PMTiles reader

- Status: Accepted
- Date: 2026-08-23

## Context

COG, GeoParquet, COPC, and PMTiles already expose or require range-oriented
access, but library-specific counters do not establish the same claims. A
successful decode can also conceal repeated overlapping reads or a whole-file
fallback. Phase 12 requires request, transfer, memory, latency, and GPU evidence
under one budget.

The tile crate was only a placeholder. Pulling a complete tile server or a
generic remote object stack into the core would enlarge the trusted boundary
and make its fallback behavior harder to inspect.

## Decision

`genegis-storage` defines one format-neutral `IoReceipt`. Exact request ranges,
object and logical sizes, selected predicate, transferred bytes, largest
response, fallback, decoded item count, wall time, peak RSS, and optional wgpu
metrics are independently budgeted. Whole-object fallback is a distinct hard
failure even when timing is fast.

`genegis-tile` implements a small PMTiles v3 selector for header, root/leaf
directory, and one z/x/y tile. It:

- requests only explicit ranges and rejects HTTP 200 fallback;
- validates the 127-byte v3 header, section bounds, 16 KiB root constraint,
  varints, Hilbert IDs, and directory decompression budget;
- supports transparent no-compression and gzip directory decoding;
- returns compressed tile bytes without guessing their semantic content;
- fails explicitly for unsupported internal compression.

GeoParquet HTTP access uses a shared 64 KiB aligned block cache. Physical I/O
counters advance only when a block is actually fetched, so overlapping
Parquet reads cannot be counted as optimized merely because each request used
HTTP 206.

## Consequences

- Every format can be compared under the same evidence vocabulary.
- CI detected and removed a 2.63× GeoParquet transfer-amplification defect.
- PMTiles support is useful for exact tile selection without becoming a tile
  server or renderer.
- Brotli and zstd PMTiles directories remain explicitly unsupported until
  reviewed decoders and decompression budgets are added.
- The deterministic CI lane does not satisfy the 256 MiB / 1 GiB full-fixture
  gate; full-lane evidence remains a separate deliverable.
