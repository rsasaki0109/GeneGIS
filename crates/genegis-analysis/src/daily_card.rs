//! Square "map of the day" card (e.g. for #30DayMapChallenge).
//!
//! Wraps the final frame of one of the verified showcase renderers in a
//! 1080×1080 card with the day, the theme, the prompt, the verification
//! state and the result digest, so a daily post is one command and every
//! post carries its own evidence. The card refuses to render unless every
//! check of the source passed, and it states when the data is synthetic.

use base64::{engine::general_purpose::STANDARD, Engine as _};

use crate::nagoya_density3d::FONT;
use crate::showcase::{escape_xml, rasterize_svg};
use crate::AnalysisError;

/// Final frame of a verified renderer plus the evidence the card prints.
pub(crate) struct CardSource {
    pub png: Vec<u8>,
    pub prompt: &'static str,
    pub checks_passed: usize,
    pub checks_total: usize,
    pub result_digest: String,
    pub synthetic: bool,
}

/// Which verified renderer supplies the map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DailyMap {
    /// 3D population-mesh columns (`demo frames-nagoya3d`).
    Density3d,
    /// Walk vs walk + rail reach race (`demo frames-isochrone`).
    Isochrone,
    /// Unverified vs verified ward density (`demo frames-contrast`).
    Contrast,
}

impl DailyMap {
    pub fn parse(value: &str) -> Result<Self, AnalysisError> {
        match value {
            "density3d" => Ok(Self::Density3d),
            "isochrone" => Ok(Self::Isochrone),
            "contrast" => Ok(Self::Contrast),
            other => Err(AnalysisError::Message(format!(
                "unknown map {other}; use density3d, isochrone or contrast"
            ))),
        }
    }

    fn source(self) -> Result<CardSource, AnalysisError> {
        match self {
            Self::Density3d => crate::nagoya_density3d::card_source(),
            Self::Isochrone => crate::isochrone_race::card_source(),
            Self::Contrast => crate::verification_contrast::card_source(),
        }
    }
}

const SIZE: f64 = 1080.0;
const MARGIN: f64 = 36.0;
const MAP_Y: f64 = 196.0;
/// Source frames are 1200×675 (16:9).
const MAP_H: f64 = (SIZE - 2.0 * MARGIN) * 675.0 / 1200.0;

/// Render the 1080×1080 card for `day` (1–30) and the day's `theme`.
pub fn render_daily_card(day: u32, theme: &str, map: DailyMap) -> Result<Vec<u8>, AnalysisError> {
    if !(1..=30).contains(&day) {
        return Err(AnalysisError::Message(format!(
            "day must be 1-30, got {day}"
        )));
    }
    if theme.trim().is_empty() {
        return Err(AnalysisError::Message("theme must not be empty".into()));
    }
    let source = map.source()?;
    if source.checks_total == 0 || source.checks_passed != source.checks_total {
        return Err(AnalysisError::Message(format!(
            "daily card refused (fail-closed): {}/{} checks passed",
            source.checks_passed, source.checks_total
        )));
    }
    let digest = source.result_digest.trim_start_matches("sha256:");
    let data_line = if source.synthetic {
        "データ: 合成fixture（実観測ではない）· 同じ検証で実データに差し替え可能"
    } else {
        "データ: 名古屋市 令和2年国勢調査 / 国土数値情報 N03"
    };
    let (tag_fill, tag_stroke, tag) = if source.synthetic {
        ("#3b2a12", "#f59e0b", "SYNTHETIC DATA")
    } else {
        ("#0c2f3f", "#38bdf8", "OPEN DATA")
    };
    let map_w = SIZE - 2.0 * MARGIN;
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{SIZE}" height="{SIZE}" viewBox="0 0 {SIZE} {SIZE}">
<rect width="100%" height="100%" fill="#050a16"/>
<text x="{MARGIN}" y="64" font-family="Inter, DejaVu Sans, sans-serif" font-size="24" font-weight="bold" fill="#38bdf8">#30DayMapChallenge</text>
<text x="{MARGIN}" y="146" font-family="Inter, DejaVu Sans, sans-serif" font-size="76" font-weight="bold" fill="#f8fafc">Day {day:02}</text>
<text x="{theme_x}" y="146" font-family="Inter, {FONT}" font-size="44" font-weight="bold" fill="#fcd34d">{theme}</text>
<rect x="{MARGIN}" y="{MAP_Y}" width="{map_w}" height="{MAP_H:.1}" rx="16" fill="#08101f"/>
<image x="{MARGIN}" y="{MAP_Y}" width="{map_w}" height="{MAP_H:.1}" preserveAspectRatio="xMidYMid meet" xlink:href="data:image/png;base64,{png}"/>
<rect x="{MARGIN}" y="{prompt_y:.0}" width="{map_w}" height="64" rx="32" fill="#0f172a" stroke="#334155"/>
<circle cx="{icon_x}" cy="{icon_y:.0}" r="11" fill="none" stroke="#38bdf8" stroke-width="2.8"/>
<line x1="{icon_x2}" y1="{icon_y2:.0}" x2="{icon_x3}" y2="{icon_y3:.0}" stroke="#38bdf8" stroke-width="2.8" stroke-linecap="round"/>
<text x="{prompt_tx}" y="{prompt_ty:.0}" font-family="{FONT}" font-size="28" font-weight="bold" fill="#f1f5f9">{prompt}</text>
<rect x="{MARGIN}" y="{badge_y:.0}" width="250" height="52" rx="26" fill="#0f5132" stroke="#34d399" stroke-width="2"/>
<text x="{badge_tx}" y="{badge_ty:.0}" font-family="{FONT}" font-size="22" font-weight="bold" fill="#ecfdf5" text-anchor="middle">✓ 検証済み {passed}/{total}</text>
<rect x="{tag_x}" y="{badge_y:.0}" width="190" height="52" rx="26" fill="{tag_fill}" stroke="{tag_stroke}" stroke-width="2"/>
<text x="{tag_tx}" y="{badge_ty:.0}" font-family="DejaVu Sans, sans-serif" font-size="16" font-weight="bold" fill="{tag_stroke}" text-anchor="middle">{tag}</text>
<text x="{digest_x}" y="{digest_y:.0}" font-family="DejaVu Sans Mono, monospace" font-size="15" fill="#64748b">result sha256:{digest_short}…</text>
<text x="{MARGIN}" y="{data_y:.0}" font-family="{FONT}" font-size="17" fill="#94a3b8">{data_line}</text>
<text x="{MARGIN}" y="1050" font-family="Inter, DejaVu Sans, sans-serif" font-size="19" fill="#cbd5e1">GeneGIS — one prompt, a verified map · github.com/rsasaki0109/GeneGIS</text>
</svg>"##,
        theme_x = MARGIN + 290.0,
        theme = escape_xml(theme.trim()),
        png = STANDARD.encode(&source.png),
        prompt_y = MAP_Y + MAP_H + 28.0,
        icon_x = MARGIN + 36.0,
        icon_y = MAP_Y + MAP_H + 58.0,
        icon_x2 = MARGIN + 44.0,
        icon_y2 = MAP_Y + MAP_H + 66.0,
        icon_x3 = MARGIN + 52.0,
        icon_y3 = MAP_Y + MAP_H + 74.0,
        prompt_tx = MARGIN + 70.0,
        prompt_ty = MAP_Y + MAP_H + 70.0,
        prompt = escape_xml(source.prompt),
        badge_y = MAP_Y + MAP_H + 112.0,
        badge_tx = MARGIN + 125.0,
        badge_ty = MAP_Y + MAP_H + 145.0,
        passed = source.checks_passed,
        total = source.checks_total,
        tag_x = MARGIN + 266.0,
        tag_tx = MARGIN + 361.0,
        digest_x = MARGIN + 476.0,
        digest_y = MAP_Y + MAP_H + 144.0,
        digest_short = &digest[..digest.len().min(16)],
        data_y = MAP_Y + MAP_H + 196.0,
    );
    rasterize_svg(&svg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_out_of_range_day_and_blank_theme() {
        assert!(render_daily_card(0, "Points", DailyMap::Contrast).is_err());
        assert!(render_daily_card(31, "Points", DailyMap::Contrast).is_err());
        assert!(render_daily_card(1, "  ", DailyMap::Contrast).is_err());
        assert!(DailyMap::parse("bogus").is_err());
    }

    #[test]
    fn renders_a_square_card_for_every_source() {
        for map in [DailyMap::Density3d, DailyMap::Isochrone, DailyMap::Contrast] {
            let png = render_daily_card(5, "Lines", map).expect("card");
            // PNG IHDR width and height: 1080 × 1080.
            assert_eq!(&png[16..24], &[0, 0, 4, 56, 0, 0, 4, 56], "{map:?}");
        }
    }
}
