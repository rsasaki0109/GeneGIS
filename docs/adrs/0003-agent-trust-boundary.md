# ADR 0003: Agent Trust Boundary

- **Status:** Accepted (Phase 6 alpha)
- **Date:** 2026-06-13
- **Deciders:** GeneGIS core team

## Context

Phase 6 introduces multi-agent orchestration (`genegis-agent`) over the existing rule/LLM planner, analysis pipeline, DuckDB verification, and WASM plugins. We must define which components may mutate project state, call network APIs, or execute native code without human approval.

## Decision

Use a **layered trust model**:

| Layer | Component | Trust | Human gate |
|-------|-----------|-------|------------|
| **L0 — Deterministic core** | Rule planner, DuckDB verify, GeoWorkflow IR | Highest; offline-capable | Optional `--plan-only` preview |
| **L1 — Network LLM** | `genegis-ai` OpenAI-compatible planner | Medium; API key required | Falls back to L0; plan-only supported |
| **L2 — WASM plugins** | `genegis-plugin-host` capability gates | Medium; manifest allowlist | No auto-load; explicit `plugin load` |
| **L3 — Future tools** | Arbitrary shell / SQL / remote writes | Untrusted | Blocked in Phase 6 alpha |

Agent traces must record **role, tool name, input/output summary, ok flag** for every step. Verification failures stop the run; they are never overridden by LLM output.

## Rationale

- North star workflows must remain **offline** via rule planner + DuckDB (Phase 1–5 guarantee).
- LLM planners are optional accelerators, not authoritative verifiers.
- WASM plugins already use capability policies (`genegis-plugin-api`); agents must not bypass them.
- Full autonomous deployment without gates is explicitly out of scope for Phase 6.

## Consequences

### Phase 6 alpha (now)

- `genegis agent run` uses L0 rule planner by default.
- Agent trace JSON is the audit artifact (`.genegis/agent-run.json`).
- Workbench shows read-only agent steps; no plugin auto-execution from agent graph.

### Phase 6 beta (next)

- LLM tool calls must map to an allowlisted registry before execution.
- Server persists agent runs beside collab Automerge snapshots.
- Collab comments may reference `agent_run_id` + step id.

### Explicit non-goals

- Agents invoking unreviewed WASM plugins automatically.
- LLM-generated SQL against production PostGIS without sandbox.
- Replacing DuckDB verification with model self-grading.

## Alternatives considered

1. **Single monolithic LLM agent** — rejected; no deterministic offline path.
2. **Shell tool for agents** — rejected for Phase 6; too broad.
3. **Verify-only human UI without traces** — rejected; insufficient auditability.

## References

- [`docs/roadmap/phase-6-autonomous.md`](../roadmap/phase-6-autonomous.md)
- [`crates/genegis-agent/`](../../crates/genegis-agent/)
- [`docs/adrs/0002-crdt-backend.md`](0002-crdt-backend.md)
- RFC 0001 — agent-native architecture
