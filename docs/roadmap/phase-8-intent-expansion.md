# Phase 8: Intent Expansion Beyond Nagoya

**Goal:** Route additional natural-language intents through the same **rule planner → catalog → execute → verify** agent graph — not only the Nagoya north star.

**Star target:** 20,000 → 30,000

## Tracks

| Track | Phase 8 focus |
|-------|----------------|
| **AI / Resolver** | Extend `WorkflowId` + intent signals for new MVP workflows |
| **Analysis** | Workflow-dispatch execute + verify (`ExecutedWorkflow`) |
| **Agent** | Per-workflow executor/verifier tools in allowlist |
| **CLI / Workbench** | Second verified prompt smoke + UI hints |
| **Catalog** | STAC-ready dataset binding for new workflows |
| **Docs** | Phase 8 roadmap + orchestration guide update |

## Deliverables

### Phase 8 alpha (multi-workflow agent) — complete

- [x] Phase 8 roadmap (this document)
- [x] `ExecutedWorkflow` dispatch in `genegis-analysis` (`nagoya-density`, `remote-cog-demo`)
- [x] Agent orchestrator routes executor/verifier tools by `WorkflowId`
- [x] Allowlist: `run_remote_cog_metadata`, `cog_metadata_verify`
- [x] Plan-only agent test for remote COG intent (offline resolver smoke)
- [x] CI network smoke — `genegis agent run "リモートCOGデモのメタデータを表示"`
- [x] Workbench UI — show workflow-specific verification labels

### Phase 8 beta (catalog expansion)

- [x] STAC collection browse + bind in planner (`stac_browse`, `stac_bind`)
- [x] Third workflow template — `local-cog-demo` (bundled smoke GeoTIFF)
- [x] GPU workbench preview for non-Nagoya rasters (COG raster grid window)
- [x] CLI `genegis catalog stac list|get`
- [x] Workbench `/api/stac/collection` + workflow-aware `/api/gpu-preview`

## Recommended order

1. **Workflow dispatch** — analysis + agent branch on `WorkflowId`
2. **Remote COG path** — metadata verify (HTTP range-read, no DuckDB)
3. **CI / docs** — second verified prompt in smoke suite
4. **Phase 8 beta** — STAC + additional workflows

## CLI (target)

```bash
# North star (unchanged)
genegis agent run "名古屋市の人口密度を表示"

# Second verified workflow (remote COG metadata)
genegis agent run "リモートCOGデモのメタデータを表示"
genegis workflow run remote-cog-demo --execute
```

## Verification matrix

| Workflow | Execute tool | Verifier | Offline? |
|----------|--------------|----------|----------|
| `nagoya-density` | `run_nagoya_density` | `duckdb_verify` | Yes |
| `remote-cog-demo` | `run_remote_cog_metadata` | `cog_metadata_verify` | Needs HTTP to catalog URI |
| `local-cog-demo` | `run_local_cog_metadata` | `cog_metadata_verify` | Yes (bundled fixture) |

## Out of scope

- Arbitrary city density without curated catalog entries
- LLM-only workflow selection without rule fallback
- Signed provenance / multi-tenant audit search (Phase 7 non-goals remain)

## North star (unchanged)

「名古屋市の人口密度を表示」 — offline rule planner + DuckDB verification must keep passing in CI.

See [`phase-7-release.md`](phase-7-release.md) and [`docs/guides/agent-orchestration.md`](../guides/agent-orchestration.md).
