//! Deterministic 3D district showcase (Phase 14 M0).
//!
//! Renders orbit-camera frames over a synthetic Japanese suburban district
//! fixture: point-cloud terrain, LOD1 buildings with generated heights, a road
//! network, a POI layer, and a dashboard strip (height-band histogram, POI
//! breakdown, building KPI). Frames are painter's-algorithm SVG rasterized
//! through the same resvg pipeline as the other showcase frames, so the
//! committed GIF stays regenerable bit-stable modulo palette.

use crate::showcase::{escape_xml, rasterize_svg};
use crate::AnalysisError;

pub struct District3dFrame {
    pub name: String,
    pub png: Vec<u8>,
}

const FRAME_W: f64 = 960.0;
const FRAME_H: f64 = 600.0;
const MAP_W: f64 = 660.0;
const DASH_X: f64 = 668.0;
const FL: f64 = 400.0;
const FRAME_COUNT: usize = 18;
const YAW_STEP_DEG: f64 = 20.0;

const HEIGHT_BANDS: [(&str, f64, f64, &str); 5] = [
    ("〜4 m", 0.0, 4.0, "#cfd8dc"),
    ("4–6 m", 4.0, 6.0, "#90a4ae"),
    ("6–9 m", 6.0, 9.0, "#ffcc80"),
    ("9–15 m", 9.0, 15.0, "#ff8a65"),
    ("15 m〜", 15.0, f64::INFINITY, "#e53935"),
];

const ROAD_HALF_WIDTH: f64 = 5.5;
const STREET_X: [f64; 4] = [-105.0, -35.0, 35.0, 105.0];
const STREET_Y: [f64; 3] = [-105.0, 0.0, 105.0];

struct Lcg(u64);

impl Lcg {
    fn next_f64(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 11) as f64) / ((1u64 << 53) as f64)
    }

    fn range(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.next_f64()
    }
}

struct Building {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    base_z: f64,
    height: f64,
}

struct Poi {
    x: f64,
    y: f64,
    label: &'static str,
    category: &'static str,
}

struct Scene {
    ground: Vec<[f64; 3]>,
    vegetation: Vec<[f64; 3]>,
    buildings: Vec<Building>,
    roads_x: Vec<f64>,
    roads_y: Vec<f64>,
    pois: Vec<Poi>,
}

struct Camera {
    eye: [f64; 3],
    fwd: [f64; 3],
    right: [f64; 3],
    up: [f64; 3],
}

fn normalize(v: [f64; 3]) -> [f64; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / len, v[1] / len, v[2] / len]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

impl Camera {
    fn orbit(yaw_deg: f64) -> Self {
        let center = [0.0_f64, 0.0, 4.0];
        let radius = 215.0;
        let pitch = 32.0_f64.to_radians();
        let yaw = yaw_deg.to_radians();
        let (sy, cy) = yaw.sin_cos();
        let eye = [
            center[0] + radius * pitch.cos() * sy,
            center[1] + radius * pitch.cos() * cy,
            center[2] + radius * pitch.sin(),
        ];
        let fwd = normalize(sub(center, eye));
        let right = normalize(cross(fwd, [0.0, 0.0, 1.0]));
        let up = cross(right, fwd);
        Camera {
            eye,
            fwd,
            right,
            up,
        }
    }

    /// Project a world point to `(sx, sy, depth)`; `None` when behind the near plane.
    fn project(&self, p: [f64; 3]) -> Option<(f64, f64, f64)> {
        let d = sub(p, self.eye);
        let depth = dot(d, self.fwd);
        if depth < 12.0 {
            return None;
        }
        let sx = MAP_W / 2.0 + FL * dot(d, self.right) / depth;
        let sy = 340.0 - FL * dot(d, self.up) / depth;
        Some((sx, sy, depth))
    }

    fn distance(&self, p: [f64; 3]) -> f64 {
        let d = sub(p, self.eye);
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    }
}

fn terrain_z(x: f64, y: f64) -> f64 {
    2.2 * (x / 65.0).sin() + 1.7 * (y / 48.0).cos() + 0.9 * ((x + y) / 37.0).sin()
}

fn height_band_index(height: f64) -> usize {
    HEIGHT_BANDS
        .iter()
        .position(|&(_, low, high, _)| height >= low && height < high)
        .unwrap_or(HEIGHT_BANDS.len() - 1)
}

fn on_street(x: f64, y: f64) -> bool {
    STREET_X
        .iter()
        .any(|sx| (x - sx).abs() < ROAD_HALF_WIDTH + 2.0)
        || STREET_Y
            .iter()
            .any(|sy| (y - sy).abs() < ROAD_HALF_WIDTH + 2.0)
}

fn build_scene() -> Scene {
    let mut rng = Lcg(0x47e5_2026_0826_a11c);

    let extent: f64 = 140.0;
    let step = 4.5;
    let mut ground = Vec::new();
    let mut gy = -extent;
    while gy <= extent {
        let mut gx = -extent;
        while gx <= extent {
            if !on_street(gx, gy) {
                ground.push([gx, gy, terrain_z(gx, gy)]);
            }
            gx += step;
        }
        gy += step;
    }

    let mut vegetation = Vec::new();
    for _ in 0..90 {
        loop {
            let x = rng.range(-extent, extent);
            let y = rng.range(-extent, extent);
            if !on_street(x, y) {
                let cluster = rng.range(2.0, 6.5);
                for _ in 0..6 {
                    vegetation.push([
                        x + rng.range(-2.2, 2.2),
                        y + rng.range(-2.2, 2.2),
                        terrain_z(x, y) + cluster * rng.range(0.55, 1.05),
                    ]);
                }
                break;
            }
        }
    }

    let mut buildings = Vec::new();
    for pair in STREET_X.windows(2) {
        for span in STREET_Y.windows(2) {
            let x_lo = pair[0] + ROAD_HALF_WIDTH + 4.0;
            let x_hi = pair[1] - ROAD_HALF_WIDTH - 4.0;
            let y_lo = span[0] + ROAD_HALF_WIDTH + 4.0;
            let y_hi = span[1] - ROAD_HALF_WIDTH - 4.0;
            if x_hi - x_lo < 10.0 || y_hi - y_lo < 10.0 {
                continue;
            }
            let mut y_cursor = y_lo;
            while y_hi - y_cursor > 8.0 {
                let depth = rng.range(7.5, 11.0);
                if depth > y_hi - y_cursor {
                    break;
                }
                let mut x_cursor = x_lo;
                while x_hi - x_cursor > 7.0 {
                    let width = rng.range(7.0, 13.0);
                    if width > x_hi - x_cursor {
                        break;
                    }
                    if rng.next_f64() > 0.18 {
                        let bx = x_cursor + rng.range(0.0, 1.5);
                        let by = y_cursor + rng.range(0.0, 1.5);
                        let height = match rng.next_f64() {
                            sample if sample < 0.58 => rng.range(3.0, 5.5),
                            sample if sample < 0.85 => rng.range(5.5, 9.0),
                            sample if sample < 0.97 => rng.range(9.0, 15.0),
                            _ => rng.range(15.0, 21.0),
                        };
                        let cx = bx + width / 2.0;
                        let cy = by + depth / 2.0;
                        buildings.push(Building {
                            x0: bx,
                            y0: by,
                            x1: bx + width,
                            y1: by + depth,
                            base_z: terrain_z(cx, cy) - 0.25,
                            height,
                        });
                    }
                    x_cursor += width + rng.range(2.0, 4.5);
                }
                y_cursor += depth + rng.range(2.0, 4.5);
            }
        }
    }

    let pois = vec![
        Poi {
            x: -38.0,
            y: 52.0,
            label: "保育園",
            category: "保育・教育",
        },
        Poi {
            x: 40.0,
            y: 58.0,
            label: "ペットサロン",
            category: "生活サービス",
        },
        Poi {
            x: -68.0,
            y: -46.0,
            label: "美容室",
            category: "生活サービス",
        },
        Poi {
            x: 72.0,
            y: -52.0,
            label: "ゲストハウス",
            category: "宿泊",
        },
        Poi {
            x: 2.0,
            y: 118.0,
            label: "コンビニ",
            category: "小売",
        },
        Poi {
            x: -100.0,
            y: 20.0,
            label: "公園",
            category: "公園・緑地",
        },
    ];

    Scene {
        ground,
        vegetation,
        buildings,
        roads_x: STREET_X.to_vec(),
        roads_y: STREET_Y.to_vec(),
        pois,
    }
}

/// Shade multiplier from a fixed light direction for a face normal.
fn face_shade(normal: [f64; 3]) -> f64 {
    let light = normalize([-0.45, -0.35, 0.82]);
    0.55 + 0.45 * (dot(normal, light)).max(0.0)
}

fn shaded_hex(base: (u8, u8, u8), shade: f64) -> String {
    let channel = |value: u8| ((value as f64 * shade).round() as u8).clamp(0, 255);
    format!(
        "#{:02x}{:02x}{:02x}",
        channel(base.0),
        channel(base.1),
        channel(base.2)
    )
}

enum Item {
    Circle {
        cx: f64,
        cy: f64,
        r: f64,
        fill: String,
    },
    Polygon {
        points: String,
        fill: String,
    },
}

fn render_frame(scene: &Scene, yaw_deg: f64) -> Result<Vec<u8>, AnalysisError> {
    let camera = Camera::orbit(yaw_deg);
    let mut items: Vec<(f64, Item)> = Vec::new();

    for p in scene.ground.iter().copied() {
        let Some((sx, sy, depth)) = camera.project(p) else {
            continue;
        };
        items.push((
            camera.distance(p),
            Item::Circle {
                cx: sx,
                cy: sy,
                r: (330.0 / depth).clamp(0.5, 2.0),
                fill: "#b0bec5".into(),
            },
        ));
    }

    for p in scene.vegetation.iter().copied() {
        let Some((sx, sy, depth)) = camera.project(p) else {
            continue;
        };
        let fill = if p[2] > terrain_z(p[0], p[1]) + 4.0 {
            "#2e7d32"
        } else {
            "#66bb6a"
        };
        items.push((
            camera.distance(p),
            Item::Circle {
                cx: sx,
                cy: sy,
                r: (360.0 / depth).clamp(0.6, 2.2),
                fill: fill.into(),
            },
        ));
    }

    for building in &scene.buildings {
        let band = HEIGHT_BANDS[height_band_index(building.height)].3;
        let band_tint = hex_to_rgb(band);
        let wall_base = mix((224, 218, 206), band_tint, 0.18);
        let top_z = building.base_z + building.height;
        let corners = [
            [building.x0, building.y0],
            [building.x1, building.y0],
            [building.x1, building.y1],
            [building.x0, building.y1],
        ];

        // Roof
        push_face(
            &camera,
            &mut items,
            corners.map(|c| [c[0], c[1], top_z]),
            mix(band_tint, (250, 248, 240), 0.35),
            0.92,
        );

        // Walls between consecutive footprint corners
        for i in 0..4 {
            let a = corners[i];
            let b = corners[(i + 1) % 4];
            push_face(
                &camera,
                &mut items,
                [
                    [a[0], a[1], building.base_z],
                    [b[0], b[1], building.base_z],
                    [b[0], b[1], top_z],
                    [a[0], a[1], top_z],
                ],
                wall_base,
                1.0,
            );
        }
    }

    items.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let mut body = draw_roads(scene, &camera);
    for (_, item) in &items {
        body.push_str(&match item {
            Item::Circle { cx, cy, r, fill } => {
                format!(r#"<circle cx="{cx:.1}" cy="{cy:.1}" r="{r:.2}" fill="{fill}"/>"#)
            }
            Item::Polygon { points, fill } => format!(
                r##"<polygon points="{points}" fill="{fill}" stroke="#37474f" stroke-width="0.4"/>"##
            ),
        });
    }

    for poi in &scene.pois {
        let Some((sx, sy, _)) = camera.project([poi.x, poi.y, terrain_z(poi.x, poi.y)]) else {
            continue;
        };
        body.push_str(&format!(
            r##"<line x1="{sx:.1}" y1="{:.1}" x2="{sx:.1}" y2="{:.1}" stroke="#b71c1c" stroke-width="1.2"/><circle cx="{sx:.1}" cy="{sy:.1}" r="3" fill="#e53935" stroke="#b71c1c"/><text x="{:.1}" y="{sy:.1}" font-family="Noto Serif CJK JP, Yu Gothic, Meiryo, sans-serif" font-size="11" fill="#b71c1c">{}</text>"##,
            sy - 14.0,
            sy - 5.0,
            sx + 6.0,
            escape_xml(poi.label),
        ));
    }

    let svg = wrap_svg(scene, &body);
    rasterize_svg(&svg)
}

fn push_face(
    camera: &Camera,
    items: &mut Vec<(f64, Item)>,
    corners: [[f64; 3]; 4],
    base_rgb: (u8, u8, u8),
    shade_boost: f64,
) {
    let e1 = sub(corners[1], corners[0]);
    let e2 = sub(corners[3], corners[0]);
    let mut normal = cross(e1, e2);
    let len = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
    normal = [normal[0] / len, normal[1] / len, normal[2] / len];
    let centroid = [
        (corners[0][0] + corners[2][0]) / 2.0,
        (corners[0][1] + corners[2][1]) / 2.0,
        (corners[0][2] + corners[2][2]) / 2.0,
    ];
    let to_camera = sub(camera.eye, centroid);
    let facing = dot(normal, to_camera);
    if facing <= 0.0 {
        return;
    }
    let shade = face_shade(normal) * shade_boost;
    let mut projected = String::new();
    for corner in corners {
        let Some((sx, sy, _)) = camera.project(corner) else {
            return;
        };
        projected.push_str(&format!("{sx:.1},{sy:.1} "));
    }
    items.push((
        camera.distance(centroid),
        Item::Polygon {
            points: projected.trim_end().to_string(),
            fill: shaded_hex(base_rgb, shade.clamp(0.35, 1.05)),
        },
    ));
}

fn hex_to_rgb(hex: &str) -> (u8, u8, u8) {
    let value = hex.trim_start_matches('#');
    (
        u8::from_str_radix(&value[0..2], 16).unwrap_or(128),
        u8::from_str_radix(&value[2..4], 16).unwrap_or(128),
        u8::from_str_radix(&value[4..6], 16).unwrap_or(128),
    )
}

fn mix(a: (u8, u8, u8), b: (u8, u8, u8), t: f64) -> (u8, u8, u8) {
    let lerp = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * t).round() as u8;
    (lerp(a.0, b.0), lerp(a.1, b.1), lerp(a.2, b.2))
}

fn draw_roads(scene: &Scene, camera: &Camera) -> String {
    let mut paths = String::new();
    for &x in &scene.roads_x {
        paths.push_str(&road_path(
            camera,
            [[-130.0, x], [-65.0, x], [0.0, x], [65.0, x], [130.0, x]].map(|[y, xx]| [xx, y]),
        ));
    }
    for &y in &scene.roads_y {
        paths.push_str(&road_path(
            camera,
            [[-130.0, y], [-65.0, y], [0.0, y], [65.0, y], [130.0, y]].map(|[x, yy]| [x, yy]),
        ));
    }
    paths
}

fn road_path(camera: &Camera, points: [[f64; 2]; 5]) -> String {
    const ELEVATION: f64 = 0.18;
    let mut parts = Vec::new();
    let mut open = false;
    for [x, y] in points {
        let z = terrain_z(x, y) + ELEVATION;
        match camera.project([x, y, z]) {
            Some((sx, sy, _)) => {
                parts.push(if open {
                    format!("L {sx:.1} {sy:.1}")
                } else {
                    open = true;
                    format!("M {sx:.1} {sy:.1}")
                });
            }
            None => open = false,
        }
    }
    format!(
        r##"<path d="{}" fill="none" stroke="#ece6d9" stroke-linecap="round" stroke-width="4.5"/>"##,
        parts.join(" ")
    )
}

fn wrap_svg(scene: &Scene, map_body: &str) -> String {
    let total_buildings = scene.buildings.len();
    let mut bands: [usize; 5] = [0; 5];
    for building in &scene.buildings {
        bands[height_band_index(building.height)] += 1;
    }
    let max_band = bands.iter().copied().max().unwrap_or(1).max(1) as f64;

    let mut histogram = String::new();
    for (i, (label, _, _, color)) in HEIGHT_BANDS.iter().enumerate() {
        let bar_w = 150.0 * bands[i] as f64 / max_band;
        let y = 210.0 + i as f64 * 30.0;
        histogram.push_str(&format!(
            r##"<rect x="{}" y="{y}" width="14" height="14" rx="3" fill="{color}"/><text x="{}" y="{:.1}" font-family="Noto Serif CJK JP, Yu Gothic, Meiryo, sans-serif" font-size="11" fill="#455a64">{}</text><rect x="{}" y="{:.1}" width="{bar_w:.1}" height="12" rx="2" fill="{color}"/><text x="936" y="{:.1}" text-anchor="end" font-family="Noto Serif CJK JP, Yu Gothic, Meiryo, sans-serif" font-size="11" fill="#37474f">{}</text>"##,
            DASH_X + 4.0,
            DASH_X + 22.0,
            y + 11.0,
            escape_xml(label),
            DASH_X + 96.0,
            y + 1.0,
            y + 11.0,
            bands[i],
        ));
    }

    let mut categories: Vec<(&str, usize)> = Vec::new();
    for poi in &scene.pois {
        match categories
            .iter_mut()
            .find(|(name, _)| *name == poi.category)
        {
            Some((_, count)) => *count += 1,
            None => categories.push((poi.category, 1)),
        }
    }
    let mut poi_rows = String::new();
    for (i, (category, count)) in categories.iter().enumerate() {
        let y = 396.0 + i as f64 * 24.0;
        poi_rows.push_str(&format!(
            r##"<text x="{:.1}" y="{y}" font-family="Noto Serif CJK JP, Yu Gothic, Meiryo, sans-serif" font-size="11.5" fill="#455a64">{} {}件</text>"##,
            DASH_X + 8.0,
            escape_xml(category),
            count,
        ));
    }

    let mut legend = String::new();
    for (i, entry) in [
        ("地形点群", "#b0bec5"),
        ("植生", "#43a047"),
        ("屋根色 = 高さ帯", "#ff8a65"),
    ]
    .iter()
    .enumerate()
    {
        let x = 24.0 + i as f64 * 150.0;
        legend.push_str(&format!(
            r##"<rect x="{x}" y="574" width="12" height="12" rx="3" fill="{}"/><text x="{}" y="584" font-family="Noto Serif CJK JP, Yu Gothic, Meiryo, sans-serif" font-size="11" fill="#546e7a">{}</text>"##,
            entry.1,
            x + 17.0,
            escape_xml(entry.0),
        ));
    }

    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{FRAME_W}" height="{FRAME_H}" viewBox="0 0 {FRAME_W} {FRAME_H}">
<rect width="100%" height="100%" fill="#fafafa"/>
<rect x="0" y="0" width="{MAP_W}" height="{FRAME_H}" fill="#eef2f3"/>
<clipPath id="map"><rect x="0" y="80" width="{MAP_W}" height="520"/></clipPath>
<g clip-path="url(#map)">{map_body}</g>
<text x="34" y="44" font-family="Noto Serif CJK JP, Yu Gothic, Meiryo, sans-serif" font-size="23" font-weight="bold" fill="#263238">3D 地区デモ — 点群地形 × LOD1 建物 × 道路網</text>
<text x="34" y="68" font-family="Noto Serif CJK JP, Yu Gothic, Meiryo, sans-serif" font-size="12" fill="#607d8b">GeneGIS · verified offline fixture · CRS: 局地平面座標 (m) · ソースは実行カプセルに記録</text>
{legend}
<rect x="{DASH_X}" y="88" width="284" height="496" rx="10" fill="#ffffff" stroke="#cfd8dc"/>
<text x="{}" y="120" font-family="Noto Serif CJK JP, Yu Gothic, Meiryo, sans-serif" font-size="15" font-weight="bold" fill="#263238">地区ダッシュボード</text>
<text x="{}" y="152" font-family="Noto Serif CJK JP, Yu Gothic, Meiryo, sans-serif" font-size="12" fill="#546e7a">建物総数</text>
<text x="{}" y="184" font-family="Noto Serif CJK JP, Yu Gothic, Meiryo, sans-serif" font-size="28" font-weight="bold" fill="#0d47a1">{} 棟</text>
<line x1="{DASH_X}" y1="200" x2="940" y2="200" stroke="#eceff1"/>
<text x="{}" y="196" font-family="Noto Serif CJK JP, Yu Gothic, Meiryo, sans-serif" font-size="12" font-weight="bold" fill="#37474f">建物高さ分布</text>
{histogram}
<line x1="{DASH_X}" y1="380" x2="940" y2="380" stroke="#eceff1"/>
<text x="{}" y="376" font-family="Noto Serif CJK JP, Yu Gothic, Meiryo, sans-serif" font-size="12" font-weight="bold" fill="#37474f">POI カテゴリ</text>
{poi_rows}
</svg>"##,
        DASH_X + 16.0,
        DASH_X + 16.0,
        DASH_X + 16.0,
        total_buildings,
        DASH_X + 16.0,
        DASH_X + 16.0,
    )
}

/// Render one orbit frame per yaw step around the fixture district.
pub fn render_district3d_frames() -> Result<Vec<District3dFrame>, AnalysisError> {
    let scene = build_scene();
    let mut frames = Vec::with_capacity(FRAME_COUNT);
    for index in 0..FRAME_COUNT {
        let yaw = index as f64 * YAW_STEP_DEG;
        frames.push(District3dFrame {
            name: format!("district3d-{index:02}"),
            png: render_frame(&scene, yaw)?,
        });
    }
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_complete_orbit_as_png() {
        let frames = render_district3d_frames().expect("render district orbit");

        assert_eq!(frames.len(), FRAME_COUNT);
        for (index, frame) in frames.iter().enumerate() {
            assert_eq!(frame.name, format!("district3d-{index:02}"));
            assert!(frame.png.starts_with(b"\x89PNG\r\n\x1a\n"));
            assert!(frame.png.len() > 10_000);
        }
    }
}
