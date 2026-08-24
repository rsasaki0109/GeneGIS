//! Generate the UC-5 two-epoch change-detection fixtures.
//!
//! Writes two deterministic uncompressed LAS files over the same 600 m ×
//! 400 m planar AOI (EPSG:6678-like local metres):
//!   examples/nagoya-population-density/data/nagoya-change-epoch-a.las  (2024)
//!   examples/nagoya-population-density/data/nagoya-change-epoch-b.las  (2025)
//!
//! Scene: sloped ground plane + texture; epoch A has three buildings and two
//! vegetation clusters. Epoch B adds one building, removes another, grows the
//! first cluster by ~2.5 m, clears the second, and leaves a control quadrant
//! byte-identical to epoch A.
//!
//! Usage: cargo run -p genegis-pointcloud --example write_change_fixture

use std::io::BufWriter;

const WIDTH_M: f64 = 600.0;
const HEIGHT_M: f64 = 400.0;
const GROUND_STEP_M: f64 = 2.5;

struct Building {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    height_m: f64,
}

/// (footprint, height) per epoch — `None` height means removed.
const BUILDINGS_A: [Building; 3] = [
    Building {
        x0: 100.0,
        y0: 80.0,
        x1: 140.0,
        y1: 120.0,
        height_m: 12.0,
    },
    Building {
        x0: 300.0,
        y0: 200.0,
        x1: 340.0,
        y1: 240.0,
        height_m: 8.0,
    },
    Building {
        x0: 450.0,
        y0: 60.0,
        x1: 480.0,
        y1: 90.0,
        height_m: 6.0,
    },
];
const BUILDING_ADDED_B: Building = Building {
    x0: 180.0,
    y0: 260.0,
    x1: 220.0,
    y1: 300.0,
    height_m: 10.0,
};

struct Trees {
    cx: f64,
    cy: f64,
    radius_m: f64,
    base_h: f64,
}

const TREES_GROWING: Trees = Trees {
    cx: 200.0,
    cy: 150.0,
    radius_m: 15.0,
    base_h: 5.0,
};
const TREES_CLEARED: Trees = Trees {
    cx: 500.0,
    cy: 300.0,
    radius_m: 12.0,
    base_h: 4.0,
};

/// Quadrant that must stay byte-identical between epochs.
pub const CONTROL_AREA: (f64, f64, f64, f64) = (20.0, 20.0, 80.0, 180.0);

fn main() {
    let data_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/nagoya-population-density/data");
    std::fs::create_dir_all(&data_dir).expect("data dir");

    let points_a = sample_scene(false);
    let points_b = sample_scene(true);
    write_las(&data_dir.join("nagoya-change-epoch-a.las"), &points_a);
    write_las(&data_dir.join("nagoya-change-epoch-b.las"), &points_b);
    println!(
        "wrote epochs: A={} pts, B={} pts",
        points_a.len(),
        points_b.len()
    );
}

fn ground_z(x: f64, y: f64) -> f64 {
    // Gentle slope plus deterministic checkerboard micro-relief.
    let parity = (((x / 2.0).floor() as i64) + ((y / 2.0).floor() as i64)).rem_euclid(2);
    0.02 * x - 0.01 * y + if parity == 0 { 0.04 } else { -0.04 }
}

fn tree_height(x: f64, y: f64, t: &Trees) -> f64 {
    // Deterministic pseudo-random canopy heights from position hash.
    let seed = ((x * 7.0).sin() + (y * 13.0).cos()) * 0.5;
    t.base_h + seed.abs() * 2.0
}

fn sample_scene(epoch_b: bool) -> Vec<las::Point> {
    let mut points = Vec::new();

    // Ground grid covers the whole AOI in both epochs (full coverage keeps
    // per-cell minima comparable).
    let mut y = 0.0;
    while y <= HEIGHT_M {
        let mut x = 0.0;
        while x <= WIDTH_M {
            points.push(point(x, y, ground_z(x, y)));
            x += GROUND_STEP_M;
        }
        y += GROUND_STEP_M;
    }

    for b in &BUILDINGS_A {
        push_building(
            &mut points,
            b,
            !epoch_b || b.height_m != BUILDINGS_A[2].height_m,
        );
    }
    if epoch_b {
        push_building(&mut points, &BUILDING_ADDED_B, true);
    } else {
        // Epoch A reserves the added-building lot with ground only (already
        // covered), so B's new roof is unambiguous.
    }

    for t in [&TREES_GROWING, &TREES_CLEARED] {
        let active = if t.cx == TREES_CLEARED.cx {
            !epoch_b
        } else {
            true
        };
        if active {
            push_trees(&mut points, t, epoch_b);
        }
    }

    points.sort_by(|a, b| {
        a.x.partial_cmp(&b.x)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal))
            .then(a.z.partial_cmp(&b.z).unwrap_or(std::cmp::Ordering::Equal))
    });
    points
}

fn push_building(points: &mut Vec<las::Point>, b: &Building, present: bool) {
    if !present {
        return;
    }
    let step = 1.6_f64;
    let mut y = b.y0;
    while y <= b.y1 {
        let mut x = b.x0;
        while x <= b.x1 {
            points.push(point(x, y, ground_z(x, y) + b.height_m));
            x += step;
        }
        y += step;
    }
}

fn push_trees(points: &mut Vec<las::Point>, t: &Trees, grown: bool) {
    let step = 2.0_f64;
    let mut dy = -t.radius_m;
    while dy <= t.radius_m {
        let mut dx = -t.radius_m;
        while dx <= t.radius_m {
            let x = t.cx + dx;
            let y = t.cy + dy;
            if dx * dx + dy * dy <= t.radius_m * t.radius_m {
                let extra = if grown && t.cx == TREES_GROWING.cx {
                    2.5
                } else {
                    0.0
                };
                let h = tree_height(x, y, t) + extra;
                points.push(point(x, y, ground_z(x, y) + h));
            }
            dx += step;
        }
        dy += step;
    }
}

fn point(x: f64, y: f64, z: f64) -> las::Point {
    las::Point {
        x,
        y,
        z,
        ..Default::default()
    }
}

fn write_las(path: &std::path::Path, points: &[las::Point]) {
    let header = las::Header::default();
    let mut writer = las::Writer::new(
        BufWriter::new(std::fs::File::create(path).expect("create fixture")),
        header,
    )
    .expect("las writer");
    for chunk in points.chunks(1024) {
        writer.write_points(chunk).expect("write points");
    }
}
