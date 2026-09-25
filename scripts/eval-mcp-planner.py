#!/usr/bin/env python3
"""Evaluate an MCP-capable agent (Claude Code) as the GeneGIS planner.

For every case in `planner_eval cases`, this script
  1. creates a fresh layer store preloaded with the Nagoya samples,
  2. runs `claude -p` with only the genegis MCP server available,
  3. scores the last verified analysis the agent stored with
     `planner_eval score-store` against the hand-written ground truth.

The agent plans; GeneGIS validates, executes, and verifies. Nothing the
agent says is scored — only the verified output layer it produced.

    python3 scripts/eval-mcp-planner.py [--cases 0,4,12] [--model sonnet] \
        [--report docs/reports/rfc-0007-planner-eval-claude-code.json]

Requires: `cargo build -p genegis-toolkit --bin genegis-mcp --examples` and
the `claude` CLI on PATH. Each case is one headless Claude Code session and
is billed to the account the CLI is logged in with.
"""

import argparse
import datetime
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EXE = ".exe" if os.name == "nt" else ""
MCP = ROOT / "target" / "debug" / f"genegis-mcp{EXE}"
EVAL = ROOT / "target" / "debug" / "examples" / f"planner_eval{EXE}"

PROMPT = """あなたは GeneGIS の MCP ツールだけを使って空間分析の質問に答えます。
名古屋のサンプルレイヤ（区界と人口・避難所・店舗・浸水想定区域・主要駅）はすでに読み込まれています。
list_layers と list_operations で確認し、計画を組んで run_plan で実行してください。
run_plan がエラーを返したら理由を読んで計画を直してください。
最後に実行する run_plan の出力レイヤが、質問への答えそのもの（集計値なら1行）になるようにしてください。
データで答えられない質問なら run_plan を呼ばずに「答えられない」と伝えてください。
{context}
質問: {question}"""


def preload(store: str) -> None:
    """Load the sample layers into `store` through the MCP server itself."""
    env = dict(os.environ, GENEGIS_LAYER_DIR=store)
    proc = subprocess.Popen([str(MCP)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=env)
    messages = [
        {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "eval", "version": "0"}}},
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "load_sample_data", "arguments": {}}},
    ]
    out, _ = proc.communicate("\n".join(json.dumps(m) for m in messages).encode() + b"\n", timeout=300)
    replies = [json.loads(line) for line in out.decode().splitlines() if line.strip()]
    if replies[-1]["result"].get("isError"):
        raise RuntimeError(f"preload failed: {replies[-1]}")


def run_case(index: int, case: dict, model: str | None, max_turns: int) -> dict:
    store = tempfile.mkdtemp(prefix="genegis-eval-")
    preload(store)
    config = Path(tempfile.mkdtemp(prefix="genegis-eval-config-")) / "mcp.json"
    config.write_text(json.dumps({"mcpServers": {"genegis": {"command": str(MCP), "args": [], "env": {"GENEGIS_LAYER_DIR": store}}}}))
    context = ""
    if case.get("point"):
        lon, lat = case["point"]
        context = f"地図でクリックされた地点: 経度 {lon}, 緯度 {lat}（EPSG:4326）"
    command = [
        "claude", "-p", PROMPT.format(context=context, question=case["prompt"]),
        "--mcp-config", str(config), "--strict-mcp-config",
        "--allowedTools", "mcp__genegis",
        "--output-format", "json", "--max-turns", str(max_turns),
    ]
    if model:
        command += ["--model", model]
    started = datetime.datetime.now(datetime.timezone.utc)
    completed = subprocess.run(command, capture_output=True, timeout=900, cwd=store, encoding="utf-8", errors="replace")
    try:
        session = json.loads(completed.stdout)
    except json.JSONDecodeError:
        session = {"result": completed.stdout[-500:], "is_error": True, "stderr": completed.stderr[-500:]}
    scored = subprocess.run([str(EVAL), "score-store", store, str(index)], capture_output=True, encoding="utf-8", errors="replace", cwd=ROOT)
    verdict = json.loads(scored.stdout.strip().splitlines()[-1])
    verdict.update({
        "agent_answer": (session.get("result") or "")[:600],
        "turns": session.get("num_turns"),
        "duration_s": round((datetime.datetime.now(datetime.timezone.utc) - started).total_seconds(), 1),
        "cost_usd": session.get("total_cost_usd"),
        "models": sorted((session.get("modelUsage") or {}).keys()),
        "agent_error": session.get("is_error", False),
    })
    return verdict


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--cases", help="comma-separated case indices (default: all)")
    parser.add_argument("--model", help="claude --model value (default: CLI default)")
    parser.add_argument("--max-turns", type=int, default=25)
    parser.add_argument("--report")
    args = parser.parse_args()
    cases = json.loads(subprocess.run([str(EVAL), "cases"], capture_output=True, encoding="utf-8", check=True, cwd=ROOT).stdout)
    indices = [int(i) for i in args.cases.split(",")] if args.cases else range(len(cases))
    rows = []
    for index in indices:
        row = run_case(index, cases[index], args.model, args.max_turns)
        rows.append(row)
        print(f"[{row['verdict']:>10}] {row['prompt']} → {row['steps']} ({row['turns']} turns, ${row['cost_usd']})", flush=True)
    correct = sum(r["verdict"] == "correct" for r in rows)
    held = [r for r in rows if r["held_out"]]
    held_correct = sum(r["verdict"] == "correct" for r in held)
    print(f"{correct}/{len(rows)} correct, held-out {held_correct}/{len(held)}")
    if args.report:
        report = {
            "schema_version": "1.0.0",
            "evaluation": "rfc-0007-planner",
            "mode": "mcp-agent",
            "agent": "claude-code " + subprocess.run(["claude", "--version"], capture_output=True, encoding="utf-8").stdout.strip(),
            "models": sorted({m for r in rows for m in r["models"]}),
            "observed_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
            "correct": correct,
            "total": len(rows),
            "held_out_correct": held_correct,
            "held_out_total": len(held),
            "criteria": "the last verified analysis the agent stored matches a hand-written ground-truth plan within 0.5 %; refusal cases must produce no analysis; the agent's prose is not scored",
            "cases": rows,
        }
        Path(args.report).write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        print(f"report → {args.report}")


if __name__ == "__main__":
    sys.exit(main())
