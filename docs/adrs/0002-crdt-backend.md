# ADR 0002: CRDT Backend for Project Metadata Sync

- **Status:** Accepted (Phase 5 alpha)
- **Date:** 2026-06-13
- **Deciders:** GeneGIS core team

## Context

Phase 5 introduces map-anchored comments, project branches, and JSON session export via `genegis-collab`. Phase 5 stretch requires multi-client sync through GeneGIS Server without losing offline-first behavior.

RFC 0001 lists **Automerge vs Yjs** as an open question. We must pick a direction before wiring real-time collaboration.

## Decision

Use **Automerge 3.x (Rust crate `automerge`)** as the CRDT backend for **project metadata only** (comments, branch pointers, workflow review state, catalog selections).

Do **not** CRDT-sync geometry tiles, raster windows, or full DuckDB tables in Phase 5.

## Rationale

| Option | Pros | Cons |
|--------|------|------|
| **Automerge (chosen)** | Native Rust library; fits `genegis-core` stack; document model maps cleanly to JSON export; offline merge without central server | Heavier dependency; TS workbench needs WASM/JS bridge later |
| **Yjs** | Mature TS ecosystem; good for browser-first apps | Rust host/server path is immature; splits brain with Tauri + Rust core |
| **Custom LWW JSON** | Minimal deps for alpha HTTP PUT | Poor conflict semantics; hard to evolve to multi-user |

GeneGIS core is Rust-first (D1). Server and CLI already export `CollabDocument` JSON. Automerge documents can wrap the same envelope while preserving merge semantics for comments and branch metadata.

## Consequences

### Phase 5 alpha (now)

- HTTP **GET/PUT** `/api/collab` on `genegis-server` with whole-document replace (no CRDT merge yet).
- `CollabDocument.schema_version` remains the compatibility gate.

### Phase 5 beta (next)

- Introduce `CollabDocument.automerge_snapshot` optional field.
- Server stores Automerge binary + projects JSON view for clients without CRDT support.
- Workbench reads merged view via `/api/collab`.

### Explicit non-goals

- Geometry CRDT (use versioned layers + immutable blobs instead).
- Marketplace / org ACL sync.
- Replacing DuckDB analytics state with CRDT.

## Alternatives considered

1. **Yjs-only** — rejected for Rust server/host alignment.
2. **Git-like branch merge for comments** — rejected; poor UX for live co-editing.
3. **Postgres row locks only** — rejected; breaks offline-first north star workflows.

## References

- [`docs/roadmap/phase-5-collab.md`](../roadmap/phase-5-collab.md)
- [`crates/genegis-collab/`](../../crates/genegis-collab/)
- [Automerge Rust docs](https://docs.rs/automerge/latest/automerge/)
- RFC 0001 open question: CRDT choice
