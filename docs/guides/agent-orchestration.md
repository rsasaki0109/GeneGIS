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

## Tool allowlist

Planner tools: `parse_intent`, `resolve_workflow`, `catalog_bind`, `llm_plan_workflow`, `plan_workflow`.

Executor tools: `catalog_resolve`, `run_nagoya_density`, `verify_retry`.

Verifier tools: `duckdb_verify`.

Unknown tools are rejected before execution (see `crates/genegis-agent/src/tool_registry.rs`).

## Server API

```bash
curl http://127.0.0.1:7813/api/agent/runs/latest
curl http://127.0.0.1:7813/api/agent/runs
curl http://127.0.0.1:7813/api/agent/runs/5bcfb044-7170-4aa1-b652-8f774d8cb28f
curl -X POST http://127.0.0.1:7813/api/agent/runs -H 'Content-Type: application/json' -d @.genegis/agent-run.json
```

Workbench proxies the same flow at `/api/agent/plan`, `/api/agent/execute`, `/api/agent/retry`, and `/api/agent/runs/latest`.

Tauri desktop uses the same UI with `invoke` commands (`agent_plan`, `agent_execute`, `agent_retry`, `agent_runs_list`, …).

## Audit export

```bash
genegis agent export-audit -o .genegis/audit-bundle.json
genegis collab provenance list
```

Bundle includes collab summary, comments, provenance entries, and agent run index. Schema: `genegis-audit-bundle-v1` (see `crates/genegis-agent/src/audit.rs`).

## Provenance

Successful or pending agent runs append to `Workspace.provenance` inside the collab project snapshot:

- `agent_run_id` — UUID of the orchestration run
- `workflow_id` — resolved MVP workflow (`nagoya-density`, …)
- `action` — `agent_plan_pending`, `agent_run_verified`, or `agent_run_failed`

## References

- [`docs/roadmap/phase-6-autonomous.md`](../roadmap/phase-6-autonomous.md)
- [`docs/adrs/0003-agent-trust-boundary.md`](../adrs/0003-agent-trust-boundary.md)
- [`crates/genegis-agent/`](../../crates/genegis-agent/)
