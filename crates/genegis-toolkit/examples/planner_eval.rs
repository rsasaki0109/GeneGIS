//! Planner evaluation over the Nagoya sample layers.
//!
//! Each case pairs a question with a hand-written ground-truth plan. A
//! planner answer is correct when its verified output contains a numeric
//! field whose values match the ground truth's key field (same feature count,
//! values within 0.5 %). Refusal cases expect the planner to decline.
//!
//! ```text
//! cargo run -p genegis-toolkit --example planner_eval -- rule --require-calibration
//! cargo run -p genegis-toolkit --example planner_eval -- llm [report.json]
//! ```
//!
//! LLM settings come from `GENEGIS_LLM_*` or `.genegis/llm.env`
//! (`KEY=VALUE` lines); the key is never printed.
//!
//! Agents that plan through the MCP server (for example Claude Code with
//! `genegis-mcp`) are scored from the layer store they wrote to:
//!
//! ```text
//! planner_eval cases                      # list cases as JSON
//! planner_eval score-store <dir> <index>  # score the last analysis in <dir>
//! ```

use std::collections::BTreeMap;

use genegis_toolkit::execute::{run_plan, ToolkitPlan};
use genegis_toolkit::planner::{plan, LlmConfig, PlannerContext, PlannerMode};
use genegis_toolkit::samples::{load_nagoya_samples, samples_dir};
use genegis_toolkit::{FieldType, Layer, LayerStore};
use serde_json::{json, Value};

struct Case {
    prompt: &'static str,
    point: Option<[f64; 2]>,
    /// Ground-truth plan template (`{wards}` etc. are replaced by layer IDs)
    /// and the output field holding the answer. `None` = must refuse.
    truth: Option<(Value, &'static str)>,
    rules_expected: bool,
    /// Held-out cases were written after the rule planner was tuned and are
    /// never used to tune it. Once a held-out set has driven a change it is
    /// demoted to calibration and a fresh set is written first.
    held_out: bool,
}

fn cases() -> Vec<Case> {
    let near_station = |metres: &str| {
        json!({"goal": "", "steps": [
            {"id": "sel", "op": "select_by_location", "inputs": {"layer": "{shelters}", "other": "{stations}"}, "params": {"predicate": "within_distance", "distance": metres}},
            {"id": "out", "op": "summarize", "inputs": {"layer": "sel"}, "params": {"aggregates": [{"op": "count"}]}}]})
    };
    vec![
        Case {
            prompt: "駅から500m以内の避難所を数えて",
            point: None,
            truth: Some((near_station("500 m"), "count")),
            rules_expected: true,
            held_out: false,
        },
        Case {
            prompt: "この点から1km以内の人口は？",
            point: Some([136.9066, 35.1815]),
            truth: Some((
                json!({"goal": "", "steps": [
                {"id": "p", "op": "make_points", "params": {"points": [[136.9066, 35.1815]]}},
                {"id": "b", "op": "buffer", "inputs": {"layer": "p"}, "params": {"distance": "1 km"}},
                {"id": "out", "op": "spatial_join", "inputs": {"target": "b", "join": "{wards}"}, "params": {"aggregates": [{"op": "area_weighted_sum", "field": "population", "as": "population"}]}}]}),
                "population",
            )),
            rules_expected: true,
            held_out: false,
        },
        Case {
            prompt: "区ごとの店舗数",
            point: None,
            truth: Some((
                json!({"goal": "", "steps": [
                {"id": "out", "op": "spatial_join", "inputs": {"target": "{wards}", "join": "{pois}"}, "params": {"aggregates": [{"op": "count"}]}}]}),
                "count",
            )),
            rules_expected: true,
            held_out: false,
        },
        Case {
            prompt: "区の人口密度",
            point: None,
            truth: Some((
                json!({"goal": "", "steps": [
                {"id": "a", "op": "measure", "inputs": {"layer": "{wards}"}, "params": {"area_unit": "km2"}},
                {"id": "out", "op": "calculate", "inputs": {"layer": "a"}, "params": {"field": "d", "expression": "population / area_km2"}}]}),
                "d",
            )),
            rules_expected: true,
            held_out: false,
        },
        Case {
            prompt: "浸水想定区域の中にある避難所の収容人数の合計は？",
            point: None,
            truth: Some((
                json!({"goal": "", "steps": [
                {"id": "v", "op": "make_valid", "inputs": {"layer": "{flood}"}},
                {"id": "c", "op": "clip", "inputs": {"layer": "{shelters}", "mask": "v"}},
                {"id": "out", "op": "summarize", "inputs": {"layer": "c"}, "params": {"aggregates": [{"op": "sum", "field": "capacity", "as": "v"}]}}]}),
                "v",
            )),
            rules_expected: false,
            held_out: false,
        },
        Case {
            prompt: "浸水想定区域の外にある避難所はいくつ？",
            point: None,
            truth: Some((
                json!({"goal": "", "steps": [
                {"id": "v", "op": "make_valid", "inputs": {"layer": "{flood}"}},
                {"id": "e", "op": "erase", "inputs": {"layer": "{shelters}", "mask": "v"}},
                {"id": "out", "op": "summarize", "inputs": {"layer": "e"}, "params": {"aggregates": [{"op": "count"}]}}]}),
                "count",
            )),
            rules_expected: false,
            held_out: false,
        },
        Case {
            prompt: "人口が15万人以上の区はいくつある？",
            point: None,
            truth: Some((
                json!({"goal": "", "steps": [
                {"id": "f", "op": "filter", "inputs": {"layer": "{wards}"}, "params": {"where": "population >= 150000"}},
                {"id": "out", "op": "summarize", "inputs": {"layer": "f"}, "params": {"aggregates": [{"op": "count"}]}}]}),
                "count",
            )),
            rules_expected: false,
            held_out: false,
        },
        Case {
            prompt: "店舗から最寄りの避難所までの平均距離は何メートル？",
            point: None,
            truth: Some((
                json!({"goal": "", "steps": [
                {"id": "n", "op": "distance_to_nearest", "inputs": {"layer": "{pois}", "target": "{shelters}"}},
                {"id": "out", "op": "summarize", "inputs": {"layer": "n"}, "params": {"aggregates": [{"op": "mean", "field": "nearest_distance_m", "as": "v"}]}}]}),
                "v",
            )),
            rules_expected: false,
            held_out: false,
        },
        Case {
            prompt: "各区の面積をヘクタールで出して",
            point: None,
            truth: Some((
                json!({"goal": "", "steps": [
                {"id": "out", "op": "measure", "inputs": {"layer": "{wards}"}, "params": {"area_unit": "ha"}}]}),
                "area_ha",
            )),
            rules_expected: false,
            held_out: false,
        },
        Case {
            prompt: "栄駅から2km以内にある店舗の数",
            point: None,
            truth: Some((
                json!({"goal": "", "steps": [
                {"id": "s", "op": "filter", "inputs": {"layer": "{stations}"}, "params": {"where": "駅名 = '栄'"}},
                {"id": "sel", "op": "select_by_location", "inputs": {"layer": "{pois}", "other": "s"}, "params": {"predicate": "within_distance", "distance": "2 km"}},
                {"id": "out", "op": "summarize", "inputs": {"layer": "sel"}, "params": {"aggregates": [{"op": "count"}]}}]}),
                "count",
            )),
            rules_expected: false,
            held_out: false,
        },
        Case {
            prompt: "浸水想定区域から1km以内にある避難所の収容人数の合計は？",
            point: None,
            truth: Some((
                json!({"goal": "", "steps": [
                {"id": "v", "op": "make_valid", "inputs": {"layer": "{flood}"}},
                {"id": "b", "op": "buffer", "inputs": {"layer": "v"}, "params": {"distance": "1 km", "dissolve": true}},
                {"id": "c", "op": "clip", "inputs": {"layer": "{shelters}", "mask": "b"}},
                {"id": "out", "op": "summarize", "inputs": {"layer": "c"}, "params": {"aggregates": [{"op": "sum", "field": "capacity", "as": "v"}]}}]}),
                "v",
            )),
            rules_expected: false,
            held_out: false,
        },
        Case {
            prompt: "避難所から1km以内にない店舗はいくつ？",
            point: None,
            truth: Some((
                json!({"goal": "", "steps": [
                {"id": "b", "op": "buffer", "inputs": {"layer": "{shelters}"}, "params": {"distance": "1 km", "dissolve": true}},
                {"id": "e", "op": "erase", "inputs": {"layer": "{pois}", "mask": "b"}},
                {"id": "out", "op": "summarize", "inputs": {"layer": "e"}, "params": {"aggregates": [{"op": "count"}]}}]}),
                "count",
            )),
            rules_expected: false,
            held_out: false,
        },
        Case {
            prompt: "収容人数が3000人以上の避難所から500m以内にある店舗の数",
            point: None,
            truth: Some((
                json!({"goal": "", "steps": [
                {"id": "f", "op": "filter", "inputs": {"layer": "{shelters}"}, "params": {"where": "capacity >= 3000"}},
                {"id": "sel", "op": "select_by_location", "inputs": {"layer": "{pois}", "other": "f"}, "params": {"predicate": "within_distance", "distance": "500 m"}},
                {"id": "out", "op": "summarize", "inputs": {"layer": "sel"}, "params": {"aggregates": [{"op": "count"}]}}]}),
                "count",
            )),
            rules_expected: false,
            held_out: false,
        },
        Case {
            prompt: "区ごとの避難所の収容人数の合計",
            point: None,
            truth: Some((
                json!({"goal": "", "steps": [
                {"id": "out", "op": "spatial_join", "inputs": {"target": "{wards}", "join": "{shelters}"}, "params": {"aggregates": [{"op": "sum", "field": "capacity", "as": "v"}]}}]}),
                "v",
            )),
            rules_expected: false,
            held_out: false,
        },
        Case {
            prompt: "店舗が10件以上ある区の人口の合計は？",
            point: None,
            truth: Some((
                json!({"goal": "", "steps": [
                {"id": "j", "op": "spatial_join", "inputs": {"target": "{wards}", "join": "{pois}"}, "params": {"aggregates": [{"op": "count", "as": "n"}]}},
                {"id": "f", "op": "filter", "inputs": {"layer": "j"}, "params": {"where": "n >= 10"}},
                {"id": "out", "op": "summarize", "inputs": {"layer": "f"}, "params": {"aggregates": [{"op": "sum", "field": "population", "as": "v"}]}}]}),
                "v",
            )),
            rules_expected: false,
            held_out: false,
        },
        // Held-out set 2 (written 2026-09-26 before the planner safety work).
        Case {
            prompt: "浸水想定区域に含まれない店舗の数",
            point: None,
            truth: Some((json!({"goal": "", "steps": [
                {"id": "v", "op": "make_valid", "inputs": {"layer": "{flood}"}},
                {"id": "e", "op": "erase", "inputs": {"layer": "{pois}", "mask": "v"}},
                {"id": "out", "op": "summarize", "inputs": {"layer": "e"}, "params": {"aggregates": [{"op": "count"}]}}]}), "count")),
            rules_expected: false,
            held_out: true,
        },
        Case {
            prompt: "栄駅か名古屋駅から1km以内にある避難所の数",
            point: None,
            truth: Some((json!({"goal": "", "steps": [
                {"id": "s", "op": "filter", "inputs": {"layer": "{stations}"}, "params": {"where": "駅名 IN ('栄', '名古屋')"}},
                {"id": "sel", "op": "select_by_location", "inputs": {"layer": "{shelters}", "other": "s"}, "params": {"predicate": "within_distance", "distance": "1 km"}},
                {"id": "out", "op": "summarize", "inputs": {"layer": "sel"}, "params": {"aggregates": [{"op": "count"}]}}]}), "count")),
            rules_expected: false,
            held_out: true,
        },
        Case {
            prompt: "人口が10万人未満の区の面積の合計は何km²？",
            point: None,
            truth: Some((json!({"goal": "", "steps": [
                {"id": "f", "op": "filter", "inputs": {"layer": "{wards}"}, "params": {"where": "population < 100000"}},
                {"id": "m", "op": "measure", "inputs": {"layer": "f"}, "params": {"area_unit": "km2"}},
                {"id": "out", "op": "summarize", "inputs": {"layer": "m"}, "params": {"aggregates": [{"op": "sum", "field": "area_km2", "as": "v"}]}}]}), "v")),
            rules_expected: false,
            held_out: true,
        },
        Case {
            prompt: "避難所から最寄り駅までの距離の最大値は？",
            point: None,
            truth: Some((json!({"goal": "", "steps": [
                {"id": "n", "op": "distance_to_nearest", "inputs": {"layer": "{shelters}", "target": "{stations}"}},
                {"id": "out", "op": "summarize", "inputs": {"layer": "n"}, "params": {"aggregates": [{"op": "max", "field": "nearest_distance_m", "as": "v"}]}}]}), "v")),
            rules_expected: false,
            held_out: true,
        },
        Case {
            prompt: "千種区にある店舗の数は？",
            point: None,
            truth: Some((json!({"goal": "", "steps": [
                {"id": "w", "op": "filter", "inputs": {"layer": "{wards}"}, "params": {"where": "ward_name = '千種区'"}},
                {"id": "sel", "op": "select_by_location", "inputs": {"layer": "{pois}", "other": "w"}, "params": {"predicate": "intersects"}},
                {"id": "out", "op": "summarize", "inputs": {"layer": "sel"}, "params": {"aggregates": [{"op": "count"}]}}]}), "count")),
            rules_expected: false,
            held_out: true,
        },
        Case { prompt: "5年前と比べて名古屋市の人口は増えた？", point: None, truth: None, rules_expected: false, held_out: true },
        Case {
            prompt: "明日の名古屋の天気を教えて",
            point: None,
            truth: None,
            rules_expected: true,
            held_out: false,
        },
        Case {
            prompt: "東京タワーの高さは？",
            point: None,
            truth: None,
            rules_expected: true,
            held_out: false,
        },
    ]
}

fn load_layers() -> (BTreeMap<String, Layer>, BTreeMap<&'static str, String>) {
    let loaded = load_nagoya_samples(&samples_dir()).expect("sample data");
    let mut layers = BTreeMap::new();
    let mut ids = BTreeMap::new();
    for (key, layer, _) in loaded {
        ids.insert(key, layer.id());
        layers.insert(layer.id(), layer);
    }
    (layers, ids)
}

fn instantiate(template: &Value, ids: &BTreeMap<&'static str, String>) -> ToolkitPlan {
    let mut text = template.to_string();
    for (key, id) in ids {
        text = text.replace(&format!("{{{key}}}"), id);
    }
    serde_json::from_str(&text).expect("truth plan")
}

fn values(layer: &Layer, field: &str) -> Vec<f64> {
    let mut v: Vec<f64> = layer
        .features
        .iter()
        .filter_map(|f| f.properties.get(field).and_then(Value::as_f64))
        .collect();
    v.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
    v
}

fn matches(output: &Layer, truth: &[f64]) -> Option<String> {
    // A single-feature truth may also be answered by the feature count itself
    // (e.g. the planner returns the selected features instead of a count).
    for field in output
        .fields
        .iter()
        .filter(|f| matches!(f.field_type, FieldType::Integer | FieldType::Float))
    {
        let got = values(output, &field.name);
        if got.len() == truth.len()
            && got
                .iter()
                .zip(truth)
                .all(|(a, b)| (a - b).abs() <= b.abs() * 0.005 + 1e-6)
        {
            return Some(field.name.clone());
        }
    }
    if truth.len() == 1 && (output.features.len() as f64 - truth[0]).abs() < 1e-9 {
        return Some("feature_count".into());
    }
    None
}

fn load_env_file() {
    if let Ok(text) = std::fs::read_to_string(".genegis/llm.env") {
        for line in text.lines() {
            if let Some((key, value)) = line.trim().split_once('=') {
                if key.starts_with("GENEGIS_LLM_") && std::env::var(key).is_err() {
                    std::env::set_var(key, value.trim().trim_matches('"'));
                }
            }
        }
    }
}

/// Score the most recent analysis stored in `dir` against case `index`.
fn score_store(dir: &str, index: usize) {
    let case = cases().into_iter().nth(index).expect("case index");
    let (layers, ids) = load_layers();
    let (store, warnings) = LayerStore::open(dir).expect("open store");
    assert!(warnings.is_empty(), "{warnings:?}");
    let analyses: Vec<_> = store
        .list()
        .into_iter()
        .filter(|s| s.receipt.get("kind").and_then(Value::as_str) == Some("analysis"))
        .collect();
    let (verdict, detail) = match (&case.truth, analyses.last()) {
        (None, None) => ("correct", "declined (no verified analysis)".to_string()),
        (None, Some(_)) => (
            "wrong",
            "should have declined but ran an analysis".to_string(),
        ),
        (Some(_), None) => (
            "unresolved",
            "no verified analysis was produced".to_string(),
        ),
        (Some((template, key)), Some(last)) => {
            let truth_run =
                run_plan(&instantiate(template, &ids), &layers).expect("truth plan runs");
            let truth = values(&truth_run.output, key);
            let output = &store.get(&last.summary.id).expect("stored").layer;
            match matches(output, &truth) {
                Some(field) => ("correct", format!("{field} matches truth {truth:?}")),
                None => ("wrong", format!("no field matches truth {truth:?}")),
            }
        }
    };
    let steps: Vec<Value> = analyses
        .last()
        .and_then(|a| {
            a.receipt
                .pointer("/plan/steps")
                .and_then(Value::as_array)
                .cloned()
        })
        .unwrap_or_default()
        .iter()
        .map(|s| s["op"].clone())
        .collect();
    println!(
        "{}",
        json!({"prompt": case.prompt, "verdict": verdict, "detail": detail, "steps": steps,
               "analyses": analyses.len(), "held_out": case.held_out, "rules_expected": case.rules_expected})
    );
}

fn main() {
    load_env_file();
    let mut args = std::env::args().skip(1);
    let first = args.next();
    match first.as_deref() {
        Some("cases") => {
            let list: Vec<Value> = cases()
                .iter()
                .map(|c| json!({"prompt": c.prompt, "point": c.point, "held_out": c.held_out, "expects_answer": c.truth.is_some()}))
                .collect();
            println!("{}", Value::Array(list));
            return;
        }
        Some("score-store") => {
            let dir = args.next().expect("store dir");
            let index: usize = args.next().expect("case index").parse().expect("index");
            score_store(&dir, index);
            return;
        }
        _ => {}
    }
    let mut args = first.into_iter().chain(args);
    let mode = match args.next().as_deref() {
        Some("llm") => PlannerMode::Llm,
        Some("auto") => PlannerMode::Auto,
        _ => PlannerMode::Rule,
    };
    let rest: Vec<String> = args.collect();
    let require_calibration = rest.iter().any(|a| a == "--require-calibration");
    let report_path = rest.into_iter().find(|a| !a.starts_with("--"));
    let llm = LlmConfig::from_env();
    let (layers, ids) = load_layers();
    let mut rows = Vec::new();
    let (mut correct, mut total) = (0, 0);
    let (mut held_correct, mut held_total) = (0, 0);
    for case in cases() {
        total += 1;
        let context = PlannerContext {
            point: case.point,
            selected_layer: None,
        };
        let planned = plan(case.prompt, &layers, &context, mode, &llm);
        let (verdict, detail, steps) = match (&case.truth, planned) {
            (None, Err(e)) => ("correct", format!("declined: {e}"), vec![]),
            (None, Ok(p)) => (
                "wrong",
                "should have declined".to_string(),
                p.plan.steps.iter().map(|s| s.op.clone()).collect(),
            ),
            (Some(_), Err(e)) => ("unresolved", e.to_string(), vec![]),
            (Some((template, key)), Ok(p)) => {
                let steps: Vec<String> = p.plan.steps.iter().map(|s| s.op.clone()).collect();
                let truth_run =
                    run_plan(&instantiate(template, &ids), &layers).expect("truth plan runs");
                let truth = values(&truth_run.output, key);
                match run_plan(&p.plan, &layers) {
                    Err(e) => (
                        "rejected",
                        format!("execution/verification rejected: {e}"),
                        steps,
                    ),
                    Ok(run) => match matches(&run.output, &truth) {
                        Some(field) => {
                            ("correct", format!("{field} matches truth {truth:?}"), steps)
                        }
                        None => ("wrong", format!("no field matches truth {truth:?}"), steps),
                    },
                }
            }
        };
        if verdict == "correct" {
            correct += 1;
        }
        if case.held_out {
            held_total += 1;
            if verdict == "correct" {
                held_correct += 1;
            }
        }
        println!(
            "[{verdict:>10}] {} → {:?} {}",
            case.prompt,
            steps,
            detail.chars().take(140).collect::<String>()
        );
        rows.push(json!({"prompt": case.prompt, "verdict": verdict, "steps": steps, "detail": detail, "rules_expected": case.rules_expected, "held_out": case.held_out}));
    }
    println!(
        "{correct}/{total} correct, held-out {held_correct}/{held_total} ({:?})",
        mode
    );
    let calibration_failures = rows
        .iter()
        .filter(|r| {
            let known = r["held_out"] == json!(false);
            let must_answer = r["rules_expected"] == json!(true);
            let verdict = r["verdict"].as_str().unwrap_or("");
            // Known question shapes must be answered correctly; nothing
            // known may be answered wrongly (declining is acceptable).
            known && ((must_answer && verdict != "correct") || verdict == "wrong" || verdict == "rejected")
        })
        .count();
    if let Some(path) = report_path {
        let report = json!({
            "schema_version": "1.0.0",
            "evaluation": "rfc-0007-planner",
            "mode": format!("{mode:?}").to_lowercase(),
            "model": if mode == PlannerMode::Rule { Value::Null } else { json!(llm.model) },
            "endpoint": if mode == PlannerMode::Rule { Value::Null } else { json!(llm.base_url) },
            "observed_at": chrono::Utc::now().to_rfc3339(),
            "correct": correct,
            "total": total,
            "held_out_correct": held_correct,
            "held_out_total": held_total,
            "criteria": "answer field matches a hand-written ground-truth plan within 0.5 %; refusal cases must decline; every plan executes through RunWorkflow with all checks passing",
            "cases": rows,
        });
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&report).expect("report") + "\n",
        )
        .expect("write report");
        println!("report → {path}");
    }
    if require_calibration && calibration_failures > 0 {
        eprintln!("{calibration_failures} calibration cases regressed");
        std::process::exit(1);
    }
}
