# Phase 12–13: Operational Verification Workbench

**Status:** In progress.

**Goal:** Turn Phase 11 proof-carrying analysis into an operational workbench
that safely edits, executes across real GIS engines, processes large
cloud-native objects, verifies diverse data, measures human review, and states
source truth boundaries honestly.

The authoritative requirements and exit gates are in
`/home/sasaki/workspace/GeneGIS/docs/rfcs/0004-operational-verification-workbench.md`.

## Workstreams

| ID | Deliverable | Exit evidence | Status |
| --- | --- | --- | --- |
| `P0-1` | Source Assurance evidence model, digest, policy, and trust integration | Positive and fail-closed tests for freshness, checks, disputes, limitations, identity, and independent corroboration | Complete 2026-08-23 |
| `P0-2` | Adapter Manifest v0 and capability admission | Committed schema; exact capability/backend/operation admission; write, drift, opaque, and under-declaration negative tests | Complete 2026-08-23 |
| `P0-3` | Phase acceptance harness | Machine-readable report schema with every RFC 0004 row represented as pass/fail/not-measured | Complete 2026-08-23 |
| `P1-1` | PostGIS read-only adapter | Pinned real PostGIS container; typed queries; transaction/query-plan evidence; at least 5 positive and 5 negative cases | Complete 2026-08-23; 5+5, false accepts 0 |
| `P1-2` | GRASS GIS sandbox adapter | Pinned real GRASS container; disposable mapset/region; typed modules; at least 5 positive and 5 negative cases | Complete 2026-08-23; 6+5, false accepts 0 |
| `P1-3` | QGIS Processing sandbox adapter | Pinned real QGIS container; disposable profile; provider/algorithm evidence; at least 5 positive and 5 negative cases | Complete 2026-08-23; 5+5, false accepts 0 |
| `P1-4` | Cross-adapter conformance | Geometry/reprojection/area/join/null/multipart/ordering corpus and zero false admissions | Complete 2026-08-23; 33 oracle cases + 6 real-adapter observations, false admissions 0 |
| `P2-1` | Command-backed feature editing | Create/update/delete/split/merge/repair through Command+DAG with replay and provenance | Complete 2026-08-23 |
| `P2-2` | Evidence-first cartography | Classification, legend, labels, deterministic layout and artifact identity | Complete 2026-08-23; deterministic evidence-bound SVG lane |
| `P2-3` | Edit negative/mutation suite | At least 30 negative edits, 100-command replay, false accepted edits = 0 | Complete 2026-08-23; 30/30 caught, replay digests match |
| `P3-1` | Common cloud I/O receipt | Range, byte, request, fallback, time, memory, and GPU evidence model | Complete 2026-08-23 |
| `P3-2` | Large COG/GeoParquet/COPC/PMTiles fixtures | At least 256 MiB objects / 1 GiB logical datasets plus deterministic CI tier | Complete 2026-08-23; 4 pinned public range objects, identity drift fails closed |
| `P3-3` | Performance and budget report | p50/p95, RSS, transfer ratio, first frame, upload, FPS; budget gates | Complete 2026-08-23; 4/4 pass, hardware Vulkan GPU, zero whole-object fallbacks |
| `P4-1` | Five-domain source corpus | Boundaries, population, raster, point cloud, temporal change with licenses and immutable snapshots | Complete 2026-08-23; three positive sources plus two real fail-closed sources with explicit limitations |
| `P4-2` | Oracles and mutation harness | At least 100 mutations, score at least 95%, false verified/attested = 0 | Complete 2026-08-23; 112/112 caught, score 100%, false verified/attested 0 |
| `P5-1` | Map-first Trust UX | Map-to-evidence navigation for all preregistered failure categories | Complete 2026-08-23; fixed 12-task map-first TUI, interaction timing, abort preservation, digest binding |
| `P5-2` | Human UX study | At least 3 human reviewers, 12 tasks, correctness/time/interaction report | Runner/protocol preregistered; real human execution pending per user instruction to skip human work |
| `P6-1` | Capsule/standards integration | Source Assurance, adapter, edit, I/O, UX evidence bound into offline-verifiable capsule | Pending |
| `P6-2` | Final acceptance and release audit | RFC row-by-row evidence, full tests, no LLM/server verification, main-ready worktree | Pending |

## First implementation slice

1. Add Source Assurance types and fail-closed policy derivation without changing
   Phase 11 policy behavior unless a new assurance policy is selected.
2. Add `genegis-adapter` with a committed schema and pre-execution admission.
3. Add real manifests for PostGIS, GRASS, and QGIS Processing.
4. Establish pinned container fixtures and execute the smallest read-only
   positive and dangerous-operation negative case for each backend.
5. Expand only after the real boundary is proven; mocks do not satisfy `P1`.

## Environment baseline

Observed on 2026-08-23 in `/home/sasaki/workspace/GeneGIS`:

- Docker client/server: 29.6.2 on Ubuntu 24.04.4 LTS;
- host `psql`, PostgreSQL server tools, GRASS, `qgis_process`, GDAL/OGR: absent;
- therefore conformance runs use pinned container image digests and record both
  container and in-container engine identities.

## Non-goals

- QGIS desktop UI or Processing Toolbox parity;
- unrestricted SQL, shell, Python, or native plugin execution;
- declaring official or corroborated data infallible;
- hiding unsupported engine semantics behind a generic success state;
- publishing performance numbers without exact fixture and runner identity.
