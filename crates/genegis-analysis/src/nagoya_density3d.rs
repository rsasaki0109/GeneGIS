//! Verified 3D population-density orbit for the north-star prompt.
//!
//! Renders 「名古屋市の人口密度を表示」 as extruded population-mesh columns:
//! every census grid square becomes a prism whose height is its population
//! density (persons/km²). The mesh source is the same one the
//! `nagoya-population-mesh` workflow reads — the bundled synthetic fixture by
//! default, or the licensed e-Stat mesh through `GENEGIS_POPULATION_MESH_PATH`
//! plus `GENEGIS_POPULATION_MESH_SHA`.
//!
//! Rendering is fail-closed: the source checksum, CRS, cell identity, cell
//! areas (two independent area methods), population conservation, ward
//! coverage, and the city-total oracle are checked first, and no frame is
//! produced when any check fails. Every frame states whether the data is the
//! synthetic fixture or real census data, and carries the source and result
//! digests. Frames are painter's-algorithm SVG rasterized through the shared
//! resvg pipeline, like the other showcase frames.

use std::collections::{BTreeMap, BTreeSet};

use genegis_crs::Crs;
use genegis_geometry::{polygon_parts_area_km2_for_crs, PolygonRing};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::showcase::{escape_xml, rasterize_svg};
use crate::AnalysisError;

pub struct NagoyaDensity3dFrame {
    pub name: String,
    pub png: Vec<u8>,
}

/// Named check evaluated before any frame is rendered.
#[derive(Debug, Clone)]
pub struct Density3dCheck {
    pub name: &'static str,
    pub passed: bool,
    pub detail: String,
}

const NAGOYA_ORACLE_JSON: &str =
    include_str!("../../../examples/nagoya-population-density/data/nagoya-oracle-2020.json");
const FIXTURE_MANIFEST_JSON: &str = include_str!(
    "../../../examples/nagoya-population-density/data/nagoya-population-mesh-manifest.json"
);

const PROMPT: &str = "名古屋市の人口密度を表示";
pub(crate) const FONT: &str =
    "IPAGothic, Noto Sans CJK JP, Noto Serif CJK JP, Yu Gothic, Meiryo, sans-serif";

const FRAME_W: f64 = 1200.0;
const FRAME_H: f64 = 675.0;
const MAP_CX: f64 = 440.0;
const MAP_CY: f64 = 380.0;
const PANEL_X: f64 = 862.0;
const FL: f64 = 900.0;

const GROW_FRAMES: usize = 12;
const ORBIT_FRAMES: usize = 30;
const FRAME_COUNT: usize = GROW_FRAMES + ORBIT_FRAMES;
const YAW_START_DEG: f64 = -28.0;
const YAW_STEP_DEG: f64 = 12.0;

/// Tallest column, in km of scene height (vertical exaggeration is stated).
const MAX_HEIGHT_KM: f64 = 3.4;
/// Fraction of each cell left as a gap between neighbouring columns.
const CELL_INSET: f64 = 0.08;
/// City-total tolerance: boundary cells are assigned by centroid, so a real
/// mesh does not reproduce the official city total exactly.
const CITY_TOTAL_TOLERANCE: f64 = 0.02;
/// Maximum disagreement between the two independent cell-area methods.
const AREA_TOLERANCE: f64 = 0.005;

/// Inferno-like ramp: dark, perceptually ordered, readable on a night ground.
const RAMP: [(u8, u8, u8); 9] = [
    (0x2a, 0x10, 0x5c),
    (0x4a, 0x0c, 0x6b),
    (0x78, 0x1c, 0x6d),
    (0xa5, 0x2c, 0x60),
    (0xcf, 0x44, 0x46),
    (0xed, 0x69, 0x25),
    (0xfb, 0x9b, 0x06),
    (0xf7, 0xd1, 0x3d),
    (0xfc, 0xff, 0xa4),
];

#[derive(Debug, Clone)]
struct Cell {
    mesh_id: String,
    bounds: [f64; 4],
    population: u64,
    area_km2: f64,
    density: f64,
    ward_name: String,
}

#[derive(Debug, Clone)]
struct Ward {
    name: String,
    rings: Vec<PolygonRing>,
    density: f64,
}

struct Scene {
    cells: Vec<Cell>,
    wards: Vec<Ward>,
    checks: Vec<Density3dCheck>,
    synthetic: bool,
    mesh_label: &'static str,
    population_total: u64,
    oracle_total: u64,
    min_density: f64,
    max_density: f64,
    source_digest: String,
    result_digest: String,
    center: (f64, f64),
}

#[derive(serde::Deserialize)]
struct Oracle {
    population_total: u64,
    wards: Vec<OracleWard>,
}

#[derive(serde::Deserialize)]
struct OracleWard {
    ward_code: String,
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn strip_sha(value: &str) -> String {
    value.trim().trim_start_matches("sha256:").to_lowercase()
}

/// Mesh source and its declared checksum: the fixture manifest by default, the
/// `GENEGIS_POPULATION_MESH_*` override otherwise.
fn mesh_source() -> (String, Option<String>, bool) {
    if let Ok(path) = std::env::var("GENEGIS_POPULATION_MESH_PATH") {
        let declared = std::env::var("GENEGIS_POPULATION_MESH_SHA").ok();
        return (path, declared, true);
    }
    let manifest: Value =
        serde_json::from_str(FIXTURE_MANIFEST_JSON).expect("fixture manifest is valid JSON");
    let declared = manifest["mesh"]["sha256"].as_str().map(str::to_string);
    (
        genegis_catalog::nagoya_population_mesh_path().to_string(),
        declared,
        false,
    )
}

/// Spherical (authalic radius) area of a lon/lat rectangle — independent of
/// the ellipsoidal method used by `genegis-geometry`.
fn authalic_rect_area_km2(b: [f64; 4]) -> f64 {
    const R: f64 = 6371.0072;
    R * R * (b[2] - b[0]).to_radians() * (b[3].to_radians().sin() - b[1].to_radians().sin())
}

fn ring_bounds(coords: &[(f64, f64)]) -> [f64; 4] {
    coords.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |b, &(x, y)| [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)],
    )
}

fn mesh_label(cell_height_deg: f64) -> &'static str {
    let metres = cell_height_deg * 111_000.0;
    if metres < 375.0 {
        "250mメッシュ"
    } else if metres < 750.0 {
        "500mメッシュ"
    } else {
        "1kmメッシュ"
    }
}

fn build_scene() -> Result<Scene, AnalysisError> {
    let (path, declared, overridden) = mesh_source();
    let bytes = std::fs::read(&path)
        .map_err(|err| AnalysisError::Message(format!("read mesh {path}: {err}")))?;
    let observed = sha256_hex(&bytes);
    let raw: Value = serde_json::from_slice(&bytes)
        .map_err(|err| AnalysisError::Message(format!("parse mesh {path}: {err}")))?;
    let description = raw["description"].as_str().unwrap_or_default();
    let synthetic = !overridden || description.to_uppercase().contains("SYNTHETIC");

    let mut checks = Vec::new();
    let checksum_ok = declared
        .as_deref()
        .is_some_and(|expected| strip_sha(expected) == observed);
    checks.push(Density3dCheck {
        name: "source_checksum",
        passed: checksum_ok,
        detail: format!(
            "expected={} observed=sha256:{observed}",
            declared
                .as_deref()
                .unwrap_or("undeclared (set GENEGIS_POPULATION_MESH_SHA)")
        ),
    });

    let dataset = genegis_vector::read_geojson_path(&path)
        .map_err(|err| AnalysisError::Message(err.to_string()))?;
    let crs = Crs::parse(&dataset.crs).map_err(|err| AnalysisError::Message(err.to_string()))?;
    let geographic = crs.require_known().is_ok() && !crs.is_projected();
    checks.push(Density3dCheck {
        name: "crs_declared",
        passed: geographic,
        detail: format!("CRS = {crs} (grid squares are defined in lon/lat)"),
    });
    if !geographic {
        return Err(refusal(&checks));
    }

    // Merge rows that split one grid square across wards; a repeated
    // (mesh_id, ward_code) pair or a square with two geometries is an error.
    let mut by_mesh: BTreeMap<String, Cell> = BTreeMap::new();
    let mut seen_pairs = BTreeSet::new();
    let mut identity_errors = Vec::new();
    let mut source_total = 0u64;
    let mut max_area_error = 0.0_f64;
    let mut ward_codes = BTreeSet::new();
    for feature in &dataset.features {
        let props = &feature.properties;
        let mesh_id = props["mesh_id"]
            .as_str()
            .or_else(|| props["mesh_code"].as_str())
            .unwrap_or_default()
            .to_string();
        let ward_code = props["ward_code"].as_str().unwrap_or_default().to_string();
        let Some(population) = props["population"].as_u64() else {
            identity_errors.push(format!(
                "{mesh_id}: population is not a non-negative integer"
            ));
            continue;
        };
        let Some(ring) = feature.rings.first() else {
            identity_errors.push(format!("{mesh_id}: no polygon"));
            continue;
        };
        if mesh_id.is_empty() || !seen_pairs.insert((mesh_id.clone(), ward_code.clone())) {
            identity_errors.push(format!("{mesh_id}/{ward_code}: duplicate or missing id"));
            continue;
        }
        ward_codes.insert(ward_code);
        source_total += population;
        let bounds = ring_bounds(&ring.coords);
        let entry = by_mesh.entry(mesh_id.clone()).or_insert_with(|| Cell {
            mesh_id: mesh_id.clone(),
            bounds,
            population: 0,
            area_km2: 0.0,
            density: 0.0,
            ward_name: props["ward_name"].as_str().unwrap_or("").to_string(),
        });
        if entry
            .bounds
            .iter()
            .zip(bounds.iter())
            .any(|(a, b)| (a - b).abs() > 1e-6)
        {
            identity_errors.push(format!("{mesh_id}: one square, two geometries"));
        }
        entry.population += population;
    }
    for cell in by_mesh.values_mut() {
        let ellipsoidal = polygon_parts_area_km2_for_crs(
            &[PolygonRing::new(vec![
                (cell.bounds[0], cell.bounds[1]),
                (cell.bounds[2], cell.bounds[1]),
                (cell.bounds[2], cell.bounds[3]),
                (cell.bounds[0], cell.bounds[3]),
                (cell.bounds[0], cell.bounds[1]),
            ])],
            &crs,
        )
        .map_err(|err| AnalysisError::Message(err.to_string()))?;
        let spherical = authalic_rect_area_km2(cell.bounds);
        max_area_error = max_area_error.max((ellipsoidal - spherical).abs() / ellipsoidal);
        cell.area_km2 = ellipsoidal;
        cell.density = cell.population as f64 / ellipsoidal;
    }
    let cells: Vec<Cell> = by_mesh.into_values().collect();

    checks.push(Density3dCheck {
        name: "mesh_cell_identity",
        passed: identity_errors.is_empty() && !cells.is_empty(),
        detail: if identity_errors.is_empty() {
            format!("{} grid squares, {} rows", cells.len(), seen_pairs.len())
        } else {
            identity_errors.join("; ")
        },
    });
    checks.push(Density3dCheck {
        name: "cell_area_cross_check",
        passed: max_area_error <= AREA_TOLERANCE,
        detail: format!(
            "ellipsoidal vs authalic-sphere max relative difference {:.4}% (threshold {:.1}%)",
            max_area_error * 100.0,
            AREA_TOLERANCE * 100.0
        ),
    });

    let rendered_total: u64 = cells.iter().map(|cell| cell.population).sum();
    checks.push(Density3dCheck {
        name: "population_conserved",
        passed: rendered_total == source_total,
        detail: format!("columns={rendered_total} source_rows={source_total}"),
    });

    let oracle: Oracle =
        serde_json::from_str(NAGOYA_ORACLE_JSON).expect("immutable Nagoya oracle is valid");
    let missing: Vec<&str> = oracle
        .wards
        .iter()
        .map(|ward| ward.ward_code.as_str())
        .filter(|code| !ward_codes.contains(*code))
        .collect();
    checks.push(Density3dCheck {
        name: "ward_coverage_oracle",
        passed: missing.is_empty(),
        detail: if missing.is_empty() {
            format!("all {} wards present", oracle.wards.len())
        } else {
            format!("missing wards: {}", missing.join(", "))
        },
    });
    let total_error = (rendered_total as f64 - oracle.population_total as f64).abs()
        / oracle.population_total as f64;
    checks.push(Density3dCheck {
        name: "city_total_oracle",
        passed: total_error <= CITY_TOTAL_TOLERANCE,
        detail: format!(
            "mesh={rendered_total} official={} relative_error={:.3}% (threshold {:.0}%)",
            oracle.population_total,
            total_error * 100.0,
            CITY_TOTAL_TOLERANCE * 100.0
        ),
    });

    if checks.iter().any(|check| !check.passed) {
        return Err(refusal(&checks));
    }

    // Ward density from mesh-aggregated population over the N03 ward area.
    let boundary = genegis_vector::read_geojson_path(genegis_catalog::nagoya_wards_geojson_path())
        .map_err(|err| AnalysisError::Message(err.to_string()))?;
    let mut population_by_ward: BTreeMap<String, u64> = BTreeMap::new();
    for feature in &dataset.features {
        let props = &feature.properties;
        *population_by_ward
            .entry(props["ward_code"].as_str().unwrap_or_default().to_string())
            .or_default() += props["population"].as_u64().unwrap_or(0);
    }
    let mut wards = Vec::new();
    for feature in &boundary.features {
        let code = feature.properties["ward_code"].as_str().unwrap_or_default();
        let area = polygon_parts_area_km2_for_crs(&feature.rings, &crs)
            .map_err(|err| AnalysisError::Message(err.to_string()))?;
        let population = population_by_ward.get(code).copied().unwrap_or(0);
        wards.push(Ward {
            name: feature.properties["ward_name"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            rings: feature.rings.clone(),
            density: if area > 0.0 {
                population as f64 / area
            } else {
                0.0
            },
        });
    }

    let all = cells.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |b, c| {
            [
                b[0].min(c.bounds[0]),
                b[1].min(c.bounds[1]),
                b[2].max(c.bounds[2]),
                b[3].max(c.bounds[3]),
            ]
        },
    );
    let center = ((all[0] + all[2]) / 2.0, (all[1] + all[3]) / 2.0);
    let max_density = cells.iter().map(|c| c.density).fold(0.0, f64::max);
    let min_density = cells
        .iter()
        .map(|c| c.density)
        .fold(f64::INFINITY, f64::min);
    let mut heights: Vec<f64> = cells.iter().map(|c| c.bounds[3] - c.bounds[1]).collect();
    heights.sort_by(|a, b| a.total_cmp(b));
    let mesh_label = mesh_label(heights[heights.len() / 2]);

    let canonical: String = cells
        .iter()
        .map(|c| format!("{}:{}:{:.3}\n", c.mesh_id, c.population, c.density))
        .collect();

    Ok(Scene {
        cells,
        wards,
        checks,
        synthetic,
        mesh_label,
        population_total: rendered_total,
        oracle_total: oracle.population_total,
        min_density,
        max_density,
        source_digest: observed,
        result_digest: sha256_hex(canonical.as_bytes()),
        center,
    })
}

fn refusal(checks: &[Density3dCheck]) -> AnalysisError {
    let failed: Vec<String> = checks
        .iter()
        .filter(|check| !check.passed)
        .map(|check| format!("{}: {}", check.name, check.detail))
        .collect();
    AnalysisError::Message(format!(
        "3D density render refused (fail-closed): {}",
        failed.join(" | ")
    ))
}

/// Run the pre-render checks without rendering (used by tests and the CLI).
pub fn verify_nagoya_density3d() -> Result<Vec<Density3dCheck>, AnalysisError> {
    build_scene().map(|scene| scene.checks)
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

struct Camera {
    eye: [f64; 3],
    fwd: [f64; 3],
    right: [f64; 3],
    up: [f64; 3],
    screen_center: (f64, f64),
}

impl Camera {
    fn orbit(yaw_deg: f64) -> Self {
        let center = [0.0_f64, 0.0, 0.8];
        let radius = 44.0;
        let pitch = 47.0_f64.to_radians();
        let (sy, cy) = yaw_deg.to_radians().sin_cos();
        let eye = [
            center[0] + radius * pitch.cos() * sy,
            center[1] - radius * pitch.cos() * cy,
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
            screen_center: (MAP_CX, MAP_CY),
        }
    }

    fn centered_at(mut self, x: f64, y: f64) -> Self {
        self.screen_center = (x, y);
        self
    }

    fn project(&self, p: [f64; 3]) -> Option<(f64, f64)> {
        let d = sub(p, self.eye);
        let depth = dot(d, self.fwd);
        if depth < 1.0 {
            return None;
        }
        Some((
            self.screen_center.0 + FL * dot(d, self.right) / depth,
            self.screen_center.1 - FL * dot(d, self.up) / depth,
        ))
    }

    fn distance(&self, p: [f64; 3]) -> f64 {
        let d = sub(p, self.eye);
        dot(d, d).sqrt()
    }
}

/// Local equirectangular km around the mesh centre (display only).
fn to_local(center: (f64, f64), lon: f64, lat: f64) -> (f64, f64) {
    let kx = 111.32 * center.1.to_radians().cos();
    ((lon - center.0) * kx, (lat - center.1) * 110.574)
}

pub(crate) fn ramp(t: f64) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0) * (RAMP.len() - 1) as f64;
    let i = (t.floor() as usize).min(RAMP.len() - 2);
    let f = t - i as f64;
    let (a, b) = (RAMP[i], RAMP[i + 1]);
    let mix = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * f).round() as u8;
    (mix(a.0, b.0), mix(a.1, b.1), mix(a.2, b.2))
}

fn color_position(scene: &Scene, density: f64) -> f64 {
    let span = (scene.max_density - scene.min_density).max(f64::EPSILON);
    ((density - scene.min_density) / span).clamp(0.0, 1.0)
}

pub(crate) fn hex(rgb: (u8, u8, u8), shade: f64) -> String {
    let c = |v: u8| ((v as f64 * shade).round()).clamp(0.0, 255.0) as u8;
    format!("#{:02x}{:02x}{:02x}", c(rgb.0), c(rgb.1), c(rgb.2))
}

fn points(projected: &[(f64, f64)]) -> String {
    projected
        .iter()
        .map(|(x, y)| format!("{x:.1},{y:.1}"))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn ease_in_out_cubic(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

fn render_map(scene: &Scene, camera: &Camera, grow: f64) -> String {
    let mut out = String::new();

    // City ground and ward outlines at z = 0.
    for ward in &scene.wards {
        for ring in &ward.rings {
            let projected: Option<Vec<(f64, f64)>> = ring
                .coords
                .iter()
                .map(|&(lon, lat)| {
                    let (x, y) = to_local(scene.center, lon, lat);
                    camera.project([x, y, 0.0])
                })
                .collect();
            if let Some(projected) = projected {
                out.push_str(&format!(
                    r##"<polygon points="{}" fill="#0c1a33" fill-opacity="0.85" stroke="#3d5a8a" stroke-width="1.1" stroke-opacity="0.9"/>"##,
                    points(&projected)
                ));
            }
        }
    }

    let light = normalize([-0.5, -0.4, 0.75]);
    let mut order: Vec<(f64, usize)> = scene
        .cells
        .iter()
        .enumerate()
        .map(|(i, cell)| {
            let (x0, y0) = to_local(scene.center, cell.bounds[0], cell.bounds[1]);
            let (x1, y1) = to_local(scene.center, cell.bounds[2], cell.bounds[3]);
            (camera.distance([(x0 + x1) / 2.0, (y0 + y1) / 2.0, 0.0]), i)
        })
        .collect();
    order.sort_by(|a, b| b.0.total_cmp(&a.0));

    let mut glow = String::new();
    for (_, i) in order {
        let cell = &scene.cells[i];
        // Height is proportional from zero; colour spans the observed range.
        let t = cell.density / scene.max_density;
        let base = ramp(color_position(scene, cell.density));
        let (mut x0, mut y0) = to_local(scene.center, cell.bounds[0], cell.bounds[1]);
        let (mut x1, mut y1) = to_local(scene.center, cell.bounds[2], cell.bounds[3]);
        let (ix, iy) = ((x1 - x0) * CELL_INSET / 2.0, (y1 - y0) * CELL_INSET / 2.0);
        x0 += ix;
        x1 -= ix;
        y0 += iy;
        y1 -= iy;
        let h = (t * MAX_HEIGHT_KM * grow).max(0.01);
        let corner = |x: f64, y: f64, z: f64| camera.project([x, y, z]);
        if h > 0.02 {
            let faces = [
                ([0.0, -1.0, 0.0], [(x0, y0), (x1, y0)]),
                ([1.0, 0.0, 0.0], [(x1, y0), (x1, y1)]),
                ([0.0, 1.0, 0.0], [(x1, y1), (x0, y1)]),
                ([-1.0, 0.0, 0.0], [(x0, y1), (x0, y0)]),
            ];
            for (normal, [(ax, ay), (bx, by)]) in faces {
                let mid = [(ax + bx) / 2.0, (ay + by) / 2.0, h / 2.0];
                if dot(normal, sub(camera.eye, mid)) <= 0.0 {
                    continue;
                }
                let quad = [
                    corner(ax, ay, 0.0),
                    corner(bx, by, 0.0),
                    corner(bx, by, h),
                    corner(ax, ay, h),
                ];
                if let [Some(a), Some(b), Some(c), Some(d)] = quad {
                    let shade = 0.38 + 0.4 * dot(normal, light).max(0.0);
                    out.push_str(&format!(
                        r##"<polygon points="{}" fill="{}"/>"##,
                        points(&[a, b, c, d]),
                        hex(base, shade)
                    ));
                }
            }
        }
        let top = [
            corner(x0, y0, h),
            corner(x1, y0, h),
            corner(x1, y1, h),
            corner(x0, y1, h),
        ];
        if let [Some(a), Some(b), Some(c), Some(d)] = top {
            let top_points = points(&[a, b, c, d]);
            out.push_str(&format!(
                r##"<polygon points="{top_points}" fill="{}"/>"##,
                hex(base, 1.0)
            ));
            if color_position(scene, cell.density) > 0.8 {
                glow.push_str(&format!(
                    r##"<polygon points="{top_points}" fill="{}"/>"##,
                    hex(base, 1.0)
                ));
            }
        }
    }
    out.push_str(&format!(
        r##"<g filter="url(#bloom)" opacity="{:.2}">{glow}</g>"##,
        0.55 * grow
    ));
    out
}

fn render_frame(scene: &Scene, index: usize) -> Result<Vec<u8>, AnalysisError> {
    let growing = index < GROW_FRAMES;
    let grow = if growing {
        ease_in_out_cubic((index + 1) as f64 / GROW_FRAMES as f64)
    } else {
        1.0
    };
    let camera = Camera::orbit(YAW_START_DEG + index as f64 * YAW_STEP_DEG);
    let map = render_map(scene, &camera, grow);

    // Checks are computed before rendering; the rise reveals them in order.
    let revealed = if growing {
        ((index + 1) * scene.checks.len()).div_ceil(GROW_FRAMES)
    } else {
        scene.checks.len()
    };
    let all_passed = !growing;
    let (badge_fill, badge_text) = if all_passed {
        (
            "#0f5132",
            format!("✓ 検証済み {}/{}", scene.checks.len(), scene.checks.len()),
        )
    } else {
        (
            "#3a2f0b",
            format!("検証中 {revealed}/{}", scene.checks.len()),
        )
    };
    let badge_stroke = if all_passed { "#34d399" } else { "#fbbf24" };

    let mut check_rows = String::new();
    for (i, check) in scene.checks.iter().enumerate() {
        let y = 452.0 + i as f64 * 19.0;
        let (mark, color) = if i < revealed {
            ("✓", "#34d399")
        } else {
            ("·", "#475569")
        };
        check_rows.push_str(&format!(
            r##"<text x="{:.0}" y="{y:.0}" font-family="{FONT}" font-size="12" fill="{color}">{mark}</text><text x="{:.0}" y="{y:.0}" font-family="DejaVu Sans Mono, monospace" font-size="11.5" fill="{}">{}</text>"##,
            PANEL_X + 18.0,
            PANEL_X + 36.0,
            if i < revealed { "#cbd5e1" } else { "#475569" },
            escape_xml(check.name)
        ));
    }

    let mut ranked: Vec<&Ward> = scene.wards.iter().collect();
    ranked.sort_by(|a, b| b.density.total_cmp(&a.density));
    let top_density = ranked.first().map(|w| w.density).unwrap_or(1.0);
    let mut bars = String::new();
    for (i, ward) in ranked.iter().take(5).enumerate() {
        let y = 268.0 + i as f64 * 24.0;
        let width = 150.0 * ward.density / top_density * grow;
        bars.push_str(&format!(
            r##"<text x="{:.0}" y="{:.0}" font-family="{FONT}" font-size="12" fill="#cbd5e1">{}</text><rect x="{:.0}" y="{:.0}" width="{width:.1}" height="12" rx="3" fill="{}"/><text x="{:.0}" y="{:.0}" font-family="{FONT}" font-size="11" fill="#94a3b8">{}</text>"##,
            PANEL_X + 18.0,
            y + 10.0,
            escape_xml(&ward.name),
            PANEL_X + 72.0,
            y,
            hex(ramp(color_position(scene, ward.density)), 1.0),
            PANEL_X + 72.0 + width + 6.0,
            y + 10.0,
            thousands(ward.density.round() as u64)
        ));
    }

    let peak = scene
        .cells
        .iter()
        .max_by(|a, b| a.density.total_cmp(&b.density))
        .expect("scene has cells");

    let data_line = if scene.synthetic {
        "データ: 合成fixture（実観測ではない・区別公式人口を面積按分）· 実データは GENEGIS_POPULATION_MESH_PATH で同じ検証へ"
    } else {
        "出典：政府統計の総合窓口(e-Stat) 令和2年国勢調査 地域メッシュ統計を加工して作成 · 境界: 国土数値情報 N03"
    };
    let data_badge = if scene.synthetic {
        ("#3b2a12", "#f59e0b", "SYNTHETIC")
    } else {
        ("#0c2f3f", "#38bdf8", "REAL · e-Stat")
    };

    let mut ramp_stops = String::new();
    for i in 0..=8 {
        let t = i as f64 / 8.0;
        ramp_stops.push_str(&format!(
            r##"<stop offset="{t:.3}" stop-color="{}"/>"##,
            hex(ramp(t), 1.0)
        ));
    }

    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{FRAME_W}" height="{FRAME_H}" viewBox="0 0 {FRAME_W} {FRAME_H}">
<defs>
<radialGradient id="sky" cx="0.37" cy="0.6" r="0.75"><stop offset="0" stop-color="#11203d"/><stop offset="0.6" stop-color="#070d1c"/><stop offset="1" stop-color="#03060d"/></radialGradient>
<linearGradient id="ramp" x1="0" y1="0" x2="1" y2="0">{ramp_stops}</linearGradient>
<filter id="bloom" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="7"/></filter>
</defs>
<rect width="100%" height="100%" fill="url(#sky)"/>
<g>{map}</g>
<rect x="28" y="24" width="470" height="52" rx="26" fill="#0f172a" fill-opacity="0.92" stroke="#334155"/>
<circle cx="56" cy="50" r="9" fill="none" stroke="#38bdf8" stroke-width="2.4"/><line x1="62.5" y1="56.5" x2="69" y2="63" stroke="#38bdf8" stroke-width="2.4" stroke-linecap="round"/>
<text x="84" y="58" font-family="{FONT}" font-size="22" font-weight="bold" fill="#f1f5f9">{prompt}</text>
<text x="32" y="102" font-family="{FONT}" font-size="12.5" fill="#94a3b8">GeneGIS · Intent → Workflow → 検証 → 3D · {mesh} × {cells} セル · 高さ = 人口密度 (persons/km²)</text>
<rect x="{badge_x:.0}" y="24" width="168" height="40" rx="20" fill="{badge_fill}" stroke="{badge_stroke}" stroke-width="1.5"/>
<text x="{badge_tx:.0}" y="50" font-family="{FONT}" font-size="16" font-weight="bold" fill="#ecfdf5" text-anchor="middle">{badge_text}</text>
<rect x="{PANEL_X}" y="84" width="316" height="566" rx="14" fill="#0b1222" fill-opacity="0.88" stroke="#1e293b"/>
<text x="{px18:.0}" y="114" font-family="{FONT}" font-size="12" fill="#94a3b8">総人口（メッシュ合計）</text>
<text x="{px18:.0}" y="146" font-family="{FONT}" font-size="28" font-weight="bold" fill="#f8fafc">{total} 人</text>
<text x="{px18:.0}" y="166" font-family="{FONT}" font-size="11" fill="#64748b">公式値 {oracle} 人（令和2年国勢調査）</text>
<text x="{px18:.0}" y="196" font-family="{FONT}" font-size="12" fill="#94a3b8">最高密度セル</text>
<text x="{px18:.0}" y="222" font-family="{FONT}" font-size="20" font-weight="bold" fill="#fcd34d">{peak} 人/km²</text>
<text x="{px190:.0}" y="222" font-family="{FONT}" font-size="12" fill="#94a3b8">{peak_ward}</text>
<text x="{px18:.0}" y="252" font-family="{FONT}" font-size="12" font-weight="bold" fill="#e2e8f0">区別人口密度 TOP5（人/km²）</text>
{bars}
<text x="{px18:.0}" y="428" font-family="{FONT}" font-size="12" font-weight="bold" fill="#e2e8f0">描画前の独立チェック（1つでも落ちれば描画しない）</text>
{check_rows}
<text x="{px18:.0}" y="604" font-family="DejaVu Sans Mono, monospace" font-size="10.5" fill="#64748b">source sha256:{src}…</text>
<text x="{px18:.0}" y="622" font-family="DejaVu Sans Mono, monospace" font-size="10.5" fill="#64748b">result sha256:{res}…</text>
<text x="{px18:.0}" y="640" font-family="DejaVu Sans Mono, monospace" font-size="10.5" fill="#64748b">CRS EPSG:4326 · area: ellipsoidal WGS84</text>
<rect x="32" y="604" width="220" height="10" rx="5" fill="url(#ramp)"/>
<text x="32" y="596" font-family="{FONT}" font-size="11.5" fill="#cbd5e1">人口密度 (persons/km²) · 高さは誇張表示</text>
<text x="32" y="630" font-family="{FONT}" font-size="11" fill="#94a3b8">{min}</text>
<text x="252" y="630" font-family="{FONT}" font-size="11" fill="#94a3b8" text-anchor="end">{max}</text>
<rect x="272" y="600" width="{tag_w:.0}" height="20" rx="10" fill="{tag_fill}" stroke="{tag_stroke}"/>
<text x="{tag_tx:.0}" y="614" font-family="DejaVu Sans, sans-serif" font-size="10.5" font-weight="bold" fill="{tag_stroke}" text-anchor="middle">{tag}</text>
<text x="32" y="656" font-family="{FONT}" font-size="10.5" fill="#64748b">{data_line}</text>
</svg>"##,
        prompt = escape_xml(PROMPT),
        mesh = scene.mesh_label,
        cells = thousands(scene.cells.len() as u64),
        badge_x = PANEL_X + 148.0,
        badge_tx = PANEL_X + 232.0,
        px18 = PANEL_X + 18.0,
        px190 = PANEL_X + 196.0,
        total = thousands(scene.population_total),
        oracle = thousands(scene.oracle_total),
        peak = thousands(peak.density.round() as u64),
        peak_ward = escape_xml(&peak.ward_name),
        src = &scene.source_digest[..16],
        res = &scene.result_digest[..16],
        min = thousands(scene.min_density.round() as u64),
        max = thousands(scene.max_density.round() as u64),
        tag_w = if scene.synthetic { 88.0 } else { 108.0 },
        tag_tx = if scene.synthetic { 316.0 } else { 326.0 },
        tag_fill = data_badge.0,
        tag_stroke = data_badge.1,
        tag = data_badge.2,
        data_line = escape_xml(data_line),
    );
    rasterize_svg(&svg)
}

/// Render a 1280×640 social preview card (GitHub / Open Graph size) from the
/// same verified scene; it refuses to render on the same checks.
pub fn render_nagoya_density3d_social_card() -> Result<Vec<u8>, AnalysisError> {
    let scene = build_scene()?;
    let camera = Camera::orbit(YAW_START_DEG + 8.0 * YAW_STEP_DEG).centered_at(905.0, 330.0);
    let map = render_map(&scene, &camera, 1.0);
    let (tag_fill, tag_stroke, tag) = if scene.synthetic {
        ("#3b2a12", "#f59e0b", "SYNTHETIC 500m FIXTURE")
    } else {
        ("#0c2f3f", "#38bdf8", "REAL · e-Stat 2020 CENSUS MESH")
    };
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="1280" height="640" viewBox="0 0 1280 640">
<defs>
<radialGradient id="sky" cx="0.7" cy="0.55" r="0.8"><stop offset="0" stop-color="#13244a"/><stop offset="0.55" stop-color="#070d1c"/><stop offset="1" stop-color="#03060d"/></radialGradient>
<filter id="bloom" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="7"/></filter>
</defs>
<rect width="100%" height="100%" fill="url(#sky)"/>
<g>{map}</g>
<text x="64" y="150" font-family="Inter, DejaVu Sans, sans-serif" font-size="76" font-weight="bold" fill="#f8fafc">GeneGIS</text>
<text x="66" y="198" font-family="Inter, DejaVu Sans, sans-serif" font-size="25" fill="#cbd5e1">Verified spatial workflows</text>
<text x="66" y="232" font-family="Inter, DejaVu Sans, sans-serif" font-size="25" fill="#cbd5e1">from intent to map.</text>
<rect x="64" y="276" width="400" height="54" rx="27" fill="#0f172a" fill-opacity="0.92" stroke="#334155"/>
<circle cx="94" cy="303" r="9" fill="none" stroke="#38bdf8" stroke-width="2.4"/><line x1="100.5" y1="309.5" x2="107" y2="316" stroke="#38bdf8" stroke-width="2.4" stroke-linecap="round"/>
<text x="122" y="312" font-family="{FONT}" font-size="22" font-weight="bold" fill="#f1f5f9">{prompt}</text>
<rect x="64" y="352" width="208" height="44" rx="22" fill="#0f5132" stroke="#34d399" stroke-width="1.5"/>
<text x="168" y="381" font-family="{FONT}" font-size="18" font-weight="bold" fill="#ecfdf5" text-anchor="middle">✓ 検証済み {n}/{n}</text>
<text x="66" y="440" font-family="{FONT}" font-size="16" fill="#94a3b8">総人口 {total} 人 · {cells} セル · 高さ = 人口密度</text>
<text x="66" y="468" font-family="DejaVu Sans Mono, monospace" font-size="13" fill="#64748b">result sha256:{res}…</text>
<text x="66" y="560" font-family="Inter, DejaVu Sans, sans-serif" font-size="17" fill="#94a3b8">AI-native · Cloud-native · GPU-native · Open source</text>
<rect x="64" y="580" width="{tag_w}" height="26" rx="13" fill="{tag_fill}" stroke="{tag_stroke}"/>
<text x="{tag_tx}" y="598" font-family="DejaVu Sans, sans-serif" font-size="12" font-weight="bold" fill="{tag_stroke}" text-anchor="middle">{tag}</text>
</svg>"##,
        prompt = escape_xml(PROMPT),
        n = scene.checks.len(),
        total = thousands(scene.population_total),
        cells = thousands(scene.cells.len() as u64),
        res = &scene.result_digest[..16],
        tag_w = if scene.synthetic { 190 } else { 250 },
        tag_tx = if scene.synthetic { 159 } else { 189 },
    );
    rasterize_svg(&svg)
}

/// Final orbit frame and its verification state, for the daily map card.
pub(crate) fn card_source() -> Result<crate::daily_card::CardSource, AnalysisError> {
    let scene = build_scene()?;
    Ok(crate::daily_card::CardSource {
        png: render_frame(&scene, FRAME_COUNT - 1)?,
        prompt: PROMPT,
        checks_passed: scene.checks.iter().filter(|c| c.passed).count(),
        checks_total: scene.checks.len(),
        result_digest: scene.result_digest.clone(),
        synthetic: scene.synthetic,
    })
}

/// Render the verified rise-and-orbit sequence over the Nagoya population mesh.
pub fn render_nagoya_density3d_frames() -> Result<Vec<NagoyaDensity3dFrame>, AnalysisError> {
    let scene = build_scene()?;
    (0..FRAME_COUNT)
        .map(|index| {
            Ok(NagoyaDensity3dFrame {
                name: format!("nagoya-density3d-{index:02}"),
                png: render_frame(&scene, index)?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_passes_every_pre_render_check() {
        let checks = verify_nagoya_density3d().expect("fixture verifies");
        assert_eq!(checks.len(), 7);
        assert!(checks.iter().all(|check| check.passed), "{checks:?}");
    }

    #[test]
    fn scene_conserves_the_official_city_total() {
        let scene = build_scene().expect("scene");
        assert!(scene.synthetic, "bundled mesh must be labelled synthetic");
        assert_eq!(scene.population_total, scene.oracle_total);
        assert_eq!(scene.mesh_label, "500mメッシュ");
        assert!(scene.max_density > 0.0);
    }

    #[test]
    fn authalic_and_ellipsoidal_areas_agree_for_a_grid_square() {
        let b = [136.9, 35.15, 136.90625, 35.154166666];
        let spherical = authalic_rect_area_km2(b);
        assert!((spherical - 0.2647).abs() < 0.01, "{spherical}");
    }

    #[test]
    fn renders_first_rise_and_final_orbit_frames_as_png() {
        let scene = build_scene().expect("scene");
        for index in [0, FRAME_COUNT - 1] {
            let png = render_frame(&scene, index).expect("render");
            assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
            assert!(png.len() > 10_000);
        }
    }

    #[test]
    fn social_card_is_github_preview_sized() {
        let png = render_nagoya_density3d_social_card().expect("card");
        assert_eq!(&png[16..24], &[0, 0, 5, 0, 0, 0, 2, 128]);
    }

    #[test]
    fn undeclared_override_checksum_refuses_to_render() {
        let checks = vec![Density3dCheck {
            name: "source_checksum",
            passed: false,
            detail: "expected=undeclared".into(),
        }];
        let message = refusal(&checks).to_string();
        assert!(message.contains("fail-closed") && message.contains("source_checksum"));
    }
}
