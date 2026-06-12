# Phase 5: Figma for GIS (Collaboration)

**Goal:** Multi-user project metadata — map-anchored comments, branches, and CRDT-ready document sync toward GeneGIS Server.

**Star target:** 7,500 → 10,000

## Tracks

| Track | Phase 5 focus |
|-------|-----------------|
| **Collab** | Comment threads, project branches, collab document export |
| **Server** | GeneGIS Server sync prototype (HTTP JSON, no marketplace) |
| **Workbench** | Comments panel stub + `/api/collab` |
| **Core** | Wire collab snapshots to `genegis-core::Project` |
| **Docs** | Collaboration model guide; CRDT backend ADR |

## Deliverables

- [x] Phase 5 roadmap (this document)
- [x] Collab document model (`genegis-collab`: comments + branches + JSON export)
- [x] CLI collab smoke (`genegis collab comment|branch|export`)
- [x] Workbench comments panel stub (`GET /api/collab`)
- [x] GeneGIS Server sync prototype (`genegis-server` GET/PUT `/api/collab` on `:7813`)
- [x] Workbench ↔ Server collab sync (startup pull, comment push, `POST /api/collab/sync`)
- [x] CLI collab pull/push (`genegis collab pull|push`)
- [x] Automerge CRDT merge for comments/branches (`genegis-collab` + `.genegis/collab.json.automerge`)
- [x] CRDT backend ADR ([`docs/adrs/0002-crdt-backend.md`](../adrs/0002-crdt-backend.md) — Automerge for metadata)

## Recommended order

1. **Collab document** — `CollabDocument` wrapping `Project` + comments + branches
2. **CLI smoke** — add/list comments, create/list branches, export JSON
3. **Workbench panel** — read-only comment list from `/api/collab`
4. **Server prototype** — POST/GET session JSON on localhost
5. **CRDT ADR** — pick sync engine for project metadata (not geometry tiles)

## Collab document (target)

```rust
use genegis_collab::{CollabDocument, CollabSession, MapComment};

let mut session = CollabSession::demo_nagoya();
session.add_comment(MapComment::new("reviewer", "Check ward 中区 density"));
let json = session.export_json()?;
```

```bash
genegis collab comment list
genegis collab comment add "Verify 中区 boundary" --author reviewer
genegis collab branch create experiment-style --from main
genegis collab export -o .genegis/collab.json
```

## Workbench (target)

```bash
cargo run -p genegis-server    # terminal 1
cargo run -p genegis-workbench # terminal 2
# Sidebar → Comments panel lists map-anchored threads; Add comment + Sync
curl http://127.0.0.1:7812/api/collab
curl -X POST http://127.0.0.1:7812/api/collab/comment \
  -H 'Content-Type: application/json' \
  -d '{"author":"reviewer","body":"Check ward density"}'
curl -X POST http://127.0.0.1:7812/api/collab/sync
```

## GeneGIS Server (target)

```bash
cargo run -p genegis-server
curl http://127.0.0.1:7813/health
curl http://127.0.0.1:7813/api/collab
curl -X PUT http://127.0.0.1:7813/api/collab \
  -H 'Content-Type: application/json' \
  -d "$(jq -n --arg session "$(cat .genegis/collab.json)" '{session:$session}')"
```

Persists to `.genegis/collab.json` and `.genegis/collab.json.automerge` by default (`GENEGIS_COLLAB_PATH`, `GENEGIS_SERVER_PORT=7813`).

PUT merges incoming JSON + optional `automerge_snapshot` (base64) via Automerge — concurrent comments from multiple clients are preserved.

## Out of scope

- Real-time cursor presence / Figma-style live cursors
- Full geometry CRDT (only project metadata + comments in Phase 5 alpha)
- Billing, org SSO, enterprise ACLs
- Autonomous multi-agent GIS (Phase 6 — see [`phase-6-autonomous.md`](phase-6-autonomous.md))

## North star (unchanged)

「名古屋市の人口密度を表示」 — must keep working offline via rule planner.

## Prerequisites (Phase 4 complete)

- Plugin SDK + WASM host (`genegis-plugin-api`, `genegis-plugin-host`)
- COPC alpha read (`genegis-pointcloud`)
- Workbench plugin panel (`apps/workbench`)
- Second catalog workflow (`remote-cog-demo`)

See [`phase-4-plugins.md`](phase-4-plugins.md).
