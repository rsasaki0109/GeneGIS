use serde::{Deserialize, Serialize};

/// Signals extracted from natural language input.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IntentSignals {
    pub place: Option<String>,
    pub metric: Option<String>,
    pub visualization: Option<String>,
    pub matched_tokens: Vec<String>,
}

/// Parsed user intent before workflow binding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedIntent {
    pub raw_prompt: String,
    pub normalized_prompt: String,
    pub signals: IntentSignals,
    pub confidence: f32,
}

impl ParsedIntent {
    pub fn parse(prompt: &str) -> Self {
        let raw = prompt.trim().to_string();
        let normalized = normalize(&raw);
        let mut signals = IntentSignals::default();
        let mut score = 0.0f32;

        if contains_any(&normalized, &["名古屋", "nagoya"]) {
            signals.place = Some("名古屋市".into());
            signals.matched_tokens.push("place:nagoya".into());
            score += 0.45;
        }

        if contains_any(&normalized, &["人口密度", "人口", "density", "population density"]) {
            signals.metric = Some("population_density".into());
            signals.matched_tokens.push("metric:population_density".into());
            score += 0.35;
        }

        if contains_any(&normalized, &["cog", "geotiff", "ラスタ", "raster"])
            && contains_any(&normalized, &["リモート", "remote", "デモ", "demo", "http"])
        {
            signals.metric = Some("remote_cog".into());
            signals.matched_tokens.push("metric:remote_cog".into());
            score += 0.40;
        }

        if contains_any(&normalized, &["cog", "geotiff", "ラスタ", "raster"])
            && contains_any(&normalized, &["ローカル", "local", "同梱", "bundled"])
        {
            signals.metric = Some("local_cog".into());
            signals.matched_tokens.push("metric:local_cog".into());
            score += 0.40;
        }

        if contains_any(
            &normalized,
            &["表示", "見せ", "地図", "map", "choropleth", "コロプレス"],
        ) {
            signals.visualization = Some("choropleth".into());
            signals.matched_tokens.push("viz:choropleth".into());
            score += 0.20;
        }

        Self {
            raw_prompt: raw,
            normalized_prompt: normalized,
            signals,
            confidence: score.min(1.0),
        }
    }
}

fn normalize(text: &str) -> String {
    text.to_lowercase()
        .replace(['　', '\t', '\n', '\r'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_north_star_prompt() {
        let intent = ParsedIntent::parse("名古屋市の人口密度を表示");
        assert_eq!(intent.signals.place.as_deref(), Some("名古屋市"));
        assert_eq!(intent.signals.metric.as_deref(), Some("population_density"));
        assert!(intent.confidence >= 0.8);
    }

    #[test]
    fn parses_local_cog_demo_prompt() {
        let intent = ParsedIntent::parse("ローカルCOGデモのメタデータを表示");
        assert_eq!(intent.signals.metric.as_deref(), Some("local_cog"));
        assert!(intent.confidence >= 0.5);
    }
}
