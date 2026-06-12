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
- [ ] GeneGIS Server sync prototype (`genegis-server` — optional stretch)
- [ ] CRDT backend ADR (`docs/adrs/` — Automerge vs Yjs)

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
cargo run -p genegis-workbench
# Sidebar → Comments panel lists map-anchored threads
curl http://127.0.0.1:7812/api/collab
```

## Out of scope

- Real-time cursor presence / Figma-style live cursors
- Full geometry CRDT (only project metadata + comments in Phase 5 alpha)
- Billing, org SSO, enterprise ACLs
- Autonomous multi-agent GIS (Phase 6)

## North star (unchanged)

「名古屋市の人口密度を表示」 — must keep working offline via rule planner.

## Prerequisites (Phase 4 complete)

- Plugin SDK + WASM host (`genegis-plugin-api`, `genegis-plugin-host`)
- COPC alpha read (`genegis-pointcloud`)
- Workbench plugin panel (`apps/workbench`)
- Second catalog workflow (`remote-cog-demo`)

See [`phase-4-plugins.md`](phase-4-plugins.md).
