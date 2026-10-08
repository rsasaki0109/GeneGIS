//! Walk-only vs walk + rail reach race from Nagoya Station.
//!
//! Two side-by-side maps animate travel time from 0 to 60 minutes: street
//! segments light up as they become reachable, POIs switch on when reached,
//! and on the multimodal side reach jumps along the rail corridors. Both sides
//! use the same Dijkstra engines as the accessibility workflows (`WalkGraph`
//! and `TransitGraph`) over the catalog walk network, transit corridors and
//! POIs, so the env overrides of those workflows apply here too.
//!
//! Fail-closed: the origin snap, the physical speed floors of both modes,
//! multimodal monotonicity (adding rail never makes a node slower or
//! unreachable) and POI snapping are checked before any frame is rendered.

use genegis_network::{TransitGraph, WalkGraph};
use serde_json::Value;

use crate::accessibility::{load_poi_points, load_transit_corridors};
use crate::nagoya_density3d::{hex, ramp, FONT};
use crate::showcase::{escape_xml, rasterize_svg};
use crate::AnalysisError;

pub struct IsochroneRaceFrame {
    pub name: String,
    pub png: Vec<u8>,
}

/// Named check evaluated before any frame is rendered.
#[derive(Debug, Clone)]
pub struct RaceCheck {
    pub name: &'static str,
    pub passed: bool,
    pub detail: String,
}

/// Nagoya Station (the transit fixture's interchange stop).
const ORIGIN: (f64, f64) = (136.8815, 35.1707);
const ORIGIN_SNAP_LIMIT_M: f64 = 500.0;
const MAX_MINUTES: usize = 60;
const STEP_MINUTES: usize = 2;
const HIGHLIGHT_MINUTES: f64 = 45.0;
/// Fastest modelled mode: 30 km/h rail (matches `load_transit_corridors`).
const RIDE_SPEED_M_PER_MIN: f64 = 500.0;
const TRANSFER_PENALTY_MIN: f64 = 3.0;
const EPS_MIN: f64 = 1e-6;

const FRAME_W: f64 = 1200.0;
const FRAME_H: f64 = 675.0;
const PANEL_Y: f64 = 132.0;
const PANEL_W: f64 = 564.0;
const PANEL_H: f64 = 430.0;
const LEFT_X: f64 = 24.0;
const RIGHT_X: f64 = 612.0;

struct Race {
    walk: WalkGraph,
    /// `(from, to, minutes, metres)`, each undirected segment once.
    edges: Vec<(u32, u32, f64, f64)>,
    walk_times: Vec<Option<f64>>,
    transit_times: Vec<Option<f64>>,
    pois: Vec<u32>,
    wards: Vec<Vec<(f64, f64)>>,
    rails: Vec<Vec<(f64, f64)>>,
    checks: Vec<RaceCheck>,
    bounds: [f64; 4],
    synthetic: bool,
}

fn read_json(path: &str) -> Result<Value, AnalysisError> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| AnalysisError::Message(format!("read {path}: {err}")))?;
    serde_json::from_str(&text)
        .map_err(|err| AnalysisError::Message(format!("parse {path}: {err}")))
}

fn is_synthetic(doc: &Value) -> bool {
    doc["description"]
        .as_str()
        .is_some_and(|text| text.to_uppercase().contains("SYNTHETIC"))
}

fn line_strings(doc: &Value, filter: impl Fn(&Value) -> bool) -> Vec<Vec<(f64, f64)>> {
    let mut out = Vec::new();
    for feature in doc["features"].as_array().into_iter().flatten() {
        if !filter(&feature["properties"]) {
            continue;
        }
        let geometry = &feature["geometry"];
        let parts: Vec<&Value> = match geometry["type"].as_str() {
            Some("LineString") => vec![&geometry["coordinates"]],
            Some("Polygon") => geometry["coordinates"]
                .as_array()
                .into_iter()
                .flatten()
                .take(1)
                .collect(),
            Some("MultiPolygon") => geometry["coordinates"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|polygon| polygon.get(0))
                .collect(),
            _ => Vec::new(),
        };
        for part in parts {
            let points: Vec<(f64, f64)> = part
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| Some((c.get(0)?.as_f64()?, c.get(1)?.as_f64()?)))
                .collect();
            if points.len() >= 2 {
                out.push(points);
            }
        }
    }
    out
}

fn build_race() -> Result<Race, AnalysisError> {
    let network_path = genegis_catalog::nagoya_walk_network_path();
    let transit_path = genegis_catalog::nagoya_transit_path();
    let pois_path = genegis_catalog::nagoya_pois_path();

    let walk = WalkGraph::from_geojson_path(network_path)
        .map_err(|err| AnalysisError::Message(err.to_string()))?;
    let (stops, rides) = load_transit_corridors(transit_path, &walk)?;
    let transit = TransitGraph::new(walk.clone(), stops, rides, TRANSFER_PENALTY_MIN)
        .map_err(|err| AnalysisError::Message(err.to_string()))?;

    let mut checks = Vec::new();
    let origin = walk
        .snap_node(ORIGIN)
        .map_err(|err| AnalysisError::Message(err.to_string()))?;
    let snap_m = WalkGraph::euclidean_distance_m(ORIGIN, walk.node(origin));
    checks.push(RaceCheck {
        name: "origin_snap",
        passed: snap_m <= ORIGIN_SNAP_LIMIT_M,
        detail: format!("名古屋駅 snapped {snap_m:.0} m (limit {ORIGIN_SNAP_LIMIT_M:.0} m)"),
    });

    let walk_times = walk.travel_times_from(origin);
    let transit_times = transit.travel_times_from(origin);
    let speed = walk.walk_speed_m_per_min();
    let mut walk_floor_violations = 0;
    let mut ride_floor_violations = 0;
    let mut slower = 0;
    for node in 0..walk.node_count() {
        let straight = WalkGraph::euclidean_distance_m(walk.node(origin), walk.node(node as u32));
        if walk_times[node].is_some_and(|t| t + EPS_MIN < straight / speed) {
            walk_floor_violations += 1;
        }
        if transit_times[node].is_some_and(|t| t + EPS_MIN < straight / RIDE_SPEED_M_PER_MIN) {
            ride_floor_violations += 1;
        }
        match (walk_times[node], transit_times[node]) {
            (Some(w), Some(t)) if t > w + EPS_MIN => slower += 1,
            (Some(_), None) => slower += 1,
            _ => {}
        }
    }
    checks.push(RaceCheck {
        name: "walk_speed_floor",
        passed: walk_floor_violations == 0,
        detail: format!("{walk_floor_violations} nodes faster than straight line at {speed} m/min"),
    });
    checks.push(RaceCheck {
        name: "ride_speed_floor",
        passed: ride_floor_violations == 0,
        detail: format!(
            "{ride_floor_violations} nodes faster than straight line at {RIDE_SPEED_M_PER_MIN} m/min"
        ),
    });
    checks.push(RaceCheck {
        name: "transit_never_slower",
        passed: slower == 0,
        detail: format!("{slower} nodes slower or unreachable once rail is added"),
    });

    let poi_points = load_poi_points(pois_path)?;
    let pois: Vec<u32> = poi_points
        .iter()
        .filter_map(|(point, _)| walk.snap_node(*point).ok())
        .collect();
    checks.push(RaceCheck {
        name: "poi_snapping",
        passed: pois.len() == poi_points.len(),
        detail: format!(
            "{}/{} POIs snapped to the walk graph",
            pois.len(),
            poi_points.len()
        ),
    });

    if let Some(failed) = checks.iter().find(|check| !check.passed) {
        return Err(AnalysisError::Message(format!(
            "isochrone race render refused (fail-closed): {}: {}",
            failed.name, failed.detail
        )));
    }

    let mut edges = Vec::new();
    for from in 0..walk.node_count() as u32 {
        for &(to, cost) in walk.adjacency(from) {
            if from < to {
                let metres = WalkGraph::euclidean_distance_m(walk.node(from), walk.node(to));
                edges.push((from, to, cost, metres));
            }
        }
    }
    // Frame the area the faster mode reaches by the end, with a margin.
    let reach = (0..walk.node_count() as u32)
        .filter(|&n| reached(&transit_times, n, MAX_MINUTES as f64))
        .map(|n| walk.node(n))
        .fold(
            [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ],
            |b, (x, y)| [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)],
        );
    let (mx, my) = ((reach[2] - reach[0]) * 0.12, (reach[3] - reach[1]) * 0.12);
    let bounds = [reach[0] - mx, reach[1] - my, reach[2] + mx, reach[3] + my];

    let network_doc = read_json(network_path)?;
    let transit_doc = read_json(transit_path)?;
    let wards_doc = read_json(genegis_catalog::nagoya_wards_geojson_path())?;
    let synthetic = is_synthetic(&network_doc) || is_synthetic(&transit_doc);
    let rails = line_strings(&transit_doc, |props| {
        matches!(props["mode"].as_str(), Some("rail") | Some("bus"))
    });
    let wards = line_strings(&wards_doc, |_| true);

    Ok(Race {
        walk,
        edges,
        walk_times,
        transit_times,
        pois,
        wards,
        rails,
        checks,
        bounds,
        synthetic,
    })
}

/// Run the pre-render checks without rendering.
pub fn verify_isochrone_race() -> Result<Vec<RaceCheck>, AnalysisError> {
    build_race().map(|race| race.checks)
}

struct Viewport {
    ox: f64,
    oy: f64,
    kx: f64,
    scale: f64,
    west: f64,
    north: f64,
}

impl Viewport {
    fn fit(bounds: [f64; 4], x: f64, y: f64, w: f64, h: f64) -> Self {
        let kx = ((bounds[1] + bounds[3]) / 2.0).to_radians().cos();
        let scale = (w / ((bounds[2] - bounds[0]) * kx)).min(h / (bounds[3] - bounds[1]));
        Viewport {
            ox: x + (w - (bounds[2] - bounds[0]) * kx * scale) / 2.0,
            oy: y + (h - (bounds[3] - bounds[1]) * scale) / 2.0,
            kx,
            scale,
            west: bounds[0],
            north: bounds[3],
        }
    }

    fn at(&self, (lon, lat): (f64, f64)) -> (f64, f64) {
        (
            self.ox + (lon - self.west) * self.kx * self.scale,
            self.oy + (self.north - lat) * self.scale,
        )
    }
}

fn reached(times: &[Option<f64>], node: u32, minutes: f64) -> bool {
    times[node as usize].is_some_and(|t| t <= minutes + EPS_MIN)
}

fn count_pois(race: &Race, times: &[Option<f64>], minutes: f64) -> usize {
    race.pois
        .iter()
        .filter(|&&node| reached(times, node, minutes))
        .count()
}

/// Street length (km) whose both ends are reached within `minutes`.
fn reached_km(race: &Race, times: &[Option<f64>], minutes: f64) -> f64 {
    race.edges
        .iter()
        .filter(|(a, b, _, _)| reached(times, *a, minutes) && reached(times, *b, minutes))
        .map(|(_, _, _, metres)| metres)
        .sum::<f64>()
        / 1000.0
}

fn draw_panel(
    race: &Race,
    times: &[Option<f64>],
    minutes: f64,
    view: &Viewport,
    with_rail: bool,
) -> String {
    let mut out = String::new();
    for ring in &race.wards {
        let points: Vec<String> = ring
            .iter()
            .map(|&p| {
                let (x, y) = view.at(p);
                format!("{x:.1},{y:.1}")
            })
            .collect();
        out.push_str(&format!(
            r##"<polygon points="{}" fill="#0b1630" stroke="#24375c" stroke-width="0.8"/>"##,
            points.join(" ")
        ));
    }
    for &(a, b, cost, _) in &race.edges {
        let (pa, pb) = (view.at(race.walk.node(a)), view.at(race.walk.node(b)));
        out.push_str(&format!(
            r##"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="#1c2a45" stroke-width="1.2"/>"##,
            pa.0, pa.1, pb.0, pb.1
        ));
        // Grow the lit part of each segment from whichever end is reached.
        for (from, to, from_p, to_p) in [(a, b, pa, pb), (b, a, pb, pa)] {
            let Some(start) = times[from as usize] else {
                continue;
            };
            if minutes < start {
                continue;
            }
            let finish = times[to as usize].unwrap_or(f64::INFINITY);
            let fraction = if finish < start {
                0.0
            } else {
                ((minutes - start) / cost.max(EPS_MIN)).clamp(0.0, 1.0)
            };
            if fraction <= 0.0 {
                continue;
            }
            let end = (
                from_p.0 + (to_p.0 - from_p.0) * fraction,
                from_p.1 + (to_p.1 - from_p.1) * fraction,
            );
            let color = hex(ramp(1.0 - start / MAX_MINUTES as f64), 1.0);
            out.push_str(&format!(
                r##"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="{color}" stroke-width="2.4" stroke-linecap="round"/>"##,
                from_p.0, from_p.1, end.0, end.1
            ));
        }
    }
    if with_rail {
        for rail in &race.rails {
            let points: Vec<String> = rail
                .iter()
                .map(|&p| {
                    let (x, y) = view.at(p);
                    format!("{x:.1},{y:.1}")
                })
                .collect();
            out.push_str(&format!(
                r##"<polyline points="{}" fill="none" stroke="#38bdf8" stroke-width="3.2" stroke-opacity="0.85" stroke-dasharray="9 5"/>"##,
                points.join(" ")
            ));
        }
    }
    for &node in &race.pois {
        let (x, y) = view.at(race.walk.node(node));
        let on = reached(times, node, minutes);
        out.push_str(&format!(
            r##"<circle cx="{x:.1}" cy="{y:.1}" r="{}" fill="{}" stroke="#0b1222" stroke-width="1"/>"##,
            if on { 4.2 } else { 2.6 },
            if on { "#fde68a" } else { "#334155" },
        ));
    }
    let (sx, sy) = view.at(ORIGIN);
    out.push_str(&format!(
        r##"<circle cx="{sx:.1}" cy="{sy:.1}" r="7" fill="#f8fafc" stroke="#0b1222" stroke-width="2"/><text x="{:.1}" y="{:.1}" font-family="{FONT}" font-size="12" font-weight="bold" fill="#f8fafc">名古屋駅</text>"##,
        sx + 10.0,
        sy - 8.0
    ));
    out
}

fn render_frame(race: &Race, minute: usize) -> Result<Vec<u8>, AnalysisError> {
    let minutes = minute as f64;
    let total = race.pois.len();
    let mut body = String::new();
    for (x, title, times, with_rail, accent) in [
        (LEFT_X, "徒歩のみ", &race.walk_times, false, "#94a3b8"),
        (
            RIGHT_X,
            "徒歩 ＋ 鉄道",
            &race.transit_times,
            true,
            "#38bdf8",
        ),
    ] {
        let view = Viewport::fit(
            race.bounds,
            x + 12.0,
            PANEL_Y + 40.0,
            PANEL_W - 24.0,
            PANEL_H - 52.0,
        );
        let now = count_pois(race, times, minutes);
        let km = reached_km(race, times, minutes);
        let at_mark = count_pois(race, times, HIGHLIGHT_MINUTES);
        let map = format!(
            r##"<clipPath id="clip{id}"><rect x="{x}" y="{cy:.0}" width="{PANEL_W}" height="{ch:.0}" rx="14"/></clipPath><g clip-path="url(#clip{id})">{}</g>"##,
            draw_panel(race, times, minutes, &view, with_rail),
            id = x as u32,
            cy = PANEL_Y + 40.0,
            ch = PANEL_H - 40.0,
        );
        let mark_text = if minutes >= HIGHLIGHT_MINUTES {
            format!("{at_mark} 施設")
        } else {
            "— 施設".into()
        };
        body.push_str(&format!(
            r##"<rect x="{x}" y="{PANEL_Y}" width="{PANEL_W}" height="{PANEL_H}" rx="14" fill="#08101f" stroke="#1e293b"/>
<text x="{tx:.0}" y="{ty:.0}" font-family="{FONT}" font-size="17" font-weight="bold" fill="{accent}">{title}</text>
<text x="{rx:.0}" y="{ty:.0}" font-family="{FONT}" font-size="15" fill="#e2e8f0" text-anchor="end">到達道路 <tspan font-weight="bold" fill="#f8fafc">{km:.0} km</tspan> · 施設 <tspan font-weight="bold" fill="#fde68a">{now}</tspan> / {total}</text>
{map}
<text x="{tx:.0}" y="{sy:.0}" font-family="{FONT}" font-size="13" fill="#94a3b8">{mark:.0}分で到達できる施設</text>
<text x="{tx:.0}" y="{sy2:.0}" font-family="{FONT}" font-size="30" font-weight="bold" fill="{mark_fill}">{mark_text}</text>"##,
            tx = x + 18.0,
            ty = PANEL_Y + 28.0,
            rx = x + PANEL_W - 18.0,
            sy = PANEL_Y + PANEL_H + 26.0,
            sy2 = PANEL_Y + PANEL_H + 60.0,
            mark = HIGHLIGHT_MINUTES,
            mark_fill = if minutes >= HIGHLIGHT_MINUTES { "#f8fafc" } else { "#334155" },
        ));
    }

    let walk_mark = count_pois(race, &race.walk_times, HIGHLIGHT_MINUTES);
    let rail_mark = count_pois(race, &race.transit_times, HIGHLIGHT_MINUTES);
    let gain = if minutes >= HIGHLIGHT_MINUTES && walk_mark > 0 {
        format!(
            r##"<text x="{x:.0}" y="{y:.0}" font-family="{FONT}" font-size="20" font-weight="bold" fill="#38bdf8" text-anchor="end">徒歩の {:.1} 倍</text>"##,
            rail_mark as f64 / walk_mark as f64,
            x = RIGHT_X + PANEL_W - 18.0,
            y = PANEL_Y + PANEL_H + 58.0,
        )
    } else {
        String::new()
    };

    let mut legend = String::new();
    for i in 0..=8 {
        let t = i as f64 / 8.0;
        legend.push_str(&format!(
            r##"<stop offset="{t:.3}" stop-color="{}"/>"##,
            hex(ramp(1.0 - t), 1.0)
        ));
    }

    let check_text = format!(
        "✓ 描画前チェック {}/{}（{}）",
        race.checks.len(),
        race.checks.len(),
        race.checks
            .iter()
            .map(|check| check.name)
            .collect::<Vec<_>>()
            .join(" · ")
    );
    let data_line = if race.synthetic {
        "データ: 合成fixture（約400m格子の歩行網・3路線の鉄道回廊・100施設。実測ではない）· 実データは GENEGIS_WALK_NETWORK_PATH / GENEGIS_TRANSIT_PATH / GENEGIS_POIS_PATH"
    } else {
        "データ: © OpenStreetMap contributors (ODbL) · 国土数値情報 N02/N07 · 計画用の近似（時刻表ではない）"
    };

    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{FRAME_W}" height="{FRAME_H}" viewBox="0 0 {FRAME_W} {FRAME_H}">
<defs><linearGradient id="time" x1="0" y1="0" x2="1" y2="0">{legend}</linearGradient></defs>
<rect width="100%" height="100%" fill="#050a16"/>
<text x="24" y="50" font-family="{FONT}" font-size="26" font-weight="bold" fill="#f8fafc">名古屋駅から、何分でどこまで行ける？</text>
<text x="24" y="76" font-family="{FONT}" font-size="13" fill="#94a3b8">同じ歩行ネットワーク・同じ施設。右だけ鉄道を足して Dijkstra（待ち時間 + 乗換 {TRANSFER_PENALTY_MIN:.0} 分込み）</text>
<text x="24" y="104" font-family="{FONT}" font-size="12" fill="#34d399">{checks}</text>
<text x="1176" y="72" font-family="{FONT}" font-size="58" font-weight="bold" fill="#f8fafc" text-anchor="end">{minute}<tspan font-size="24" fill="#94a3b8"> 分</tspan></text>
<rect x="960" y="92" width="216" height="8" rx="4" fill="url(#time)"/>
<text x="960" y="118" font-family="{FONT}" font-size="11" fill="#94a3b8">0 分</text>
<text x="1176" y="118" font-family="{FONT}" font-size="11" fill="#94a3b8" text-anchor="end">{MAX_MINUTES} 分（色 = 到達時刻）</text>
{body}
{gain}
<text x="24" y="664" font-family="{FONT}" font-size="10.5" fill="#64748b">{data}</text>
</svg>"##,
        checks = escape_xml(&check_text),
        data = escape_xml(data_line),
    );
    rasterize_svg(&svg)
}

/// Render one frame every `STEP_MINUTES` from 0 to `MAX_MINUTES`.
pub fn render_isochrone_race_frames() -> Result<Vec<IsochroneRaceFrame>, AnalysisError> {
    let race = build_race()?;
    (0..=MAX_MINUTES)
        .step_by(STEP_MINUTES)
        .enumerate()
        .map(|(index, minute)| {
            Ok(IsochroneRaceFrame {
                name: format!("isochrone-race-{index:02}"),
                png: render_frame(&race, minute)?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_passes_every_pre_render_check() {
        let checks = verify_isochrone_race().expect("race verifies");
        assert_eq!(checks.len(), 5);
        assert!(checks.iter().all(|check| check.passed), "{checks:?}");
    }

    #[test]
    fn rail_reaches_more_pois_and_streets_by_the_highlight_mark() {
        let race = build_race().expect("race");
        assert!(
            race.synthetic,
            "bundled fixtures must be labelled synthetic"
        );
        let walk = count_pois(&race, &race.walk_times, HIGHLIGHT_MINUTES);
        let rail = count_pois(&race, &race.transit_times, HIGHLIGHT_MINUTES);
        assert!(rail > walk, "rail {rail} vs walk {walk}");
        let walk_km = reached_km(&race, &race.walk_times, HIGHLIGHT_MINUTES);
        let rail_km = reached_km(&race, &race.transit_times, HIGHLIGHT_MINUTES);
        assert!(rail_km > walk_km, "rail {rail_km} km vs walk {walk_km} km");
        assert_eq!(count_pois(&race, &race.walk_times, 0.0), 0);
    }

    #[test]
    fn renders_first_and_last_minute_as_png() {
        let race = build_race().expect("race");
        for minute in [0, MAX_MINUTES] {
            let png = render_frame(&race, minute).expect("render");
            assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        }
    }
}
