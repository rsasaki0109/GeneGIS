//! Scale benchmark for the spatial operations on synthetic city-scale data.
//!
//! ```text
//! cargo run --release -p genegis-toolkit --example bench_ops -- [points] [grid]
//! ```
//!
//! `points` random points and a `grid`×`grid` polygon mesh over central
//! Nagoya; times spatial_join (count), select_by_location (within 300 m),
//! and distance_to_nearest. Every run still executes the independent checks.

use std::collections::BTreeMap;
use std::time::Instant;

use genegis_toolkit::layer::{CrsStatus, Feature, Layer};
use genegis_toolkit::ops;
use geo_types::{polygon, Geometry, Point};
use serde_json::{json, Value};

fn lcg(seed: &mut u64) -> f64 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (*seed >> 11) as f64 / (1u64 << 53) as f64
}

fn main() {
    let mut args = std::env::args().skip(1);
    let points: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(20_000);
    let grid: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(40);
    let (x0, y0, w, h) = (136.80, 35.05, 0.25, 0.20);
    let mut seed = 42;

    let mut pts = Layer::new("points", "EPSG:4326", CrsStatus::Declared);
    for i in 0..points {
        pts.features.push(Feature {
            id: i as u64,
            geometry: Some(Geometry::Point(Point::new(
                x0 + lcg(&mut seed) * w,
                y0 + lcg(&mut seed) * h,
            ))),
            properties: BTreeMap::from([("v".to_string(), Value::from(1))]),
        });
    }
    pts.refresh_schema();

    let mut mesh = Layer::new("mesh", "EPSG:4326", CrsStatus::Declared);
    let (dx, dy) = (w / grid as f64, h / grid as f64);
    for r in 0..grid {
        for c in 0..grid {
            let (x, y) = (x0 + c as f64 * dx, y0 + r as f64 * dy);
            mesh.features.push(Feature {
                id: (r * grid + c) as u64,
                geometry: Some(Geometry::Polygon(polygon![
                    (x: x, y: y), (x: x + dx, y: y), (x: x + dx, y: y + dy), (x: x, y: y + dy), (x: x, y: y)
                ])),
                properties: BTreeMap::from([("pop".to_string(), Value::from(100))]),
            });
        }
    }
    mesh.refresh_schema();

    let mut stations = Layer::new("stations", "EPSG:4326", CrsStatus::Declared);
    for i in 0..200 {
        stations.features.push(Feature {
            id: i,
            geometry: Some(Geometry::Point(Point::new(
                x0 + lcg(&mut seed) * w,
                y0 + lcg(&mut seed) * h,
            ))),
            properties: BTreeMap::new(),
        });
    }

    let run = |label: &str, op: &str, inputs: Vec<(&str, &Layer)>, params: Value| {
        let inputs: BTreeMap<String, Layer> = inputs
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        let started = Instant::now();
        let output = ops::run(op, &inputs, &params).expect(label);
        let elapsed = started.elapsed().as_secs_f64();
        assert!(
            output.checks.iter().all(|c| c.passed),
            "{label}: {:?}",
            output.checks
        );
        println!(
            "{label:<48} {elapsed:>8.3} s  ({} output features)",
            output.layer.features.len()
        );
    };
    println!("{points} points, {} mesh cells, 200 stations", grid * grid);
    run(
        "spatial_join mesh ← points (count)",
        "spatial_join",
        vec![("target", &mesh), ("join", &pts)],
        json!({"aggregates": [{"op": "count"}]}),
    );
    run(
        "select_by_location points within 300 m of stations",
        "select_by_location",
        vec![("layer", &pts), ("other", &stations)],
        json!({"predicate": "within_distance", "distance": "300 m"}),
    );
    run(
        "distance_to_nearest points → stations",
        "distance_to_nearest",
        vec![("layer", &pts), ("target", &stations)],
        json!({}),
    );
}
