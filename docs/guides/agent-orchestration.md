# Agent orchestration guide

GeneGIS Phase 6 runs **plan → catalog → execute → verify** as an auditable agent graph. The north star prompt must keep working offline via the rule planner and DuckDB verification.

## Trust layers (ADR 0003)

| Layer | Component | Default |
|-------|-----------|---------|
| L0 | Rule planner + DuckDB verify | Offline, highest trust |
| L1 | LLM planner (`GENEGIS_LLM_*`) | Optional; falls back to L0 |
| L2 | WASM plugins | Explicit load only |
| L3 | Shell / arbitrary SQL | Blocked |

All agent steps record **role, tool, input/output, ok** in `.genegis/agent-run.json`.

## CLI quick start

```bash
# Full run (offline rule planner)
genegis agent run "名古屋市の人口密度を表示"

# Human gate — plan first, approve later
genegis agent plan "名古屋市の人口密度を表示"
genegis agent execute

# Plan-only preview (same graph, stops after planner)
genegis agent run "名古屋市の人口密度を表示" --plan-only

# Sync trace to GeneGIS Server
genegis agent run "名古屋市の人口密度を表示" --push
genegis agent pull
genegis agent list
genegis agent get RUN_ID
```

Pending plans are stored at `.genegis/agent-plan.json`. Runs are stored at `.genegis/agent-run.json` and `.genegis/agent-runs/{id}.json`.

## Workbench human gate

1. Start `genegis-server` (port 7813) and `genegis-workbench` (port 7812).
2. Sidebar → **Agent trace** → **Plan only** saves a pending plan.
3. **Approve & execute** runs catalog → analysis → DuckDB verify without re-planning.
4. Collab comments and project provenance record `agent_run_id` on failure or success.
5. **External STAC** panel — fetch collection JSON or import STAC items into `.genegis/catalog-overlay.json`.

## Tool allowlist

Planner tools: `parse_intent`, `resolve_workflow`, `stac_browse`, `stac_bind`, `stac_fetch`, `llm_plan_workflow`, `plan_workflow`.

Executor tools: `catalog_resolve`, `run_nagoya_density`, `run_remote_cog_metadata`, `run_local_cog_metadata`, `run_geoparquet_read`, `run_geoparquet_density`, `run_stac_fetch`, `verify_retry`.

Verifier tools: `duckdb_verify`, `cog_metadata_verify`, `geoparquet_feature_verify`, `stac_collection_verify`.

Unknown tools are rejected before execution (see `crates/genegis-agent/src/tool_registry.rs`).

## Verification matrix (Phase 9)

| Workflow | Execute tool | Verifier | Offline? |
|----------|--------------|----------|----------|
| `nagoya-density` | `run_nagoya_density` | `duckdb_verify` | Yes |
| `remote-cog-demo` | `run_remote_cog_metadata` | `cog_metadata_verify` | Needs HTTP |
| `local-cog-demo` | `run_local_cog_metadata` | `cog_metadata_verify` | Yes |
| `nagoya-geoparquet` | `run_geoparquet_read` | `geoparquet_feature_verify` | Yes (fixture) |
| `nagoya-geoparquet-density` | `run_geoparquet_density` | `duckdb_verify` | Yes (fixture) |
| `external-stac-demo` | `run_stac_fetch` | `stac_collection_verify` | Yes (sample JSON) |

## Server API

```bash
curl http://127.0.0.1:7813/api/agent/runs/latest
curl http://127.0.0.1:7813/api/agent/runs
curl http://127.0.0.1:7813/api/agent/runs/5bcfb044-7170-4aa1-b652-8f774d8cb28f
curl -X POST http://127.0.0.1:7813/api/agent/runs -H 'Content-Type: application/json' -d @.genegis/agent-run.json
```

Workbench proxies the same flow at `/api/agent/plan`, `/api/agent/execute`, `/api/agent/retry`, and `/api/agent/runs/latest`. STAC overlay APIs: `/api/stac/fetch`, `/api/stac/import`, `/api/stac/overlay`.

Tauri desktop uses the same UI with `invoke` commands (`agent_plan`, `agent_execute`, `agent_retry`, `agent_runs_list`, …).

## Audit export

```bash
genegis agent export-audit -o .genegis/audit-bundle.json
genegis collab provenance list
```

Bundle includes collab summary, comments, provenance entries, agent run index, and STAC snapshots (`alpha`, `overlay`, `merged`). Schema: `genegis-audit-bundle-v3` (see `crates/genegis-agent/src/audit.rs`).

## Provenance

Successful or pending agent runs append to `Workspace.provenance` inside the collab project snapshot:

- `agent_run_id` — UUID of the orchestration run
- `workflow_id` — resolved MVP workflow (`nagoya-density`, …)
- `action` — `agent_plan_pending`, `agent_run_verified`, or `agent_run_failed`

## References

- [`docs/roadmap/phase-6-autonomous.md`](../roadmap/phase-6-autonomous.md)
- [`docs/roadmap/phase-9-external-data.md`](../roadmap/phase-9-external-data.md)
- [`docs/adrs/0003-agent-trust-boundary.md`](../adrs/0003-agent-trust-boundary.md)
- [`crates/genegis-agent/`](../../crates/genegis-agent/)
