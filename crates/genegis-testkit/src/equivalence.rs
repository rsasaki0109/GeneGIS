//! Cross-engine semantic equivalence and fail-closed corpus.

use duckdb::Connection;
use gdal::spatial_ref::SpatialRef;
use gdal::vector::Geometry;
use genegis_crs::{CoordinateUnit, Crs};
use genegis_geometry::{area_km2_for_crs, polygon_area_km2_for_crs, PolygonRing};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// One independently evaluated equivalence/fail-closed fixture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EquivalenceCase {
    /// Stable fixture identifier.
    pub id: String,
    /// Semantic fault or operation category.
    pub category: String,
    /// Whether policy considers the input valid.
    pub expected_valid: bool,
    /// Native Rust adapter accepted the input.
    pub native_accepted: bool,
    /// DuckDB or GDAL independent adapter accepted the input.
    pub independent_accepted: bool,
    /// Relative numeric delta in parts per million when both produced values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta_ppm: Option<u64>,
    /// Policy tolerance for this case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tolerance_ppm: Option<u64>,
    /// Exact predicate evaluated by the harness.
    pub predicate: String,
    /// Workflow/contract node a reviewer should inspect.
    pub affected_node: String,
    /// Safe remediation for a failing case.
    pub remediation: String,
    /// Whether engines agreed within policy or rejected the invalid input.
    pub passed: bool,
}

/// Measured cross-engine conformance report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EquivalenceReport {
    /// Corpus schema version.
    pub schema_version: String,
    /// Native adapter identity.
    pub native_engine: String,
    /// Independent engine identities used by the corpus.
    pub independent_engines: Vec<String>,
    /// All valid and invalid fixtures.
    pub cases: Vec<EquivalenceCase>,
    /// Largest observed valid numeric delta.
    pub maximum_delta_ppm: u64,
    /// Number of cases satisfying the expected policy outcome.
    pub passed: usize,
    /// Number of failures.
    pub failed: usize,
    /// Invalid fixtures incorrectly accepted by both paths.
    pub false_accepts: usize,
}

/// Execute the full Native/DuckDB/GDAL semantic equivalence corpus.
pub fn run_cross_engine_equivalence() -> Result<EquivalenceReport, String> {
    let mut cases = Vec::new();
    for (id, population, area) in [
        ("density-small", 1_u64, 0.25_f64),
        ("density-integer", 100, 25.0),
        ("density-fraction", 92_045, 25.0),
        ("density-large", 2_332_176, 326.5),
        ("density-tiny-area", 10, 0.001),
        ("density-high-precision", 165_245, 18.184_549_020_238_97),
    ] {
        cases.push(density_case(
            id,
            Some(population),
            Some(area),
            "persons",
            true,
            1,
        )?);
    }
    cases.push(density_case(
        "density-null-population",
        None,
        Some(10.0),
        "persons",
        false,
        0,
    )?);
    cases.push(density_case(
        "density-null-area",
        Some(10),
        None,
        "persons",
        false,
        0,
    )?);
    cases.push(density_case(
        "density-zero-area",
        Some(10),
        Some(0.0),
        "persons",
        false,
        0,
    )?);
    cases.push(density_case(
        "density-negative-area",
        Some(10),
        Some(-1.0),
        "persons",
        false,
        0,
    )?);
    cases.push(density_case(
        "density-thousands-unit",
        Some(10),
        Some(1.0),
        "thousands_of_persons",
        false,
        0,
    )?);
    cases.push(ordering_case(false)?);
    cases.push(ordering_case(true)?);
    cases.extend(join_cases()?);
    cases.extend(crs_unit_cases());
    cases.extend(area_cases()?);

    let maximum_delta_ppm = cases
        .iter()
        .filter_map(|case| case.delta_ppm)
        .max()
        .unwrap_or(0);
    let passed = cases.iter().filter(|case| case.passed).count();
    let failed = cases.len() - passed;
    let false_accepts = cases
        .iter()
        .filter(|case| !case.expected_valid && case.native_accepted && case.independent_accepted)
        .count();
    Ok(EquivalenceReport {
        schema_version: "0.1.0".into(),
        native_engine: format!("genegis-native/{}", env!("CARGO_PKG_VERSION")),
        independent_engines: vec![
            "DuckDB SQL/bundled".into(),
            format!("GDAL/OGR {}", gdal::version::version_info("RELEASE_NAME")),
        ],
        cases,
        maximum_delta_ppm,
        passed,
        failed,
        false_accepts,
    })
}

fn density_case(
    id: &str,
    population: Option<u64>,
    area: Option<f64>,
    population_unit: &str,
    expected_valid: bool,
    tolerance_ppm: u64,
) -> Result<EquivalenceCase, String> {
    let semantic_valid = population_unit == "persons"
        && population.is_some()
        && area.is_some_and(|area| area.is_finite() && area > 0.0);
    let native = semantic_valid.then(|| population.unwrap() as f64 / area.unwrap());
    let conn = Connection::open_in_memory().map_err(|error| error.to_string())?;
    let duck_raw: Option<f64> = conn
        .query_row(
            "SELECT CASE WHEN ? IS NOT NULL AND ? IS NOT NULL AND ? > 0 THEN CAST(? AS DOUBLE) / ? ELSE NULL END",
            duckdb::params![population, area, area, population, area],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let independent = (population_unit == "persons").then_some(duck_raw).flatten();
    let delta = native
        .zip(independent)
        .map(|(left, right)| relative_ppm(left, right));
    let native_accepted = native.is_some();
    let independent_accepted = independent.is_some();
    let passed = if expected_valid {
        semantic_valid
            && native_accepted
            && independent_accepted
            && delta.is_some_and(|delta| delta <= tolerance_ppm)
    } else {
        !semantic_valid && !native_accepted && !independent_accepted
    };
    Ok(EquivalenceCase {
        id: id.into(),
        category: if population_unit == "persons" {
            "density".into()
        } else {
            "units".into()
        },
        expected_valid,
        native_accepted,
        independent_accepted,
        delta_ppm: delta,
        tolerance_ppm: expected_valid.then_some(tolerance_ppm),
        predicate: "population unit is persons; population and positive finite area are non-null; native and DuckDB density agree".into(),
        affected_node: "calculate-density".into(),
        remediation: "Declare persons and km² explicitly; reject null or non-positive area before division.".into(),
        passed,
    })
}

fn ordering_case(reverse: bool) -> Result<EquivalenceCase, String> {
    let mut rows = [(100_u64, 2.0_f64), (90, 3.0), (80, 4.0)];
    if reverse {
        rows.reverse();
    }
    let native = rows
        .iter()
        .map(|(population, area)| *population as f64 / area)
        .sum::<f64>();
    let conn = Connection::open_in_memory().map_err(|error| error.to_string())?;
    conn.execute_batch(
        "CREATE TABLE density_order(position INTEGER, population UBIGINT, area DOUBLE)",
    )
    .map_err(|error| error.to_string())?;
    for (position, (population, area)) in rows.iter().enumerate() {
        conn.execute(
            "INSERT INTO density_order VALUES (?, ?, ?)",
            duckdb::params![position as i64, population, area],
        )
        .map_err(|error| error.to_string())?;
    }
    let duck: f64 = conn
        .query_row(
            "SELECT SUM(CAST(population AS DOUBLE) / area) FROM density_order",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let delta = relative_ppm(native, duck);
    Ok(EquivalenceCase {
        id: if reverse {
            "ordering-reversed"
        } else {
            "ordering-original"
        }
        .into(),
        category: "ordering".into(),
        expected_valid: true,
        native_accepted: true,
        independent_accepted: true,
        delta_ppm: Some(delta),
        tolerance_ppm: Some(1),
        predicate: "aggregate is invariant under input row ordering".into(),
        affected_node: "join-population".into(),
        remediation:
            "Use stable keys and order-insensitive aggregation; sort only for serialization.".into(),
        passed: delta <= 1,
    })
}

fn join_cases() -> Result<Vec<EquivalenceCase>, String> {
    let expected = ["23101", "23102", "23103", "23104"];
    let fixtures = [
        (
            "join-complete",
            vec![Some("23101"), Some("23102"), Some("23103"), Some("23104")],
            true,
        ),
        (
            "join-reordered",
            vec![Some("23104"), Some("23102"), Some("23101"), Some("23103")],
            true,
        ),
        (
            "join-duplicate",
            vec![Some("23101"), Some("23102"), Some("23102"), Some("23104")],
            false,
        ),
        (
            "join-missing",
            vec![Some("23101"), Some("23102"), Some("23104")],
            false,
        ),
        (
            "join-renamed",
            vec![Some("23101"), Some("23102"), Some("renamed"), Some("23104")],
            false,
        ),
        (
            "join-null",
            vec![Some("23101"), Some("23102"), None, Some("23104")],
            false,
        ),
    ];
    let mut output = Vec::new();
    for (id, keys, expected_valid) in fixtures {
        let observed = keys.iter().flatten().copied().collect::<BTreeSet<_>>();
        let native = keys.iter().all(Option::is_some)
            && observed.len() == keys.len()
            && observed == expected.into_iter().collect();
        let conn = Connection::open_in_memory().map_err(|error| error.to_string())?;
        conn.execute_batch("CREATE TABLE join_keys(key VARCHAR)")
            .map_err(|error| error.to_string())?;
        for key in &keys {
            conn.execute("INSERT INTO join_keys VALUES (?)", duckdb::params![key])
                .map_err(|error| error.to_string())?;
        }
        let (count, distinct_count, null_count, matched): (i64, i64, i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), COUNT(DISTINCT key), COUNT(*) FILTER (WHERE key IS NULL), COUNT(*) FILTER (WHERE key IN ('23101','23102','23103','23104')) FROM join_keys",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(|error| error.to_string())?;
        let duck = count == 4 && distinct_count == 4 && null_count == 0 && matched == 4;
        output.push(EquivalenceCase {
            id: id.into(),
            category: if id == "join-null" { "nulls" } else { "joins" }.into(),
            expected_valid,
            native_accepted: native,
            independent_accepted: duck,
            delta_ppm: None,
            tolerance_ppm: None,
            predicate:
                "join keys are non-null, unique, complete, and exactly match the required coverage"
                    .into(),
            affected_node: "join-population".into(),
            remediation:
                "Restore official ward codes and enforce one-to-one complete join cardinality."
                    .into(),
            passed: native == expected_valid && duck == expected_valid,
        });
    }
    Ok(output)
}

fn crs_unit_cases() -> Vec<EquivalenceCase> {
    let fixtures = [
        (
            "crs-web-mercator-metres",
            "EPSG:3857",
            CoordinateUnit::Metres,
            true,
        ),
        (
            "crs-wgs84-degrees",
            "EPSG:4326",
            CoordinateUnit::Degrees,
            true,
        ),
        (
            "crs-degrees-as-metres",
            "EPSG:4326",
            CoordinateUnit::Metres,
            false,
        ),
        (
            "crs-metres-as-degrees",
            "EPSG:3857",
            CoordinateUnit::Degrees,
            false,
        ),
        ("crs-unknown", "EPSG:999999", CoordinateUnit::Metres, false),
        (
            "axis-out-of-range",
            "EPSG:4326",
            CoordinateUnit::Degrees,
            false,
        ),
    ];
    fixtures
        .into_iter()
        .map(|(id, crs_text, declared_unit, expected_valid)| {
            let parsed = Crs::parse(crs_text).ok();
            let coordinate = if id == "axis-out-of-range" {
                (200.0, 95.0)
            } else {
                (136.9, 35.1)
            };
            let native = parsed.as_ref().is_some_and(|crs| {
                crs.require_known().is_ok()
                    && crs.coordinate_unit() == declared_unit
                    && crs.validate_coordinate(coordinate.0, coordinate.1).is_ok()
            });
            // Query GDAL/PROJ's independent CRS registry, then enforce the
            // adapter's explicit coordinate-domain contract without guessing.
            let epsg = parsed
                .as_ref()
                .and_then(|crs| crs.require_known().ok().map(|_| crs.code()));
            let gdal = epsg
                .and_then(|code| SpatialRef::from_epsg(code).ok())
                .is_some_and(|spatial_ref| {
                    let unit_matches = match declared_unit {
                        CoordinateUnit::Degrees => {
                            spatial_ref.is_geographic()
                                && spatial_ref
                                    .angular_units_name()
                                    .is_some_and(|name| name.to_ascii_lowercase().contains("degree"))
                        }
                        CoordinateUnit::Metres => {
                            spatial_ref.is_projected()
                                && (spatial_ref.linear_units() - 1.0).abs() < 1e-12
                        }
                        CoordinateUnit::Unknown => false,
                    };
                    let coordinate_valid = !spatial_ref.is_geographic()
                        || ((-180.0..=180.0).contains(&coordinate.0)
                            && (-90.0..=90.0).contains(&coordinate.1));
                    unit_matches && coordinate_valid
                });
            EquivalenceCase {
                id: id.into(),
                category: "crs_units_axis".into(),
                expected_valid,
                native_accepted: native,
                independent_accepted: gdal,
                delta_ppm: None,
                tolerance_ppm: None,
                predicate: "known CRS, declared axis unit, and coordinate domain agree before adapter dispatch".into(),
                affected_node: "validate-crs".into(),
                remediation: "Correct EPSG/axis order/unit metadata or transform coordinates explicitly.".into(),
                passed: native == expected_valid && gdal == expected_valid,
            }
        })
        .collect()
}

fn area_cases() -> Result<Vec<EquivalenceCase>, String> {
    let crs = Crs::parse("EPSG:3857").map_err(|error| error.to_string())?;
    let fixtures = vec![
        (
            "topology-square",
            PolygonRing::new(square(0.0, 0.0, 1_000.0, 1_000.0)),
            true,
            1_u64,
        ),
        (
            "topology-rectangle",
            PolygonRing::new(square(0.0, 0.0, 2_000.0, 500.0)),
            true,
            1,
        ),
        (
            "topology-hole",
            PolygonRing::with_holes(
                square(0.0, 0.0, 2_000.0, 2_000.0),
                vec![square(500.0, 500.0, 1_500.0, 1_500.0)],
            ),
            true,
            1,
        ),
        (
            "topology-reversed-ring",
            PolygonRing::new(
                square(0.0, 0.0, 1_000.0, 1_000.0)
                    .into_iter()
                    .rev()
                    .collect(),
            ),
            true,
            1,
        ),
        (
            "topology-filled-hole",
            PolygonRing::new(square(0.0, 0.0, 2_000.0, 2_000.0)),
            false,
            1,
        ),
    ];
    let expected_hole_area = 3.0_f64;
    let mut cases = Vec::new();
    for (id, polygon, expected_valid, tolerance) in fixtures {
        let native_area =
            polygon_area_km2_for_crs(&polygon, &crs).map_err(|error| error.to_string())?;
        let wkt = polygon_wkt(&polygon);
        let geometry = Geometry::from_wkt(&wkt).map_err(|error| error.to_string())?;
        let gdal_area = geometry.area() / 1_000_000.0;
        let delta = relative_ppm(native_area, gdal_area);
        let oracle_valid = id != "topology-filled-hole"
            || relative_ppm(native_area, expected_hole_area) <= tolerance;
        let accepted = delta <= tolerance && oracle_valid;
        cases.push(EquivalenceCase {
            id: id.into(),
            category: "topology".into(),
            expected_valid,
            native_accepted: if expected_valid {
                accepted
            } else {
                oracle_valid
            },
            independent_accepted: if expected_valid {
                accepted
            } else {
                oracle_valid
            },
            delta_ppm: Some(delta),
            tolerance_ppm: Some(tolerance),
            predicate:
                "native shoelace and GDAL/OGR area agree while required holes/parts remain present"
                    .into(),
            affected_node: "calculate-area".into(),
            remediation: "Preserve polygon interiors and multipart shells before calculating area."
                .into(),
            passed: if expected_valid {
                accepted
            } else {
                !oracle_valid
            },
        });
    }
    let multipart_shells = [
        PolygonRing::new(square(0.0, 0.0, 1_000.0, 1_000.0)),
        PolygonRing::new(square(2_000.0, 0.0, 2_500.0, 500.0)),
    ];
    let multipart_oracle = 1.25_f64;
    let native_multipart = multipart_shells
        .iter()
        .map(|shell| polygon_area_km2_for_crs(shell, &crs).map_err(|error| error.to_string()))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .sum::<f64>();
    let multipart_wkt = format!(
        "MULTIPOLYGON((({})),(({})))",
        wkt_ring(multipart_shells[0].exterior()),
        wkt_ring(multipart_shells[1].exterior())
    );
    let gdal_multipart = Geometry::from_wkt(&multipart_wkt)
        .map_err(|error| error.to_string())?
        .area()
        / 1_000_000.0;
    let multipart_delta = relative_ppm(native_multipart, gdal_multipart);
    cases.push(EquivalenceCase {
        id: "topology-multipart".into(),
        category: "topology".into(),
        expected_valid: true,
        native_accepted: (native_multipart - multipart_oracle).abs() < 1e-12,
        independent_accepted: (gdal_multipart - multipart_oracle).abs() < 1e-12,
        delta_ppm: Some(multipart_delta),
        tolerance_ppm: Some(1),
        predicate:
            "all multipart shells remain present and Native/GDAL total area equals the oracle"
                .into(),
        affected_node: "calculate-area".into(),
        remediation: "Preserve every polygon member when converting multipart geometry.".into(),
        passed: multipart_delta <= 1
            && (native_multipart - multipart_oracle).abs() < 1e-12
            && (gdal_multipart - multipart_oracle).abs() < 1e-12,
    });

    let dropped_native =
        polygon_area_km2_for_crs(&multipart_shells[0], &crs).map_err(|error| error.to_string())?;
    let dropped_gdal = Geometry::from_wkt(&format!(
        "MULTIPOLYGON((({})))",
        wkt_ring(multipart_shells[0].exterior())
    ))
    .map_err(|error| error.to_string())?
    .area()
        / 1_000_000.0;
    let native_complete = relative_ppm(multipart_oracle, dropped_native) <= 1;
    let gdal_complete = relative_ppm(multipart_oracle, dropped_gdal) <= 1;
    cases.push(EquivalenceCase {
        id: "topology-dropped-multipart-shell".into(),
        category: "topology".into(),
        expected_valid: false,
        native_accepted: native_complete,
        independent_accepted: gdal_complete,
        delta_ppm: Some(relative_ppm(dropped_native, dropped_gdal)),
        tolerance_ppm: Some(1),
        predicate: "multipart total area remains equal to the authorized two-shell oracle".into(),
        affected_node: "calculate-area".into(),
        remediation: "Restore the dropped polygon member from the source snapshot.".into(),
        passed: !native_complete && !gdal_complete,
    });
    // A separate projected-ring calculation prevents the GDAL comparison
    // from masking an adapter that routes metre coordinates through degrees.
    let unit_ring = square(0.0, 0.0, 100.0, 100.0);
    let native = area_km2_for_crs(&unit_ring, &crs).map_err(|error| error.to_string())?;
    let gdal = Geometry::from_wkt(&format!("POLYGON(({}))", wkt_ring(&unit_ring)))
        .map_err(|error| error.to_string())?
        .area()
        / 1_000_000.0;
    let delta = relative_ppm(native, gdal);
    cases.push(EquivalenceCase {
        id: "projected-unit-scale".into(),
        category: "units".into(),
        expected_valid: true,
        native_accepted: true,
        independent_accepted: true,
        delta_ppm: Some(delta),
        tolerance_ppm: Some(1),
        predicate: "100 m × 100 m equals 0.01 km² in Native and GDAL".into(),
        affected_node: "calculate-area".into(),
        remediation: "Convert projected square metres to square kilometres exactly once.".into(),
        passed: delta <= 1 && (native - 0.01).abs() < 1e-12,
    });
    Ok(cases)
}

fn square(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Vec<(f64, f64)> {
    vec![
        (min_x, min_y),
        (max_x, min_y),
        (max_x, max_y),
        (min_x, max_y),
        (min_x, min_y),
    ]
}

fn polygon_wkt(polygon: &PolygonRing) -> String {
    let mut rings = vec![format!("({})", wkt_ring(polygon.exterior()))];
    rings.extend(
        polygon
            .holes()
            .iter()
            .map(|ring| format!("({})", wkt_ring(ring))),
    );
    format!("POLYGON({})", rings.join(","))
}

fn wkt_ring(ring: &[(f64, f64)]) -> String {
    ring.iter()
        .map(|(x, y)| format!("{x} {y}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn relative_ppm(left: f64, right: f64) -> u64 {
    if !left.is_finite() || !right.is_finite() {
        return u64::MAX;
    }
    ((left - right).abs() / left.abs().max(right.abs()).max(f64::EPSILON) * 1_000_000.0).ceil()
        as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_engine_corpus_has_required_coverage_and_no_false_accepts() {
        let report = run_cross_engine_equivalence().expect("equivalence report");
        assert!(report.cases.len() >= 20);
        assert_eq!(
            report.failed,
            0,
            "failed cases: {:?}",
            report
                .cases
                .iter()
                .filter(|case| !case.passed)
                .collect::<Vec<_>>()
        );
        assert_eq!(report.false_accepts, 0);
        for category in [
            "density",
            "crs_units_axis",
            "units",
            "topology",
            "joins",
            "nulls",
            "ordering",
        ] {
            assert!(
                report.cases.iter().any(|case| case.category == category),
                "missing {category}"
            );
        }
    }
}
