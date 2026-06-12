# GeneGIS AI Native Architecture

AI is not a chat sidebar — it is part of the OS-level design.

## Phase 1: Rule-based Intent Resolver

```
Prompt → ParsedIntent → ResolvedWorkflow → GeoWorkflow IR → Execute
```

Example:

```bash
genegis ask "名古屋市の人口密度を表示"
genegis ask "名古屋市の人口密度を表示" --plan-only
```

Implemented in `crates/genegis-ai`:
- `ParsedIntent` — place / metric / visualization signals
- `resolve_workflow` — binds intent to MVP workflows
- `plan_from_prompt` / `plan_with_config` — returns full GeoWorkflow IR

## Phase 2: LLM planner (alpha)

Optional OpenAI-compatible backend; GeoWorkflow IR and validators unchanged.

```bash
export GENEGIS_LLM_API_KEY=sk-...
genegis ask "Show Nagoya population density" --planner llm --plan-only
```

| Component | Role |
|-----------|------|
| `PlannerBackend` | `rule` (default) or `llm` |
| `PlannerConfig` | API key / base URL / model from env |
| `plan_with_llm` | HTTP JSON planner → `ResolvedWorkflow` |
| Rule fallback | On LLM failure, `--planner llm` falls back to rules |

See `GeneGIS/docs/roadmap/phase-2-alpha.md`.

## Human-in-the-loop

| CLI flag | Mode |
|----------|------|
| `--plan-only` | Strict — plan JSON only, no execution |
| default `ask` | Auto — resolve, execute, export HTML |

## Verification (mandatory)

| Risk | Check |
|------|-------|
| CRS errors | CRS metadata required on area calculations |
| Wrong units | Unit metadata + sanity checks |
| Missing sources | Dataset must exist in catalog or user source |
| Stale statistics | Dataset date required |
| Join mismatch | Granularity / key validation |

## MVP north star

「名古屋市の人口密度を表示」 — supported via `genegis ask`.
