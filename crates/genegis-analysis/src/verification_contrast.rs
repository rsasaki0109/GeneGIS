//! "Same prompt, same data — the difference is verification" showcase.
//!
//! Reproduces a common generated-code mistake for 「名古屋市の人口密度を表示」:
//! measuring ward area after reprojecting to Web Mercator
//! (`gdf.to_crs(3857).area / 1e6`). The map looks plausible, but Mercator
//! inflates area by roughly 1/cos²(lat), so every density is about a third
//! too low. The candidate is run through the exact release verifier of the
//! Nagoya workflow (`verify_nagoya_density_features`): the format checks pass,
//! the independent area and density oracles reject it, and the verified
//! workflow result is shown instead.
//!
//! The frames are built only when the verified result passes every check and
//! the Mercator candidate is actually rejected, so the GIF cannot claim a
//! catch the verifier did not make.

use genegis_crs::Crs;
use genegis_geometry::PolygonRing;

use crate::nagoya_density3d::{hex, ramp, thousands, FONT};
use crate::result::{AnalysisResult, DensityFeature, VerificationReport};
use crate::showcase::{escape_xml, rasterize_svg};
use crate::AnalysisError;
use crate::{run_nagoya_population_density_from_catalog, verify_nagoya_density_features};

pub struct ContrastFrame {
    pub name: String,
    pub png: Vec<u8>,
}

const PROMPT: &str = "名古屋市の人口密度を表示";
const NAIVE_CODE: &str = "area_km2 = gdf.to_crs(3857).area / 1e6";
const MONO: &str = "DejaVu Sans Mono, monospace";

const FRAME_W: f64 = 1200.0;
const FRAME_H: f64 = 675.0;
const LEFT_X: f64 = 24.0;
const RIGHT_X: f64 = 612.0;
const PANEL_W: f64 = 564.0;

const REVEAL_FRAMES: usize = 13;
const REJECT_FRAMES: usize = 10;
const VERIFIED_FRAMES: usize = 14;
const FRAME_COUNT: usize = REVEAL_FRAMES + REJECT_FRAMES + VERIFIED_FRAMES;

const WEB_MERCATOR_RADIUS_M: f64 = 6_378_137.0;

struct Contrast {
    verified: AnalysisResult,
    naive: Vec<DensityFeature>,
    naive_report: VerificationReport,
    bounds: [f64; 4],
    min_density: f64,
    max_density: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    Reveal(usize),
    Rejected,
    Verified,
}

fn mercator_ring_area_m2(coords: &[(f64, f64)]) -> f64 {
    let projected: Vec<(f64, f64)> = coords
        .iter()
        .map(|&(lon, lat)| {
            let phi = lat.to_radians();
            (
                WEB_MERCATOR_RADIUS_M * lon.to_radians(),
                WEB_MERCATOR_RADIUS_M * (std::f64::consts::FRAC_PI_4 + phi / 2.0).tan().ln(),
            )
        })
        .collect();
    let mut twice = 0.0;
    for i in 0..projected.len() {
        let (x1, y1) = projected[i];
        let (x2, y2) = projected[(i + 1) % projected.len()];
        twice += x1 * y2 - x2 * y1;
    }
    twice.abs() / 2.0
}

/// Planar area in EPSG:3857 metres — what `to_crs(3857).area` returns.
fn web_mercator_area_km2(rings: &[PolygonRing]) -> f64 {
    rings
        .iter()
        .map(|ring| {
            mercator_ring_area_m2(&ring.coords)
                - ring
                    .holes
                    .iter()
                    .map(|hole| mercator_ring_area_m2(hole))
                    .sum::<f64>()
        })
        .sum::<f64>()
        / 1e6
}

fn build_contrast() -> Result<Contrast, AnalysisError> {
    let verified = run_nagoya_population_density_from_catalog()?;
    if let Some(failed) = verified.verification.checks.iter().find(|c| !c.passed) {
        return Err(AnalysisError::Message(format!(
            "verified baseline failed {}: {}",
            failed.name, failed.detail
        )));
    }

    let naive: Vec<DensityFeature> = verified
        .features
        .iter()
        .map(|feature| {
            let area_km2 = web_mercator_area_km2(&feature.rings);
            DensityFeature {
                area_km2,
                density_per_km2: feature.population as f64 / area_km2,
                ..feature.clone()
            }
        })
        .collect();
    let mercator =
        Crs::parse("EPSG:3857").map_err(|err| AnalysisError::Message(err.to_string()))?;
    let naive_report =
        verify_nagoya_density_features(&mercator, &naive, verified.verification.source.clone());
    if naive_report.checks.iter().all(|check| check.passed) {
        return Err(AnalysisError::Message(
            "the Web Mercator candidate was not rejected; refusing to claim a catch".into(),
        ));
    }

    let bounds = verified.features.iter().flat_map(|f| &f.rings).fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |b, ring| {
            ring.coords.iter().fold(b, |b, &(x, y)| {
                [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)]
            })
        },
    );
    let densities = verified.features.iter().map(|f| f.density_per_km2);
    let min_density = densities.clone().fold(f64::INFINITY, f64::min);
    let max_density = densities.fold(0.0, f64::max);
    Ok(Contrast {
        verified,
        naive,
        naive_report,
        bounds,
        min_density,
        max_density,
    })
}

fn city_density(features: &[DensityFeature]) -> f64 {
    let population: u64 = features.iter().map(|f| f.population).sum();
    let area: f64 = features.iter().map(|f| f.area_km2).sum();
    population as f64 / area
}

/// Ward choropleth fitted into `(x, y, w, h)`; one colour scale for both sides.
fn choropleth(contrast: &Contrast, features: &[DensityFeature], frame: [f64; 4]) -> String {
    let [x, y, w, h] = frame;
    let b = contrast.bounds;
    let kx = ((b[1] + b[3]) / 2.0).to_radians().cos();
    let scale = (w / ((b[2] - b[0]) * kx)).min(h / (b[3] - b[1]));
    let ox = x + (w - (b[2] - b[0]) * kx * scale) / 2.0;
    let oy = y + (h - (b[3] - b[1]) * scale) / 2.0;
    let to_screen = |&(lon, lat): &(f64, f64)| {
        format!(
            "{:.1},{:.1}",
            ox + (lon - b[0]) * kx * scale,
            oy + (b[3] - lat) * scale
        )
    };
    let span = (contrast.max_density - contrast.min_density).max(f64::EPSILON);
    let mut out = String::new();
    for feature in features {
        let t = ((feature.density_per_km2 - contrast.min_density) / span).clamp(0.0, 1.0);
        let fill = hex(ramp(t), 1.0);
        for ring in &feature.rings {
            let points: Vec<String> = ring.coords.iter().map(to_screen).collect();
            out.push_str(&format!(
                r##"<polygon points="{}" fill="{fill}" stroke="#0b1222" stroke-width="1.2"/>"##,
                points.join(" ")
            ));
        }
    }
    out
}

fn relative(actual: f64, expected: f64) -> f64 {
    (actual - expected) / expected
}

fn signed_percent(value: f64) -> String {
    format!(
        "{}{:.0}%",
        if value >= 0.0 { "+" } else { "−" },
        value.abs() * 100.0
    )
}

fn render_frame(contrast: &Contrast, stage: Stage) -> Result<Vec<u8>, AnalysisError> {
    let naive_city = city_density(&contrast.naive);
    let verified_city = city_density(&contrast.verified.features);
    let naive_area: f64 = contrast.naive.iter().map(|f| f.area_km2).sum();
    let verified_area: f64 = contrast.verified.features.iter().map(|f| f.area_km2).sum();
    let area_error = relative(naive_area, verified_area);
    let density_error = relative(naive_city, verified_city);

    let mut body = String::new();

    // Left: the unverified generated code, run as-is.
    body.push_str(&format!(
        r##"<rect x="{LEFT_X}" y="112" width="{PANEL_W}" height="540" rx="14" fill="#0b1222" stroke="#1e293b"/>
<text x="{lx:.0}" y="142" font-family="{FONT}" font-size="15" font-weight="bold" fill="#e2e8f0">検証なし：生成コードをそのまま実行</text>
<rect x="{lx:.0}" y="156" width="{cw:.0}" height="32" rx="6" fill="#020617" stroke="#334155"/>
<text x="{lx2:.0}" y="177" font-family="{MONO}" font-size="13.5" fill="#fca5a5">{code}</text>
{map}
<text x="{lx:.0}" y="604" font-family="{FONT}" font-size="12" fill="#94a3b8">市全体の人口密度</text>
<text x="{lx:.0}" y="636" font-family="{FONT}" font-size="26" font-weight="bold" fill="#f8fafc">{naive} 人/km²</text>
<text x="{lx3:.0}" y="636" font-family="{FONT}" font-size="12.5" fill="#94a3b8">見た目はもっともらしい</text>"##,
        lx = LEFT_X + 20.0,
        lx2 = LEFT_X + 32.0,
        lx3 = LEFT_X + 262.0,
        cw = PANEL_W - 40.0,
        code = escape_xml(NAIVE_CODE),
        map = choropleth(contrast, &contrast.naive, [LEFT_X + 20.0, 200.0, PANEL_W - 40.0, 376.0]),
        naive = thousands(naive_city.round() as u64),
    ));
    if !matches!(stage, Stage::Reveal(_)) {
        body.push_str(&format!(
            r##"<rect x="{x:.0}" y="330" width="{w:.0}" height="96" rx="12" fill="#450a0a" fill-opacity="0.9" stroke="#f87171" stroke-width="2"/>
<text x="{cx:.0}" y="370" font-family="{FONT}" font-size="22" font-weight="bold" fill="#fecaca" text-anchor="middle">✗ 公式値と不一致</text>
<text x="{cx:.0}" y="404" font-family="{FONT}" font-size="15" fill="#fecaca" text-anchor="middle">面積 {area} ・ 人口密度 {density}（市全体）</text>"##,
            x = LEFT_X + 60.0,
            w = PANEL_W - 120.0,
            cx = LEFT_X + PANEL_W / 2.0,
            area = signed_percent(area_error),
            density = signed_percent(density_error),
        ));
    }

    // Right: GeneGIS runs the same candidate through the release verifier.
    body.push_str(&format!(
        r##"<rect x="{RIGHT_X}" y="112" width="{PANEL_W}" height="540" rx="14" fill="#0b1222" stroke="{stroke}"/>"##,
        stroke = if stage == Stage::Verified { "#34d399" } else { "#1e293b" },
    ));
    let rx = RIGHT_X + 20.0;
    match stage {
        Stage::Reveal(_) | Stage::Rejected => {
            let revealed = match stage {
                Stage::Reveal(n) => n,
                _ => contrast.naive_report.checks.len(),
            };
            body.push_str(&format!(
                r##"<text x="{rx:.0}" y="142" font-family="{FONT}" font-size="15" font-weight="bold" fill="#e2e8f0">GeneGIS：同じ結果をリリース検証器にかける</text>
<text x="{rx:.0}" y="164" font-family="{FONT}" font-size="12" fill="#94a3b8">形式のチェックは通る。独立 oracle（公式の区別面積・人口）が捕まえる</text>"##
            ));
            for (i, check) in contrast.naive_report.checks.iter().enumerate() {
                let y = 194.0 + i as f64 * 25.0;
                if i >= revealed {
                    body.push_str(&format!(
                        r##"<text x="{rx:.0}" y="{y:.0}" font-family="{MONO}" font-size="12.5" fill="#334155">·  {}</text>"##,
                        escape_xml(&check.name)
                    ));
                    continue;
                }
                let (mark, color) = if check.passed {
                    ("✓", "#34d399")
                } else {
                    ("✗", "#f87171")
                };
                body.push_str(&format!(
                    r##"<text x="{rx:.0}" y="{y:.0}" font-family="{FONT}" font-size="13" font-weight="bold" fill="{color}">{mark}</text><text x="{:.0}" y="{y:.0}" font-family="{MONO}" font-size="12.5" fill="{}">{}</text>"##,
                    rx + 20.0,
                    if check.passed { "#cbd5e1" } else { "#fecaca" },
                    escape_xml(&check.name)
                ));
            }
            if stage == Stage::Rejected {
                let failed = contrast
                    .naive_report
                    .checks
                    .iter()
                    .filter(|check| !check.passed)
                    .count();
                body.push_str(&format!(
                    r##"<rect x="{rx:.0}" y="540" width="{w:.0}" height="92" rx="10" fill="#450a0a" stroke="#f87171"/>
<text x="{tx:.0}" y="574" font-family="{FONT}" font-size="20" font-weight="bold" fill="#fecaca">REJECTED · {failed} チェック失敗</text>
<text x="{tx:.0}" y="606" font-family="{FONT}" font-size="13" fill="#fecaca">結果は返さず、計画を差し戻す（Web メルカトルは等積ではない）</text>"##,
                    w = PANEL_W - 40.0,
                    tx = rx + 18.0,
                ));
            }
        }
        Stage::Verified => {
            let checks = &contrast.verified.verification.checks;
            body.push_str(&format!(
                r##"<text x="{rx:.0}" y="142" font-family="{FONT}" font-size="15" font-weight="bold" fill="#e2e8f0">GeneGIS：楕円体面積で再計画 → 検証済み</text>
<rect x="{bx:.0}" y="124" width="132" height="28" rx="14" fill="#0f5132" stroke="#34d399"/>
<text x="{btx:.0}" y="143" font-family="{FONT}" font-size="13.5" font-weight="bold" fill="#ecfdf5" text-anchor="middle">✓ {n}/{n} 検証済み</text>
<text x="{rx:.0}" y="164" font-family="{FONT}" font-size="12" fill="#94a3b8">area: {method} · CRS {crs} · 単位 {unit}</text>
{map}"##,
                bx = RIGHT_X + PANEL_W - 152.0,
                btx = RIGHT_X + PANEL_W - 86.0,
                n = checks.len(),
                method = escape_xml(&contrast.verified.verification.area_method),
                crs = escape_xml(&contrast.verified.verification.crs),
                unit = escape_xml(&contrast.verified.verification.density_unit),
                map = choropleth(
                    contrast,
                    &contrast.verified.features,
                    [rx, 176.0, 250.0, 300.0]
                ),
            ));
            // Per-ward correction table, densest wards first.
            let mut order: Vec<usize> = (0..contrast.verified.features.len()).collect();
            order.sort_by(|&a, &b| {
                contrast.verified.features[b]
                    .density_per_km2
                    .total_cmp(&contrast.verified.features[a].density_per_km2)
            });
            let tx = rx + 270.0;
            body.push_str(&format!(
                r##"<text x="{tx:.0}" y="196" font-family="{FONT}" font-size="11.5" fill="#94a3b8">区</text><text x="{:.0}" y="196" font-family="{FONT}" font-size="11.5" fill="#94a3b8" text-anchor="end">検証なし</text><text x="{:.0}" y="196" font-family="{FONT}" font-size="11.5" fill="#94a3b8" text-anchor="end">GeneGIS</text>"##,
                tx + 150.0,
                tx + 250.0,
            ));
            for (row, &i) in order.iter().take(10).enumerate() {
                let y = 222.0 + row as f64 * 24.0;
                let verified = &contrast.verified.features[i];
                let naive = &contrast.naive[i];
                body.push_str(&format!(
                    r##"<text x="{tx:.0}" y="{y:.0}" font-family="{FONT}" font-size="12.5" fill="#e2e8f0">{}</text><text x="{:.0}" y="{y:.0}" font-family="{MONO}" font-size="12" fill="#fca5a5" text-anchor="end">{}</text><text x="{:.0}" y="{y:.0}" font-family="{MONO}" font-size="12" fill="#6ee7b7" text-anchor="end">{}</text>"##,
                    escape_xml(&verified.ward_name),
                    tx + 150.0,
                    thousands(naive.density_per_km2.round() as u64),
                    tx + 250.0,
                    thousands(verified.density_per_km2.round() as u64),
                ));
            }
            body.push_str(&format!(
                r##"<text x="{rx:.0}" y="604" font-family="{FONT}" font-size="12" fill="#94a3b8">市全体の人口密度</text>
<text x="{rx:.0}" y="636" font-family="{FONT}" font-size="26" font-weight="bold" fill="#6ee7b7">{verified} 人/km²</text>
<text x="{tx2:.0}" y="636" font-family="{FONT}" font-size="12.5" fill="#94a3b8">公式の区別面積・人口と一致（±0.5%）</text>"##,
                verified = thousands(verified_city.round() as u64),
                tx2 = rx + 262.0,
            ));
        }
    }

    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{FRAME_W}" height="{FRAME_H}" viewBox="0 0 {FRAME_W} {FRAME_H}">
<rect width="100%" height="100%" fill="#050a16"/>
<rect x="24" y="22" width="420" height="50" rx="25" fill="#0f172a" stroke="#334155"/>
<circle cx="52" cy="47" r="9" fill="none" stroke="#38bdf8" stroke-width="2.4"/><line x1="58.5" y1="53.5" x2="65" y2="60" stroke="#38bdf8" stroke-width="2.4" stroke-linecap="round"/>
<text x="80" y="55" font-family="{FONT}" font-size="21" font-weight="bold" fill="#f1f5f9">{prompt}</text>
<text x="466" y="46" font-family="{FONT}" font-size="20" font-weight="bold" fill="#f8fafc">同じプロンプト、同じデータ。違いは検証。</text>
<text x="466" y="68" font-family="{FONT}" font-size="12" fill="#94a3b8">よくある生成コードの誤りを再現 · 同じ配色スケール · GeneGIS のリリース検証器をそのまま使用</text>
<text x="24" y="100" font-family="{FONT}" font-size="11" fill="#64748b">データ: 名古屋市 令和2年国勢調査 区別人口 · 境界: 国土数値情報 N03 · 色 = 人口密度 {lo}–{hi} persons/km²（範囲外は端の色）</text>
{body}
</svg>"##,
        prompt = escape_xml(PROMPT),
        lo = thousands(contrast.min_density.round() as u64),
        hi = thousands(contrast.max_density.round() as u64),
    );
    rasterize_svg(&svg)
}

fn stage_for(index: usize) -> Stage {
    if index < REVEAL_FRAMES {
        Stage::Reveal(index + 1)
    } else if index < REVEAL_FRAMES + REJECT_FRAMES {
        Stage::Rejected
    } else {
        Stage::Verified
    }
}

/// Render the unverified-vs-verified contrast sequence.
pub fn render_verification_contrast_frames() -> Result<Vec<ContrastFrame>, AnalysisError> {
    let contrast = build_contrast()?;
    (0..FRAME_COUNT)
        .map(|index| {
            Ok(ContrastFrame {
                name: format!("contrast-{index:02}"),
                png: render_frame(&contrast, stage_for(index))?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_mercator_inflates_area_by_about_one_over_cos_squared() {
        let square = PolygonRing::new(vec![
            (136.90, 35.10),
            (136.91, 35.10),
            (136.91, 35.11),
            (136.90, 35.11),
            (136.90, 35.10),
        ]);
        let ellipsoidal = genegis_geometry::polygon_parts_area_km2_for_crs(
            std::slice::from_ref(&square),
            &Crs::parse("EPSG:4326").expect("crs"),
        )
        .expect("area");
        let inflation = web_mercator_area_km2(&[square]) / ellipsoidal;
        let expected = 1.0 / 35.105_f64.to_radians().cos().powi(2);
        assert!(
            (inflation - expected).abs() < 0.01,
            "{inflation} vs {expected}"
        );
    }

    #[test]
    fn release_verifier_rejects_the_mercator_candidate_on_oracles_only() {
        let contrast = build_contrast().expect("contrast");
        let failed: Vec<&str> = contrast
            .naive_report
            .checks
            .iter()
            .filter(|check| !check.passed)
            .map(|check| check.name.as_str())
            .collect();
        assert_eq!(failed, ["area_oracle_relative_error", "density_oracle"]);
        let low = city_density(&contrast.naive) / city_density(&contrast.verified.features);
        assert!((0.6..0.75).contains(&low), "{low}");
    }

    #[test]
    fn renders_each_stage_as_png() {
        let contrast = build_contrast().expect("contrast");
        for stage in [Stage::Reveal(3), Stage::Rejected, Stage::Verified] {
            let png = render_frame(&contrast, stage).expect("render");
            assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        }
        assert_eq!(stage_for(0), Stage::Reveal(1));
        assert_eq!(stage_for(FRAME_COUNT - 1), Stage::Verified);
    }
}
