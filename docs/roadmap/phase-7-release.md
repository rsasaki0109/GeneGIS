# Phase 7: Audit Trail & Release Workbench

**Goal:** Make agent orchestration auditable in the UI — run history, provenance timeline, and server-backed trace retrieval for reviewers.

**Star target:** 15,000 → 20,000

## Tracks

| Track | Phase 7 focus |
|-------|-----------------|
| **Server** | Agent run list + get-by-id APIs |
| **Workbench** | Run history panel + provenance sidebar |
| **CLI** | `genegis agent list|get` for audit scripts |
| **Collab** | Expose workspace provenance in `/api/collab` |
| **Desktop** | Tauri invoke parity for agent + provenance |
| **Docs** | Phase 7 roadmap + update orchestration guide |

## Deliverables

### Phase 7 alpha (audit trail)

- [x] Phase 7 roadmap (this document)
- [x] `GET /api/agent/runs` — list run summaries (newest first)
- [x] `GET /api/agent/runs/:id` — fetch full trace by UUID
- [x] CLI `genegis agent list|get RUN_ID`
- [x] Workbench agent history buttons + provenance panel
- [x] Collab API includes `provenance` entries with `agent_run_id`

### Phase 7 beta (release workbench)

- [x] Tauri desktop — agent plan/execute/retry/list/get + collab provenance
- [x] Workbench verification retry — `POST /api/agent/retry` + **Retry verify** button
- [x] Provenance filter — history click filters provenance by `agent_run_id`
- [x] CLI `genegis agent export-audit` — collab + provenance + run index bundle
- [x] CLI `genegis collab provenance list`

### Phase 7 gamma (release hardening)

- [x] CI — `genegis agent run` north-star smoke + human gate + export-audit
- [x] Server E2E — in-process axum tests for agent run list/get/latest POST
- [x] Audit bundle regression — `build_audit_bundle` schema + run index tests
- [x] Workbench hero HTML refresh (Phase 7 agent history + provenance panels)
- [x] Workbench hero GIF re-render (`scripts/render-readme-hero.sh`)

## Recommended order

1. **AgentRunSummary** — lightweight index for history APIs
2. **Server list/get** — extend `AgentRunStore`
3. **CLI audit** — list/get with server fallback to `.genegis/agent-runs/`
4. **Workbench UI** — history + provenance panels
5. **Phase 7 beta** — Tauri desktop parity, provenance filters, export bundle
6. **Phase 7 gamma** — CI agent smoke, server E2E tests, audit bundle regression

## CLI (target)

```bash
genegis agent list
genegis agent get 5bcfb044-7170-4aa1-b652-8f774d8cb28f
genegis agent export-audit -o .genegis/audit-bundle.json
genegis collab provenance list
curl http://127.0.0.1:7813/api/agent/runs
curl http://127.0.0.1:7813/api/agent/runs/5bcfb044-7170-4aa1-b652-8f774d8cb28f
```

## Workbench (target)

Sidebar → **Agent trace** shows latest run steps; **history** lists recent runs (click to load). **Provenance** lists workspace audit entries linked to agent runs. Failed verification shows **Retry verify**.

## Out of scope

- Signed provenance / WORM storage
- Org-wide audit search across tenants
- Geometry tile provenance (metadata only)

## North star (unchanged)

「名古屋市の人口密度を表示」 — offline rule planner + DuckDB verification.

## Next

Phase 8 — intent expansion beyond Nagoya: [`phase-8-intent-expansion.md`](phase-8-intent-expansion.md)
