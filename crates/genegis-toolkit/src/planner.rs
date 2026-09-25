//! Natural language → [`ToolkitPlan`].
//!
//! Two planners compose the same operation catalog:
//!
//! * a deterministic rule planner for common Japanese/English spatial
//!   questions (offline, used in CI and as the fallback), and
//! * an LLM planner (OpenAI-compatible chat completions, configured with the
//!   same `GENEGIS_LLM_*` variables as `genegis-ai`) that may assemble any
//!   graph from the catalog.
//!
//! Neither planner verifies anything: every plan is structurally validated
//! against the loaded layers here, then executed through the Command bus
//! where each operation's independent checks decide whether the result is
//! accepted. LLM output is never its own verifier.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{Result, ToolkitError};
use crate::execute::{validate_plan, PlanStep, ToolkitPlan};
use crate::layer::{FieldType, GeometryKind, Layer};
use crate::ops;

/// Optional interaction context from the map.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PlannerContext {
    /// Last clicked location `[lon, lat]` (EPSG:4326), for "この点" / "here".
    pub point: Option<[f64; 2]>,
    /// Currently selected layer ID.
    pub selected_layer: Option<String>,
}

/// Which planner to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlannerMode {
    /// LLM when configured, otherwise rules.
    #[default]
    Auto,
    /// Deterministic rules only.
    Rule,
    /// LLM only (fails if not configured).
    Llm,
}

/// Planner output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannerResult {
    /// The validated plan.
    pub plan: ToolkitPlan,
    /// `rule` or `llm`.
    pub backend: String,
    /// Planner confidence in 0–1.
    pub confidence: f32,
    /// Why this plan.
    pub rationale: Vec<String>,
    /// Open questions the user may want to resolve.
    pub ambiguities: Vec<String>,
    /// LLM attempts that were rejected by validation (with reasons).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rejected_attempts: Vec<String>,
}

/// OpenAI-compatible LLM configuration.
#[derive(Debug, Clone)]
pub struct LlmConfig {
    /// API key.
    pub api_key: Option<String>,
    /// Base URL ending before `/chat/completions`.
    pub base_url: String,
    /// Model name.
    pub model: String,
}

impl LlmConfig {
    /// Read `GENEGIS_LLM_API_KEY`, `GENEGIS_LLM_BASE_URL`, `GENEGIS_LLM_MODEL`.
    pub fn from_env() -> Self {
        Self {
            api_key: std::env::var("GENEGIS_LLM_API_KEY")
                .ok()
                .filter(|k| !k.trim().is_empty()),
            base_url: std::env::var("GENEGIS_LLM_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".into()),
            model: std::env::var("GENEGIS_LLM_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into()),
        }
    }

    /// Whether an API key is configured.
    pub fn ready(&self) -> bool {
        self.api_key.is_some()
    }
}

/// Plan a prompt against the loaded layers.
pub fn plan(
    prompt: &str,
    layers: &BTreeMap<String, Layer>,
    context: &PlannerContext,
    mode: PlannerMode,
    llm: &LlmConfig,
) -> Result<PlannerResult> {
    let use_llm = match mode {
        PlannerMode::Rule => false,
        PlannerMode::Llm => {
            if !llm.ready() {
                return Err(ToolkitError::Unresolved(
                    "GENEGIS_LLM_API_KEY is not set".into(),
                ));
            }
            true
        }
        PlannerMode::Auto => llm.ready(),
    };
    if use_llm {
        match plan_with_llm(prompt, layers, context, llm) {
            Ok(result) => return Ok(result),
            Err(error) if mode == PlannerMode::Auto => {
                let mut result = plan_with_rules(prompt, layers, context)?;
                result
                    .rejected_attempts
                    .push(format!("LLM planner failed, fell back to rules: {error}"));
                return Ok(result);
            }
            Err(error) => return Err(error),
        }
    }
    plan_with_rules(prompt, layers, context)
}

// ---------------------------------------------------------------------------
// Rule planner
// ---------------------------------------------------------------------------

const SYNONYMS: &[(&str, &[&str])] = &[
    ("駅", &["station", "stations", "駅"]),
    ("避難所", &["shelter", "shelters", "避難所", "避難場所"]),
    ("学校", &["school", "schools", "学校", "小学校", "中学校"]),
    ("病院", &["hospital", "hospitals", "病院", "医療機関"]),
    (
        "店",
        &[
            "shop",
            "shops",
            "store",
            "stores",
            "店",
            "店舗",
            "スーパー",
            "supermarket",
            "poi",
            "pois",
        ],
    ),
    ("公園", &["park", "parks", "公園"]),
    ("区", &["ward", "wards", "区", "行政区", "zones"]),
    ("浸水", &["flood", "浸水", "洪水", "浸水想定"]),
    ("道路", &["road", "roads", "道路", "network"]),
    ("施設", &["facility", "facilities", "施設", "poi", "pois"]),
];

fn normalize(text: &str) -> String {
    // Full-width ASCII → half-width, lower-case.
    text.chars()
        .map(|c| {
            let code = c as u32;
            if (0xFF01..=0xFF5E).contains(&code) {
                char::from_u32(code - 0xFEE0).unwrap_or(c)
            } else if c == '　' {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .to_lowercase()
}

/// Aliases a layer can be referred to by.
fn layer_aliases(layer: &Layer) -> Vec<String> {
    let name = normalize(&layer.name);
    let mut aliases = vec![name.clone()];
    for part in name.split(['_', '-', ' ', '.', '（', '）', '(', ')', '・', '/', '／']) {
        if part.chars().count() >= 2
            && ![
                "nagoya",
                "real",
                "data",
                "layer",
                "2020",
                "名古屋",
                "名古屋市",
                "サンプル",
                "sample",
            ]
            .contains(&part)
        {
            aliases.push(part.to_string());
        }
    }
    // Multi-character synonyms match anywhere in the name. A single kanji
    // (区, 駅, 店) matches only when the name belongs to no other group via a
    // multi-character word, so 浸水想定区域 is a flood layer, not a ward.
    let multi_groups: Vec<usize> = SYNONYMS
        .iter()
        .enumerate()
        .filter(|(_, (_, words))| {
            words
                .iter()
                .any(|w| w.chars().count() >= 2 && name.contains(&normalize(w)))
        })
        .map(|(i, _)| i)
        .collect();
    for (index, (_, words)) in SYNONYMS.iter().enumerate() {
        let single = words
            .iter()
            .any(|w| w.chars().count() == 1 && name.contains(&normalize(w)));
        if multi_groups.contains(&index) || (single && multi_groups.is_empty()) {
            aliases.extend(words.iter().map(|w| normalize(w)));
        }
    }
    aliases.sort_by_key(|a| std::cmp::Reverse(a.chars().count()));
    aliases.dedup();
    aliases
}

/// Whether a layer is source data (not a stored analysis result or a
/// geometry-less table). Analysis outputs are named after the question that
/// produced them, so matching them by name would hijack later questions.
fn is_source(layer: &Layer) -> bool {
    layer.provenance.format != "derived" && layer.geometry_kind() != GeometryKind::None
}

/// Layer mentions in the prompt: (byte position, layer ID). Source layers
/// win; derived layers are only considered when no source layer matches.
fn mentions(prompt: &str, layers: &BTreeMap<String, Layer>) -> Vec<(usize, String)> {
    let sources: BTreeMap<String, Layer> = layers
        .iter()
        .filter(|(_, l)| is_source(l))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let found = mentions_in(prompt, &sources);
    if found.is_empty() {
        let usable: BTreeMap<String, Layer> = layers
            .iter()
            .filter(|(_, l)| l.geometry_kind() != GeometryKind::None)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        return mentions_in(prompt, &usable);
    }
    found
}

fn mentions_in(prompt: &str, layers: &BTreeMap<String, Layer>) -> Vec<(usize, String)> {
    // Every (start, end, layer) occurrence of every alias.
    let mut spans: Vec<(usize, usize, String)> = Vec::new();
    for (id, layer) in layers {
        for alias in layer_aliases(layer) {
            if alias.is_empty() {
                continue;
            }
            for (pos, _) in prompt.match_indices(&alias) {
                spans.push((pos, pos + alias.len(), id.clone()));
            }
        }
    }
    // Longest matches claim their text first, so 区 inside 浸水想定区域 does
    // not also count as a mention of the ward layer.
    spans.sort_by(|a, b| (b.1 - b.0).cmp(&(a.1 - a.0)).then(a.0.cmp(&b.0)));
    let mut claimed: Vec<(usize, usize)> = Vec::new();
    let mut first: BTreeMap<String, usize> = BTreeMap::new();
    for (start, end, id) in spans {
        if claimed.iter().any(|(s, e)| start < *e && *s < end) {
            continue;
        }
        claimed.push((start, end));
        let entry = first.entry(id).or_insert(start);
        *entry = (*entry).min(start);
    }
    let mut found: Vec<(usize, String)> = first.into_iter().map(|(id, pos)| (pos, id)).collect();
    found.sort();
    found
}

/// Extract a distance in metres and the assumption used, if any.
fn extract_distance(prompt: &str) -> Option<(f64, Option<String>)> {
    let chars: Vec<char> = prompt.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let start = i;
            while i < chars.len()
                && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == ',')
            {
                i += 1;
            }
            let number: f64 = chars[start..i]
                .iter()
                .filter(|c| **c != ',')
                .collect::<String>()
                .parse()
                .ok()?;
            let rest: String = chars[i..]
                .iter()
                .collect::<String>()
                .trim_start()
                .to_string();
            let before: String = chars[..start].iter().collect();
            if rest.starts_with("km") || rest.starts_with("キロ") {
                return Some((number * 1000.0, None));
            }
            if rest.starts_with('m') || rest.starts_with("メートル") {
                return Some((number, None));
            }
            if rest.starts_with('分') || rest.starts_with("min") {
                if before.ends_with("徒歩") || before.contains("walk") || before.ends_with("歩いて")
                {
                    return Some((
                        number * 80.0,
                        Some(format!(
                            "徒歩{number}分 = {} m（80 m/分の不動産表示基準で換算）",
                            number * 80.0
                        )),
                    ));
                }
                if before.ends_with("自転車") || before.contains("bike") {
                    return Some((
                        number * 250.0,
                        Some(format!(
                            "自転車{number}分 = {} m（15 km/h で換算）",
                            number * 250.0
                        )),
                    ));
                }
            }
            continue;
        }
        i += 1;
    }
    None
}

fn is_numeric(layer: &Layer, field: &str) -> bool {
    layer
        .field(field)
        .is_some_and(|f| matches!(f.field_type, FieldType::Integer | FieldType::Float))
}

fn population_field(layer: &Layer) -> Option<String> {
    let candidates = [
        "population",
        "pop",
        "人口",
        "総人口",
        "人口総数",
        "persons",
        "jinko",
        "total_pop",
    ];
    layer
        .fields
        .iter()
        .filter(|f| matches!(f.field_type, FieldType::Integer | FieldType::Float))
        .find(|f| {
            let lower = f.name.to_lowercase();
            candidates.iter().any(|c| lower == *c || lower.contains(c))
                || f.unit.as_deref() == Some("persons")
        })
        .map(|f| f.name.clone())
}

/// Japanese words that refer to common numeric fields.
const FIELD_WORDS: &[(&str, &[&str])] = &[
    ("capacity", &["収容人数", "収容", "定員", "capacity"]),
    ("population", &["人口", "population"]),
    ("area", &["面積"]),
];

fn numeric_field_mentioned(prompt: &str, layer: &Layer) -> Option<String> {
    let direct = layer
        .fields
        .iter()
        .filter(|f| is_numeric(layer, &f.name))
        .filter(|f| prompt.contains(&normalize(&f.name)))
        .max_by_key(|f| f.name.chars().count())
        .map(|f| f.name.clone());
    direct.or_else(|| {
        layer
            .fields
            .iter()
            .filter(|f| is_numeric(layer, &f.name))
            .find(|f| {
                let lower = f.name.to_lowercase();
                FIELD_WORDS.iter().any(|(key, words)| {
                    lower.contains(key) && words.iter().any(|w| prompt.contains(w))
                })
            })
            .map(|f| f.name.clone())
    })
}

/// Named features of `layer` mentioned in the prompt, e.g. 「栄駅」「千種区」:
/// text values that select a minority of the layer's features. Returns the
/// field and every matching value (longest match wins per position).
fn named_values(prompt: &str, layer: &Layer) -> Option<(String, Vec<String>)> {
    let total = layer.features.len().max(1);
    let mut best: Option<(String, Vec<String>)> = None;
    for field in layer
        .fields
        .iter()
        .filter(|f| f.field_type == FieldType::Text)
    {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for feature in &layer.features {
            if let Some(value) = feature.properties.get(&field.name).and_then(Value::as_str) {
                *counts.entry(normalize(value.trim())).or_default() += 1;
            }
        }
        let mut hits: Vec<String> = counts
            .into_iter()
            .filter(|(value, count)| {
                value.chars().count() >= 1
                    && value.chars().count() <= 20
                    && *count * 2 <= total
                    && prompt.contains(value.as_str())
            })
            .map(|(value, _)| value)
            .collect();
        // Drop values contained in a longer hit (名古屋 inside 名古屋駅西).
        let snapshot = hits.clone();
        hits.retain(|v| !snapshot.iter().any(|w| w != v && w.contains(v.as_str())));
        // Single-character values only count when followed by the layer alias
        // (栄 in 栄駅), otherwise they match too much text.
        let aliases = layer_aliases(layer);
        hits.retain(|v| {
            v.chars().count() >= 2 || aliases.iter().any(|a| prompt.contains(&format!("{v}{a}")))
        });
        if !hits.is_empty() && best.as_ref().is_none_or(|(_, b)| hits.len() > b.len()) {
            best = Some((field.name.clone(), hits));
        }
    }
    best
}

/// Filter expression selecting the named values.
fn named_expression(field: &str, values: &[String]) -> String {
    let quoted: Vec<String> = values
        .iter()
        .map(|v| format!("'{}'", v.replace('\'', "''")))
        .collect();
    if quoted.len() == 1 {
        format!("\"{field}\" = {}", quoted[0])
    } else {
        format!("\"{field}\" IN ({})", quoted.join(", "))
    }
}

/// Restrict `layer_ref` to the named features the prompt mentions; returns
/// the reference to use afterwards.
fn apply_named(
    rp: &mut RulePlan,
    text: &str,
    layers: &BTreeMap<String, Layer>,
    layer_ref: &str,
    step_id: &str,
) -> String {
    let Some(layer) = layers.get(layer_ref) else {
        return layer_ref.to_string();
    };
    let Some((field, values)) = named_values(text, layer) else {
        return layer_ref.to_string();
    };
    let expression = named_expression(&field, &values);
    rp.steps.push(step(
        step_id,
        "filter",
        &[("layer", layer_ref)],
        json!({"where": expression}),
    ));
    rp.rationale
        .push(format!("{} を {expression} に絞り込む", layer.name));
    rp.used.insert("named");
    step_id.to_string()
}

/// Numeric threshold such as 「人口が15万人以上」 → (operator, value).
fn threshold(prompt: &str) -> Option<(&'static str, f64)> {
    let chars: Vec<char> = prompt.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == ',') {
            i += 1;
        }
        let Ok(mut value) = chars[start..i]
            .iter()
            .filter(|c| **c != ',')
            .collect::<String>()
            .parse::<f64>()
        else {
            continue;
        };
        let mut rest: String = chars[i..].iter().collect();
        for (unit, factor) in [("万", 10_000.0), ("千", 1_000.0), ("億", 100_000_000.0)] {
            if let Some(stripped) = rest.strip_prefix(unit) {
                value *= factor;
                rest = stripped.to_string();
            }
        }
        let rest = rest.trim_start_matches(['人', '件', '所', '棟', ' ']);
        for (word, op) in [
            ("以上", ">="),
            ("以下", "<="),
            ("未満", "<"),
            ("を超える", ">"),
            ("超", ">"),
            ("より多い", ">"),
            ("より少ない", "<"),
        ] {
            if rest.starts_with(word) {
                return Some((op, value));
            }
        }
    }
    None
}

fn any(prompt: &str, words: &[&str]) -> bool {
    words.iter().any(|w| prompt.contains(w))
}

struct RulePlan {
    steps: Vec<PlanStep>,
    rationale: Vec<String>,
    assumptions: Vec<String>,
    ambiguities: Vec<String>,
    confidence: f32,
    /// Conditions detected in the question (negation, threshold, …).
    signals: Vec<(&'static str, &'static str)>,
    /// Conditions the chosen rule actually applied.
    used: std::collections::BTreeSet<&'static str>,
}

/// Conditions whose presence changes the answer. If the question contains
/// one that the chosen rule does not apply, the rule planner declines rather
/// than answering a different question with confidence.
fn detect_signals(text: &str) -> Vec<(&'static str, &'static str)> {
    let mut signals = Vec::new();
    if any(
        text,
        &[
            "ない", "以外", "除く", "除外", "無い", "not ", "without", "except",
        ],
    ) {
        signals.push(("negation", "否定（〜ない・以外）"));
    }
    if threshold(text).is_some() {
        signals.push(("threshold", "数値の条件（以上・未満など）"));
    }
    if any(
        text,
        &[
            "平均",
            "最大",
            "最小",
            "最も遠い",
            "最も多い",
            "最も少ない",
            "mean",
            "average",
            "maximum",
            "minimum",
        ],
    ) {
        signals.push(("statistic", "統計値（平均・最大など）"));
    }
    if any(text, &["合計", "総計", "sum", "total"]) {
        signals.push(("sum", "合計"));
    }
    if any(
        text,
        &[
            "年前", "前年", "昨年", "比べ", "推移", "増え", "減っ", "変化", "trend",
        ],
    ) {
        signals.push(("temporal", "時点の比較"));
    }
    signals
}

fn step(id: &str, op: &str, inputs: &[(&str, &str)], params: Value) -> PlanStep {
    PlanStep {
        id: id.into(),
        op: op.into(),
        inputs: inputs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        params,
    }
}

/// Deterministic planner for common spatial questions.
pub fn plan_with_rules(
    prompt: &str,
    layers: &BTreeMap<String, Layer>,
    context: &PlannerContext,
) -> Result<PlannerResult> {
    let text = normalize(prompt);
    let found = mentions(&text, layers);
    let kind = |id: &str| layers[id].geometry_kind();
    let distance = extract_distance(&text);
    let here = any(
        &text,
        &[
            "この点",
            "ここ",
            "この地点",
            "クリック",
            "here",
            "this point",
            "this location",
        ],
    );
    let counting = any(
        &text,
        &[
            "数",
            "いくつ",
            "何件",
            "何か所",
            "何箇所",
            "件数",
            "count",
            "how many",
        ],
    );
    let population = any(&text, &["人口", "population", "住んで", "人数"]);
    let density = any(&text, &["密度", "density"]);
    let nearest = any(
        &text,
        &["最寄", "最も近い", "一番近い", "最短", "nearest", "closest"],
    );
    let area = any(&text, &["面積", "area", "広さ"]);
    let clip_words = any(
        &text,
        &["切り抜", "クリップ", "clip", "内側だけ", "の範囲内"],
    );
    let per_polygon = any(
        &text,
        &["ごと", "毎", "別の", "per ", "by ", "for each", "各"],
    );
    let reproject = text.find("epsg:").map(|i| {
        text[i..]
            .chars()
            .take_while(|c| !c.is_whitespace() && *c != 'に' && *c != 'へ')
            .collect::<String>()
    });

    let mut rp = RulePlan {
        steps: Vec::new(),
        rationale: Vec::new(),
        assumptions: Vec::new(),
        ambiguities: Vec::new(),
        confidence: 0.8,
        signals: detect_signals(&text),
        used: Default::default(),
    };
    if layers
        .values()
        .filter(|l| is_source(l))
        .any(|l| named_values(&text, l).is_some())
    {
        rp.signals.push(("named", "地物名による絞り込み"));
    }
    if let Some((_, Some(assumption))) = &distance {
        rp.assumptions.push(assumption.clone());
    }
    let polygon_layers: Vec<&String> = found
        .iter()
        .map(|(_, id)| id)
        .filter(|id| kind(id) == GeometryKind::Polygon)
        .collect();
    let population_source = || -> Option<(String, String)> {
        // Prefer a mentioned polygon layer with a population field, then any loaded one.
        polygon_layers
            .iter()
            .map(|id| id.to_string())
            .chain(
                layers
                    .keys()
                    .filter(|id| {
                        layers[*id].geometry_kind() == GeometryKind::Polygon
                            && is_source(&layers[*id])
                    })
                    .cloned(),
            )
            .find_map(|id| population_field(&layers[&id]).map(|f| (id, f)))
    };

    // 1. "この点から 1km 以内の人口" / "この点から500m以内の避難所"
    if here && distance.is_some() {
        let point = context.point.ok_or_else(|| {
            ToolkitError::Unresolved(
                "「この点」を使うには先に地図をクリックして地点を選んでください".into(),
            )
        })?;
        let (metres, _) = distance.clone().expect("checked");
        rp.steps.push(step(
            "here",
            "make_points",
            &[],
            json!({"points": [{"lon": point[0], "lat": point[1], "name": "選択地点"}], "name": "選択地点"}),
        ));
        rp.rationale.push(format!(
            "選択地点 ({:.5}, {:.5}) を起点にする",
            point[0], point[1]
        ));
        let target = found
            .iter()
            .map(|(_, id)| id.clone())
            .find(|id| kind(id) != GeometryKind::Polygon || !population);
        if population {
            let (source, field) = population_source()
                .ok_or_else(|| ToolkitError::Unresolved("人口フィールドを持つポリゴンレイヤがありません（population / 人口 などの列を含むデータを読み込んでください）".into()))?;
            rp.steps.push(step(
                "area",
                "buffer",
                &[("layer", "here")],
                json!({"distance": format!("{metres} m")}),
            ));
            rp.steps.push(step(
                "result",
                "spatial_join",
                &[("target", "area"), ("join", &source)],
                json!({"aggregates": [{"op": "area_weighted_sum", "field": field, "as": "population", "unit": "persons"}]}),
            ));
            rp.rationale.push(format!(
                "半径 {metres} m の円と {} を面積按分して人口を推計",
                layers[&source].name
            ));
            rp.assumptions
                .push("人口は各ポリゴン内に一様に分布すると仮定（面積按分）".into());
        } else if let Some(target) = target {
            rp.steps.push(step(
                "result",
                "select_by_location",
                &[("layer", &target), ("other", "here")],
                json!({"predicate": "within_distance", "distance": format!("{metres} m")}),
            ));
            rp.rationale.push(format!(
                "選択地点から {metres} m 以内の {} を選択",
                layers[&target].name
            ));
            if counting {
                rp.steps.last_mut().expect("pushed").id = "selected".into();
                rp.steps.push(step(
                    "result",
                    "summarize",
                    &[("layer", "selected")],
                    json!({"aggregates": [{"op": "count"}]}),
                ));
            }
        } else {
            rp.steps.push(step(
                "result",
                "buffer",
                &[("layer", "here")],
                json!({"distance": format!("{metres} m")}),
            ));
            rp.rationale
                .push(format!("選択地点の半径 {metres} m の範囲を作成"));
        }
        return finish(prompt, rp, layers);
    }

    // 2. "A から 500m 以内の B（を数えて）" / "各 A から 500m 以内の B の数"
    if let (Some((metres, _)), true) = (&distance, found.len() >= 2) {
        let from_pos = text
            .find("から")
            .or_else(|| text.find("from"))
            .or_else(|| text.find("within"));
        let (reference, subject) = match from_pos {
            Some(pos) => {
                let before: Vec<&String> = found
                    .iter()
                    .filter(|(p, _)| *p < pos)
                    .map(|(_, id)| id)
                    .collect();
                let after: Vec<&String> = found
                    .iter()
                    .filter(|(p, _)| *p > pos)
                    .map(|(_, id)| id)
                    .collect();
                match (before.last(), after.first()) {
                    (Some(a), Some(b)) => ((*a).clone(), (*b).clone()),
                    _ => (found[0].1.clone(), found[1].1.clone()),
                }
            }
            None => (found[0].1.clone(), found[1].1.clone()),
        };
        // 「収容人数が3000人以上の避難所から…」: a threshold on a field of the
        // reference (or subject) layer restricts that layer first.
        let (reference, subject) = match threshold(&text) {
            Some((op, value)) => {
                let on_reference =
                    numeric_field_mentioned(&text, &layers[&reference]).map(|f| (true, f));
                let on_subject =
                    numeric_field_mentioned(&text, &layers[&subject]).map(|f| (false, f));
                match on_reference.or(on_subject) {
                    Some((is_reference, field)) => {
                        let target = if is_reference { &reference } else { &subject };
                        let expression = format!("\"{field}\" {op} {value}");
                        rp.steps.push(step(
                            "limited",
                            "filter",
                            &[("layer", target)],
                            json!({"where": expression}),
                        ));
                        rp.rationale.push(format!(
                            "{} を {expression} に絞り込む",
                            layers[target].name
                        ));
                        rp.used.insert("threshold");
                        if is_reference {
                            ("limited".to_string(), subject)
                        } else {
                            (reference, "limited".to_string())
                        }
                    }
                    None => (reference, subject),
                }
            }
            None => (reference, subject),
        };
        // 「…以内にない」: keep what is NOT within the distance.
        let invert = any(
            &text,
            &[
                "以内にない",
                "以内に無い",
                "以内にはない",
                "以内にない",
                "以内には無い",
            ],
        );
        if invert {
            rp.used.insert("negation");
        }
        // 「栄駅から…」: restrict the reference layer to the named feature.
        let reference_name = layers
            .get(&reference)
            .map(|l| l.name.clone())
            .unwrap_or_else(|| "条件に合う地物".into());
        let reference = apply_named(&mut rp, &text, layers, &reference, "named");
        let subject_name = layers
            .get(&subject)
            .map(|l| l.name.clone())
            .unwrap_or_else(|| "条件に合う地物".into());
        if population && layers.get(&subject).and_then(population_field).is_some() {
            let field = population_field(&layers[&subject]).expect("checked");
            rp.steps.push(step(
                "area",
                "buffer",
                &[("layer", &reference)],
                json!({"distance": format!("{metres} m"), "dissolve": !per_polygon}),
            ));
            rp.steps.push(step(
                "result",
                "spatial_join",
                &[("target", "area"), ("join", &subject)],
                json!({"aggregates": [{"op": "area_weighted_sum", "field": field, "as": "population", "unit": "persons"}]}),
            ));
            rp.rationale.push(format!(
                "{} から {metres} m の範囲の人口を {} から面積按分",
                reference_name, subject_name
            ));
            rp.assumptions
                .push("人口は各ポリゴン内に一様に分布すると仮定（面積按分）".into());
        } else if per_polygon && counting {
            rp.steps.push(step(
                "result",
                "spatial_join",
                &[("target", &reference), ("join", &subject)],
                json!({"predicate": "within_distance", "distance": format!("{metres} m"), "aggregates": [{"op": "count", "as": "count_within"}]}),
            ));
            rp.rationale.push(format!(
                "{} ごとに {metres} m 以内の {} を数える",
                reference_name, subject_name
            ));
        } else {
            rp.steps.push(step(
                "selected",
                "select_by_location",
                &[("layer", &subject), ("other", &reference)],
                json!({"predicate": "within_distance", "distance": format!("{metres} m"), "invert": invert}),
            ));
            rp.rationale.push(format!(
                "{} のうち {} から {metres} m 以内{}のものを選択",
                subject_name,
                reference_name,
                if invert { "にない" } else { "" }
            ));
            if counting {
                rp.steps.push(step(
                    "result",
                    "summarize",
                    &[("layer", "selected")],
                    json!({"aggregates": [{"op": "count"}]}),
                ));
                rp.rationale.push("選択結果の件数を集計".into());
            } else {
                rp.steps.last_mut().expect("pushed").id = "result".into();
            }
        }
        return finish(prompt, rp, layers);
    }

    // 3. Nearest distance: "各避難所から最寄り駅までの距離"
    if nearest && found.len() >= 2 {
        let (layer, target) = (&found[0].1, &found[1].1);
        rp.steps.push(step(
            "result",
            "distance_to_nearest",
            &[("layer", layer), ("target", target)],
            json!({}),
        ));
        rp.rationale.push(format!(
            "{} の各地物から最も近い {} までの距離（m）",
            layers[layer].name, layers[target].name
        ));
        let statistic = [
            ("平均", "mean"),
            ("mean", "mean"),
            ("average", "mean"),
            ("最大", "max"),
            ("最も遠い", "max"),
            ("最小", "min"),
        ]
        .into_iter()
        .find(|(word, _)| text.contains(word))
        .map(|(_, op)| op);
        if let Some(op) = statistic {
            rp.used.insert("statistic");
            rp.steps.last_mut().expect("pushed").id = "nearest".into();
            rp.steps.push(step(
                "result",
                "summarize",
                &[("layer", "nearest")],
                json!({"aggregates": [{"op": op, "field": "nearest_distance_m", "as": format!("{op}_nearest_distance_m"), "unit": "m"}]}),
            ));
            rp.rationale.push(format!(
                "距離の{}を集計",
                match op {
                    "mean" => "平均",
                    "max" => "最大",
                    _ => "最小",
                }
            ));
        }
        return finish(prompt, rp, layers);
    }

    // 4. Points in polygons: "区ごとの避難所の数" / "各区の店舗数". A total
    //    inside an area ("浸水想定区域内の避難所を数えて") is rule 5 instead.
    let inside = any(
        &text,
        &[
            "内の",
            "内にある",
            "の中の",
            "にある",
            "inside",
            "範囲内",
            "外の",
            "外にある",
            "outside",
        ],
    );
    if counting && found.len() >= 2 && (per_polygon || !inside) {
        let polygon = found
            .iter()
            .map(|(_, id)| id)
            .find(|id| kind(id) == GeometryKind::Polygon)
            .cloned();
        let other = found
            .iter()
            .map(|(_, id)| id)
            .find(|id| Some(*id) != polygon.as_ref())
            .cloned();
        if let (Some(polygon), Some(other)) = (polygon, other) {
            let mut aggregates = vec![json!({"op": "count", "as": "count"})];
            if let Some(field) = numeric_field_mentioned(&text, &layers[&other]) {
                aggregates.push(json!({"op": "sum", "field": field}));
                rp.used.insert("sum");
            }
            rp.steps.push(step(
                "result",
                "spatial_join",
                &[("target", &polygon), ("join", &other)],
                json!({"predicate": "intersects", "aggregates": aggregates}),
            ));
            rp.rationale.push(format!(
                "{} ごとに含まれる {} を数える",
                layers[&polygon].name, layers[&other].name
            ));
            return finish(prompt, rp, layers);
        }
    }

    // 5. Clip: "避難所を浸水想定区域で切り抜いて" / "浸水想定区域内の避難所"
    if (clip_words || inside) && found.len() >= 2 {
        let mask = found
            .iter()
            .map(|(_, id)| id)
            .find(|id| kind(id) == GeometryKind::Polygon)
            .cloned();
        let layer = found
            .iter()
            .map(|(_, id)| id)
            .find(|id| Some(*id) != mask.as_ref())
            .cloned();
        if let (Some(mask), Some(layer)) = (mask, layer) {
            let mask = apply_named(&mut rp, &text, layers, &mask, "named_area");
            let mask_name = layers
                .get(&mask)
                .map(|l| l.name.clone())
                .unwrap_or_else(|| "指定した範囲".into());
            // 「区域の外」 means everything the mask does not cover.
            let outside = any(&text, &["外", "以外", "outside"]);
            if outside && any(&text, &["以外"]) {
                rp.used.insert("negation");
            }
            let op = if outside { "erase" } else { "clip" };
            rp.steps.push(step(
                "result",
                op,
                &[("layer", &layer), ("mask", &mask)],
                json!({}),
            ));
            rp.rationale.push(format!(
                "{} を {} の範囲{}",
                layers[&layer].name,
                mask_name,
                if outside {
                    "から除く（外側だけ残す）"
                } else {
                    "で切り抜く"
                }
            ));
            let sum_field = any(&text, &["合計", "総", "sum", "total"])
                .then(|| numeric_field_mentioned(&text, &layers[&layer]))
                .flatten();
            if sum_field.is_some() {
                rp.used.insert("sum");
            }
            if counting || sum_field.is_some() {
                rp.steps.last_mut().expect("pushed").id = "clipped".into();
                let aggregate = match &sum_field {
                    Some(field) => json!({"op": "sum", "field": field}),
                    None => json!({"op": "count"}),
                };
                rp.steps.push(step(
                    "result",
                    "summarize",
                    &[("layer", "clipped")],
                    json!({"aggregates": [aggregate]}),
                ));
            }
            return finish(prompt, rp, layers);
        }
    }

    let single = found
        .first()
        .map(|(_, id)| id.clone())
        .or_else(|| {
            context
                .selected_layer
                .clone()
                .filter(|id| layers.contains_key(id))
        })
        .or_else(|| {
            (layers.len() == 1)
                .then(|| layers.keys().next().cloned())
                .flatten()
        });

    // 6a. Attribute threshold: 「人口が15万人以上の区はいくつ？」
    if let (Some((op, value)), Some(id)) = (threshold(&text), single.clone()) {
        let field = numeric_field_mentioned(&text, &layers[&id])
            .or_else(|| population.then(|| population_field(&layers[&id])).flatten());
        if let Some(field) = field {
            let expression = format!("\"{field}\" {op} {value}");
            rp.used.insert("threshold");
            rp.steps.push(step(
                "result",
                "filter",
                &[("layer", &id)],
                json!({"where": expression}),
            ));
            rp.rationale
                .push(format!("{} を {expression} で絞り込む", layers[&id].name));
            if counting {
                rp.steps.last_mut().expect("pushed").id = "filtered".into();
                rp.steps.push(step(
                    "result",
                    "summarize",
                    &[("layer", "filtered")],
                    json!({"aggregates": [{"op": "count"}]}),
                ));
            }
            return finish(prompt, rp, layers);
        }
    }

    // 6. Density: "人口密度を表示" for any polygon layer with a population field.
    if density {
        let source = single
            .clone()
            .filter(|id| {
                kind(id) == GeometryKind::Polygon && population_field(&layers[id]).is_some()
            })
            .map(|id| {
                let field = population_field(&layers[&id]).expect("filtered");
                (id, field)
            })
            .or_else(population_source)
            .ok_or_else(|| {
                ToolkitError::Unresolved("人口フィールドを持つポリゴンレイヤがありません".into())
            })?;
        let (id, field) = source;
        rp.steps.push(step(
            "area",
            "measure",
            &[("layer", &id)],
            json!({"metrics": ["area"], "area_unit": "km2"}),
        ));
        rp.steps.push(step(
            "result",
            "calculate",
            &[("layer", "area")],
            json!({"field": "density", "expression": format!("\"{field}\" / area_km2"), "unit": "persons/km²"}),
        ));
        rp.rationale.push(format!(
            "{} の {} を測地線面積 (km²) で割って人口密度を算出",
            layers[&id].name, field
        ));
        return finish(prompt, rp, layers);
    }

    if let Some(id) = single {
        // 7. Buffer: "避難所を500mバッファ"
        if let Some((metres, _)) = &distance {
            rp.steps.push(step("result", "buffer", &[("layer", &id)], json!({"distance": format!("{metres} m"), "dissolve": any(&text, &["まとめ", "結合", "dissolve", "merge"])})));
            rp.rationale
                .push(format!("{} の周囲 {metres} m のバッファ", layers[&id].name));
            return finish(prompt, rp, layers);
        }
        // 8. Reproject: "EPSG:6675 に変換"
        if let Some(target) = reproject.filter(|t| t.len() > 5) {
            rp.steps.push(step(
                "result",
                "reproject",
                &[("layer", &id)],
                json!({"crs": target.to_uppercase()}),
            ));
            rp.rationale.push(format!(
                "{} を {} に座標変換",
                layers[&id].name,
                target.to_uppercase()
            ));
            return finish(prompt, rp, layers);
        }
        // 9. Measure: "面積を計算" / "長さ"
        if area || any(&text, &["長さ", "距離", "延長", "length"]) {
            let area_unit = if any(&text, &["ヘクタール", "ha"]) {
                "ha"
            } else if any(&text, &["平方メートル", "㎡", "m2", "m²"]) {
                "m2"
            } else {
                "km2"
            };
            rp.steps.push(step(
                "result",
                "measure",
                &[("layer", &id)],
                json!({"area_unit": area_unit}),
            ));
            rp.rationale
                .push(format!("{} の測地線面積・長さを計測", layers[&id].name));
            return finish(prompt, rp, layers);
        }
        // 10. Centroid.
        if any(&text, &["重心", "中心点", "centroid", "代表点"]) {
            rp.steps.push(step(
                "result",
                "centroid",
                &[("layer", &id)],
                json!({"inside": any(&text, &["内部", "inside"])}),
            ));
            rp.rationale.push(format!("{} の重心", layers[&id].name));
            return finish(prompt, rp, layers);
        }
        // 11. Dissolve / summarize by a mentioned field.
        if per_polygon
            || any(
                &text,
                &["集計", "合計", "まとめ", "dissolve", "summarize", "total"],
            )
        {
            let by = layers[&id]
                .fields
                .iter()
                .filter(|f| !is_numeric(&layers[&id], &f.name))
                .find(|f| text.contains(&normalize(&f.name)))
                .map(|f| f.name.clone());
            let mut aggregates = vec![json!({"op": "count"})];
            if let Some(field) = numeric_field_mentioned(&text, &layers[&id])
                .or_else(|| population.then(|| population_field(&layers[&id])).flatten())
            {
                aggregates.push(json!({"op": "sum", "field": field}));
                rp.used.insert("sum");
            }
            let op = if kind(&id) == GeometryKind::Polygon
                && any(&text, &["結合", "ディゾルブ", "dissolve", "まとめ"])
            {
                "dissolve"
            } else {
                "summarize"
            };
            let mut params = json!({"aggregates": aggregates});
            if let Some(by) = &by {
                params["by"] = json!(by);
            }
            rp.steps.push(step("result", op, &[("layer", &id)], params));
            rp.rationale.push(format!(
                "{} を{}集計",
                layers[&id].name,
                by.map(|b| format!("{b} ごとに")).unwrap_or_default()
            ));
            return finish(prompt, rp, layers);
        }
    }

    Err(ToolkitError::Unresolved(format!(
        "「{prompt}」を解析できませんでした。例: 「駅から500m以内の避難所を数えて」「この点から1km以内の人口」「区ごとの店舗数」「人口密度を計算」。読み込み済みレイヤ: {}",
        layers.values().filter(|l| is_source(l)).map(|l| l.name.as_str()).collect::<Vec<_>>().join("、")
    )))
}

/// Operations that reject invalid polygon input.
const POLYGON_OPS: &[&str] = &[
    "buffer",
    "clip",
    "erase",
    "intersect",
    "dissolve",
    "spatial_join",
    "select_by_location",
    "distance_to_nearest",
];

/// Insert a `make_valid` step for every source layer with invalid polygons
/// that feeds a polygon operation, and rewire those inputs.
fn repair_invalid_inputs(rp: &mut RulePlan, layers: &BTreeMap<String, Layer>) {
    let mut repaired: BTreeMap<String, String> = BTreeMap::new();
    let mut prefix = Vec::new();
    for step in &rp.steps {
        if !POLYGON_OPS.contains(&step.op.as_str()) {
            continue;
        }
        for reference in step.inputs.values() {
            let Some(layer) = layers.get(reference) else {
                continue;
            };
            let invalid = layer.invalid_feature_ids();
            if invalid.is_empty() || repaired.contains_key(reference) {
                continue;
            }
            let id = format!("valid_{}", repaired.len() + 1);
            prefix.push(step_named(&id, "make_valid", reference));
            rp.rationale.push(format!(
                "{} の不正なポリゴン {} 件を make_valid で修復してから使う",
                layer.name,
                invalid.len()
            ));
            repaired.insert(reference.clone(), id);
        }
    }
    if repaired.is_empty() {
        return;
    }
    for step in &mut rp.steps {
        if POLYGON_OPS.contains(&step.op.as_str()) {
            for reference in step.inputs.values_mut() {
                if let Some(id) = repaired.get(reference) {
                    *reference = id.clone();
                }
            }
        }
    }
    prefix.append(&mut rp.steps);
    rp.steps = prefix;
}

fn step_named(id: &str, op: &str, layer: &str) -> PlanStep {
    step(id, op, &[("layer", layer)], json!({}))
}

fn finish(
    prompt: &str,
    mut rp: RulePlan,
    layers: &BTreeMap<String, Layer>,
) -> Result<PlannerResult> {
    let unused: Vec<&str> = rp
        .signals
        .iter()
        .filter(|(key, _)| !rp.used.contains(key))
        .map(|(_, label)| *label)
        .collect();
    if !unused.is_empty() {
        return Err(ToolkitError::Unresolved(format!(
            "「{prompt}」の{}をルール式プランナーでは扱えないため、違う質問に答えてしまわないよう回答を控えます。LLM プランナーか MCP 経由のエージェントを使ってください",
            unused.join("・")
        )));
    }
    repair_invalid_inputs(&mut rp, layers);
    let plan = ToolkitPlan {
        goal: prompt.trim().to_string(),
        steps: rp.steps,
        output: None,
        assumptions: rp.assumptions,
    };
    validate_plan(&plan, layers)?;
    Ok(PlannerResult {
        plan,
        backend: "rule".into(),
        confidence: rp.confidence,
        rationale: rp.rationale,
        ambiguities: rp.ambiguities,
        rejected_attempts: Vec::new(),
    })
}

// ---------------------------------------------------------------------------
// LLM planner
// ---------------------------------------------------------------------------

/// System prompt describing the catalog and plan format.
pub fn llm_system_prompt() -> String {
    let mut catalog = String::new();
    for spec in ops::catalog() {
        catalog.push_str(&format!(
            "- {} (inputs: {}) — {}\n",
            spec.name,
            if spec.inputs.is_empty() {
                "none".to_string()
            } else {
                spec.inputs.join(", ")
            },
            spec.description
        ));
        for param in &spec.params {
            catalog.push_str(&format!(
                "    · {}{}: {}\n",
                param.name,
                if param.required { " (required)" } else { "" },
                param.description
            ));
        }
    }
    format!(
        r#"You are the GeneGIS spatial analysis planner. Turn the user's request into a small workflow graph using ONLY these operations:

{catalog}
Rules:
- Reference loaded layers by their exact id (lyr_…) and earlier steps by their step id. Never invent layer ids.
- Every distance MUST carry a unit ("500 m", "1.2 km"). Never use degrees.
- Step ids use [A-Za-z0-9_-]. Every step must feed the final output step.
- To estimate population inside an area from polygons with population counts, use spatial_join with area_weighted_sum.
- If the user refers to "this point"/"ここ" and a clicked point is given, create it with make_points.
- If the request cannot be done with these operations and layers, return confidence 0 and explain in ambiguities.

Respond with JSON only:
{{"goal": "...", "steps": [{{"id": "...", "op": "...", "inputs": {{"role": "ref"}}, "params": {{}}}}], "output": "step id", "assumptions": ["..."], "confidence": 0.0, "rationale": ["..."], "ambiguities": ["..."]}}"#
    )
}

/// User message with the prompt, layers, and context.
pub fn llm_user_message(
    prompt: &str,
    layers: &BTreeMap<String, Layer>,
    context: &PlannerContext,
) -> String {
    let layer_list: Vec<Value> = layers
        .iter()
        .map(|(id, layer)| {
            json!({
                "id": id,
                "name": layer.name,
                "geometry": layer.geometry_kind(),
                "crs": layer.crs,
                "features": layer.features.len(),
                "fields": layer.fields.iter().map(|f| json!({"name": f.name, "type": f.field_type, "unit": f.unit})).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({
        "request": prompt,
        "loaded_layers": layer_list,
        "clicked_point_lon_lat": context.point,
        "selected_layer": context.selected_layer,
    })
    .to_string()
}

#[derive(Debug, Deserialize)]
struct LlmPlanPayload {
    goal: String,
    steps: Vec<PlanStep>,
    #[serde(default)]
    output: Option<String>,
    #[serde(default)]
    assumptions: Vec<String>,
    #[serde(default)]
    confidence: f32,
    #[serde(default)]
    rationale: Vec<String>,
    #[serde(default)]
    ambiguities: Vec<String>,
}

/// Parse and validate an LLM response body.
pub fn parse_llm_plan(content: &str, layers: &BTreeMap<String, Layer>) -> Result<PlannerResult> {
    let trimmed = content
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let payload: LlmPlanPayload = serde_json::from_str(trimmed)
        .map_err(|e| ToolkitError::Unresolved(format!("LLM returned invalid JSON: {e}")))?;
    if payload.confidence <= 0.0 {
        return Err(ToolkitError::Unresolved(format!(
            "LLM could not plan the request: {}",
            payload.ambiguities.join("; ")
        )));
    }
    let plan = ToolkitPlan {
        goal: payload.goal,
        steps: payload.steps,
        output: payload.output,
        assumptions: payload.assumptions,
    };
    validate_plan(&plan, layers)?;
    Ok(PlannerResult {
        plan,
        backend: "llm".into(),
        confidence: payload.confidence.clamp(0.0, 1.0),
        rationale: payload.rationale,
        ambiguities: payload.ambiguities,
        rejected_attempts: Vec::new(),
    })
}

fn plan_with_llm(
    prompt: &str,
    layers: &BTreeMap<String, Layer>,
    context: &PlannerContext,
    config: &LlmConfig,
) -> Result<PlannerResult> {
    let api_key = config
        .api_key
        .as_deref()
        .ok_or_else(|| ToolkitError::Unresolved("GENEGIS_LLM_API_KEY is not set".into()))?;
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let mut messages = vec![
        json!({"role": "system", "content": llm_system_prompt()}),
        json!({"role": "user", "content": llm_user_message(prompt, layers, context)}),
    ];
    let mut rejected = Vec::new();
    for _attempt in 0..3 {
        let body = json!({
            "model": config.model,
            "temperature": 0.0,
            "response_format": {"type": "json_object"},
            "messages": messages,
        });
        let mut response = ureq::post(&url)
            .header("Authorization", format!("Bearer {api_key}"))
            .send_json(body)
            .map_err(|e| ToolkitError::Provider(format!("LLM transport: {e}")))?;
        let payload: Value = response
            .body_mut()
            .read_json()
            .map_err(|e| ToolkitError::Provider(format!("LLM response: {e}")))?;
        let content = payload
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolkitError::Provider("LLM response has no message content".into()))?
            .to_string();
        match parse_llm_plan(&content, layers) {
            Ok(mut result) => {
                result.rejected_attempts = rejected;
                return Ok(result);
            }
            Err(error @ ToolkitError::Unresolved(_))
                if content.contains("\"confidence\": 0")
                    || content.contains("\"confidence\":0") =>
            {
                return Err(error)
            }
            Err(error) => {
                rejected.push(error.to_string());
                messages.push(json!({"role": "assistant", "content": content}));
                messages.push(json!({"role": "user", "content": format!("The plan was rejected by GeneGIS validation: {error}. Return a corrected plan as JSON.")}));
            }
        }
    }
    Err(ToolkitError::Unresolved(format!(
        "LLM plans were rejected: {}",
        rejected.join(" | ")
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execute::run_plan;
    use crate::import::{import_bytes, ImportOptions};

    fn sample_layers() -> BTreeMap<String, Layer> {
        let base = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/nagoya-population-density/data/"
        );
        let mut layers = BTreeMap::new();
        for (file, name) in [
            ("nagoya-wards.geojson", "区"),
            ("nagoya-shelters.geojson", "避難所"),
            ("nagoya-pois.geojson", "店舗"),
            ("nagoya-flood-zones.geojson", "浸水想定区域"),
        ] {
            let bytes = std::fs::read(format!("{base}{file}")).unwrap();
            let options = ImportOptions {
                name: Some(name.into()),
                ..Default::default()
            };
            let (layer, _) = import_bytes(file, &bytes, &options).unwrap();
            layers.insert(layer.id(), layer);
        }
        // A small station layer.
        let csv =
            "駅名,lon,lat\n名古屋,136.8815,35.1709\n栄,136.9086,35.1681\n金山,136.9006,35.1430\n";
        let (mut stations, _) =
            import_bytes("駅.csv", csv.as_bytes(), &ImportOptions::default()).unwrap();
        stations.name = "駅".into();
        layers.insert(stations.id(), stations);
        layers
    }

    fn plan_and_run(
        prompt: &str,
        context: &PlannerContext,
    ) -> (PlannerResult, crate::execute::ToolkitRun) {
        let layers = sample_layers();
        let result =
            plan_with_rules(prompt, &layers, context).unwrap_or_else(|e| panic!("{prompt}: {e}"));
        let run = run_plan(&result.plan, &layers).unwrap_or_else(|e| panic!("{prompt}: {e}"));
        (result, run)
    }

    #[test]
    fn counts_shelters_near_stations() {
        let (result, run) =
            plan_and_run("駅から500m以内の避難所を数えて", &PlannerContext::default());
        assert_eq!(result.plan.steps[0].op, "select_by_location");
        assert_eq!(result.plan.steps[1].op, "summarize");
        let count = run.output.features[0].properties["count"].as_i64().unwrap();
        assert!(count >= 0);
    }

    #[test]
    fn estimates_population_around_a_clicked_point() {
        let context = PlannerContext {
            point: Some([136.9066, 35.1815]),
            selected_layer: None,
        };
        let (result, run) = plan_and_run("この点から1km以内の人口は？", &context);
        assert_eq!(
            result
                .plan
                .steps
                .iter()
                .map(|s| s.op.as_str())
                .collect::<Vec<_>>(),
            vec!["make_points", "buffer", "spatial_join"]
        );
        let population = run.output.features[0].properties["population"]
            .as_f64()
            .unwrap();
        // 3.14 km² of central Nagoya at several thousand persons/km².
        assert!(
            (5_000.0..80_000.0).contains(&population),
            "population {population}"
        );
        assert_eq!(
            run.output.field("population").unwrap().unit.as_deref(),
            Some("persons")
        );
    }

    #[test]
    fn counts_points_per_ward_and_computes_density() {
        let (result, run) = plan_and_run("区ごとの避難所の数", &PlannerContext::default());
        assert_eq!(result.plan.steps[0].op, "spatial_join");
        let total: i64 = run
            .output
            .features
            .iter()
            .map(|f| f.properties["count"].as_i64().unwrap())
            .sum();
        // 29 of the 32 fixture shelters lie inside a ward polygon; shelter-017,
        // -018, and -023 fall just outside the N03 boundaries (confirmed with
        // an independent ray-cast count over the source GeoJSON).
        assert_eq!(total, 29);

        let (result, run) = plan_and_run("区の人口密度を計算", &PlannerContext::default());
        assert_eq!(result.plan.steps.last().unwrap().op, "calculate");
        assert_eq!(
            run.output.field("density").unwrap().unit.as_deref(),
            Some("persons/km²")
        );
        assert_eq!(run.output.features.len(), 16);
    }

    #[test]
    fn matches_descriptive_layer_names() {
        let mut layers = sample_layers();
        for layer in layers.values_mut() {
            layer.name = match layer.name.as_str() {
                "区" => "名古屋市 区界と人口（令和2年国勢調査）".into(),
                "避難所" => "避難所（サンプル）".into(),
                "店舗" => "店舗・施設（サンプル）".into(),
                "浸水想定区域" => "浸水想定区域（サンプル）".into(),
                "駅" => "主要駅（サンプル）".into(),
                other => other.into(),
            };
        }
        let result =
            plan_with_rules("区ごとの避難所の数", &layers, &PlannerContext::default()).unwrap();
        let target = &result.plan.steps[0].inputs["target"];
        assert!(
            layers[target].name.starts_with("名古屋市 区界"),
            "{}",
            layers[target].name
        );
        let result = plan_with_rules(
            "浸水想定区域内の避難所を数えて",
            &layers,
            &PlannerContext::default(),
        )
        .unwrap();
        // The sample flood zones contain self-intersecting rings, so the
        // planner repairs them first and clips with the repaired layer.
        assert_eq!(result.plan.steps[0].op, "make_valid");
        assert!(layers[&result.plan.steps[0].inputs["layer"]]
            .name
            .starts_with("浸水"));
        assert_eq!(result.plan.steps[1].op, "clip");
        assert_eq!(result.plan.steps[1].inputs["mask"], result.plan.steps[0].id);
        let result =
            plan_with_rules("駅から500m以内の店舗", &layers, &PlannerContext::default()).unwrap();
        assert!(layers[&result.plan.steps[0].inputs["other"]]
            .name
            .starts_with("主要駅"));
    }

    #[test]
    fn stored_results_do_not_hijack_later_questions() {
        let mut layers = sample_layers();
        let first = plan_with_rules("区ごとの店舗数", &layers, &PlannerContext::default()).unwrap();
        let mut output = run_plan(&first.plan, &layers).unwrap().output;
        output.name = "区ごとの店舗数".into();
        layers.insert(output.id(), output);
        let result = plan_with_rules(
            "駅から徒歩10分以内の店舗",
            &layers,
            &PlannerContext::default(),
        )
        .unwrap();
        let chosen = &layers[&result.plan.steps[0].inputs["layer"]];
        assert_eq!(chosen.name, "店舗", "picked {}", chosen.name);
    }

    #[test]
    fn declines_conditions_it_cannot_apply() {
        let layers = sample_layers();
        let context = PlannerContext::default();
        // Unsupported comparison over time.
        let error = plan_with_rules("5年前と比べて人口は増えた？", &layers, &context).unwrap_err();
        assert!(matches!(error, ToolkitError::Unresolved(_)));
        // A threshold on an aggregated count is not a rule the planner knows.
        let error =
            plan_with_rules("店舗が10件以上ある区の人口の合計は？", &layers, &context).unwrap_err();
        assert!(error.to_string().contains("回答を控えます"), "{error}");
    }

    #[test]
    fn applies_every_named_feature_or_declines() {
        let layers = sample_layers();
        let context = PlannerContext::default();
        let result = plan_with_rules(
            "栄駅か名古屋駅から1km以内にある避難所の数",
            &layers,
            &context,
        )
        .unwrap();
        let filter = result.plan.steps.iter().find(|s| s.op == "filter").unwrap();
        let expression = filter.params["where"].as_str().unwrap();
        assert!(
            expression.contains("IN")
                && expression.contains("'栄'")
                && expression.contains("'名古屋'"),
            "{expression}"
        );
        let result = plan_with_rules("千種区にある店舗の数は？", &layers, &context).unwrap();
        assert!(result.plan.steps.iter().any(|s| s.params["where"]
            .as_str()
            .is_some_and(|w| w.contains("千種区"))));
        // A named feature the matched rule cannot use makes it decline.
        assert!(plan_with_rules("千種区の人口密度", &layers, &context).is_err());
    }

    #[test]
    fn applies_negation_and_reference_thresholds() {
        let layers = sample_layers();
        let context = PlannerContext::default();
        let result =
            plan_with_rules("避難所から1km以内にない店舗はいくつ？", &layers, &context).unwrap();
        let select = result
            .plan
            .steps
            .iter()
            .find(|s| s.op == "select_by_location")
            .unwrap();
        assert_eq!(select.params["invert"], json!(true));
        let result = plan_with_rules(
            "収容人数が3000人以上の避難所から500m以内にある店舗の数",
            &layers,
            &context,
        )
        .unwrap();
        assert_eq!(result.plan.steps[0].op, "filter");
        assert!(result.plan.steps[0].params["where"]
            .as_str()
            .unwrap()
            .contains(">= 3000"));
    }

    #[test]
    fn walking_minutes_become_metres_with_an_assumption() {
        let (result, _) = plan_and_run("駅から徒歩10分以内の店舗", &PlannerContext::default());
        assert!(result.plan.assumptions.iter().any(|a| a.contains("800")));
    }

    #[test]
    fn clicked_point_is_required_for_here() {
        let layers = sample_layers();
        let error = plan_with_rules("ここから1km以内の人口", &layers, &PlannerContext::default())
            .unwrap_err();
        assert!(error.to_string().contains("クリック"));
    }

    #[test]
    fn llm_plans_are_validated_not_trusted() {
        let layers = sample_layers();
        let bad = r#"{"goal":"x","steps":[{"id":"a","op":"buffer","inputs":{"layer":"lyr_invented"},"params":{"distance":"1 km"}}],"confidence":0.9}"#;
        assert!(parse_llm_plan(bad, &layers).is_err());
        let shelters = layers.values().find(|l| l.name == "避難所").unwrap().id();
        let good = format!(
            r#"```json
{{"goal":"x","steps":[{{"id":"a","op":"buffer","inputs":{{"layer":"{shelters}"}},"params":{{"distance":"1 km"}}}}],"confidence":0.9}}
```"#
        );
        let result = parse_llm_plan(&good, &layers).unwrap();
        assert_eq!(result.backend, "llm");
        let refusal = r#"{"goal":"x","steps":[],"confidence":0,"ambiguities":["no roads layer"]}"#;
        assert!(parse_llm_plan(refusal, &layers).is_err());
        assert!(llm_system_prompt().contains("area_weighted_sum"));
    }
}
