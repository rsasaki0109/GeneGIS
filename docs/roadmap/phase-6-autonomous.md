# Phase 6: Autonomous GIS Platform

**Goal:** Multi-agent orchestration over verified GeoWorkflow IR — planners, data agents, and verifiers collaborate with humans through GeneGIS Server and the workbench.

**Star target:** 10,000 → 15,000

## Tracks

| Track | Phase 6 focus |
|-------|-----------------|
| **Agents** | Role-based agents (plan / discover / execute / verify) with tool contracts |
| **Orchestration** | Agent run graph, retries, human-in-the-loop gates on verification failure |
| **Workbench** | Agent trace panel — workflow steps, tool calls, collab comments linked to runs |
| **Server** | Persist agent runs + attach provenance to collab sessions |
| **Core** | Provenance on `Project`; workflow run IDs wired to `genegis-collab` |
| **Docs** | Agent orchestration guide; trust / sandbox ADR for WASM + LLM tools |

## Deliverables

### Phase 6 alpha (orchestration skeleton)

- [x] Phase 6 roadmap (this document)
- [x] `genegis-agent` crate — agent roles, run context, tool registry stub
- [x] Orchestrator smoke — plan → execute → verify loop for `nagoya-density` only
- [x] CLI `genegis agent run "…"` — prints agent trace JSON (no new LLM deps required for smoke)
- [x] Workbench **Agent trace** sidebar stub (`GET /api/agent/runs/latest`)
- [x] ADR: agent trust boundary ([`docs/adrs/0003-agent-trust-boundary.md`](../adrs/0003-agent-trust-boundary.md))

### Phase 6 beta (multi-agent + server)

- [x] Multi-step agent graph (planner → catalog → analysis → verify)
- [x] Verification retry policy (re-execute + re-verify on DuckDB failure)
- [x] GeneGIS Server `POST/GET /api/agent/runs` persisted under `.genegis/`
- [x] Collab comments linked to agent run id + workflow step id
- [x] LLM planner emits structured tool calls (fallback to rule planner offline)

## Recommended order

1. **Agent run model** — `AgentRun`, `AgentStep`, `ToolCall` JSON schema
2. **Orchestrator alpha** — wrap existing `plan_from_prompt` + `run_ask_pipeline` as two agent steps
3. **CLI smoke** — `genegis agent run` with trace export
4. **Workbench panel** — read-only latest run from workbench API
5. **Server persistence** — store runs beside collab Automerge snapshot
6. **Multi-agent beta** — catalog discovery agent + verify-retry loop
7. **Trust ADR** — document sandbox for plugins and LLM tool allowlists

## Agent orchestration (target)

```rust
use genegis_agent::{AgentOrchestrator, AgentRunConfig};

let run = AgentOrchestrator::new()
    .with_config(AgentRunConfig::rule_based_offline())
    .run("名古屋市の人口密度を表示")?;

assert!(run.verification_passed);
println!("{}", run.trace_json()?);
```

```bash
# Plan + execute + verify with agent trace (offline rule planner)
genegis agent run "名古屋市の人口密度を表示"

# Plan-only gate (human approves before execute)
genegis agent run "名古屋市の人口密度を表示" --plan-only

# Export trace for workbench / server
genegis agent run "名古屋市の人口密度を表示" --json -o .genegis/agent-run.json
```

## Workbench (target)

```bash
cargo run -p genegis-server    # terminal 1 — collab + agent runs
cargo run -p genegis-workbench # terminal 2
# Sidebar → Agent trace lists planner / execute / verify steps
curl http://127.0.0.1:7812/api/agent/runs/latest
```

## GeneGIS Server (target)

```bash
cargo run -p genegis-server
curl http://127.0.0.1:7813/api/agent/runs/latest
curl -X POST http://127.0.0.1:7813/api/agent/runs \
  -H 'Content-Type: application/json' \
  -d @.genegis/agent-run.json
```

Runs stored under `.genegis/agent-runs/` by default (`GENEGIS_AGENT_RUNS_DIR`).

## Out of scope

- Fully autonomous unsupervised production deployments (no human gates)
- Arbitrary code execution from LLM without sandbox / allowlist
- Replacing DuckDB verification with LLM self-grading
- Marketplace billing, org SSO, fleet management
- Geometry CRDT or real-time multi-user cursors (Phase 5 non-goals remain)

## North star (unchanged)

「名古屋市の人口密度を表示」 — must keep working **offline** via rule planner with DuckDB verification, even when LLM agents are enabled.

## Prerequisites (Phase 5 complete)

- Automerge collab merge (`genegis-collab`, `.genegis/collab.json.automerge`)
- Workbench ↔ Server collab sync (`genegis-server`, workbench comments panel)
- Plugin SDK + WASM host (`genegis-plugin-api`, `genegis-plugin-host`)
- LLM planner optional path (`genegis-ai`, `GENEGIS_LLM_*` env)

See [`phase-5-collab.md`](phase-5-collab.md).
