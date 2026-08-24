use duckdb::Connection;

use crate::error::QueryError;

/// Cross-check density values using DuckDB SQL (MVP verification path).
pub fn verify_nagoya_densities(rows: &[(String, u64, f64, f64)]) -> Result<bool, QueryError> {
    let conn = Connection::open_in_memory().map_err(|e| QueryError::DuckDb(e.to_string()))?;

    conn.execute_batch(
        "CREATE TABLE wards (
            ward_name VARCHAR,
            population UBIGINT,
            area_km2 DOUBLE,
            density DOUBLE
        );",
    )
    .map_err(|e| QueryError::DuckDb(e.to_string()))?;

    for (name, pop, area, density) in rows {
        conn.execute(
            "INSERT INTO wards VALUES (?, ?, ?, ?)",
            duckdb::params![name, pop, area, density],
        )
        .map_err(|e| QueryError::DuckDb(e.to_string()))?;
    }

    let mut stmt = conn
        .prepare(
            "SELECT COUNT(*) FROM wards
             WHERE ABS(density - (population / area_km2)) < 0.01",
        )
        .map_err(|e| QueryError::DuckDb(e.to_string()))?;

    let count: i64 = stmt
        .query_row([], |row| row.get(0))
        .map_err(|e| QueryError::DuckDb(e.to_string()))?;

    Ok(count as usize == rows.len())
}

/// Cross-check flood exposure rows using DuckDB SQL.
///
/// Each row is `(ward_name, exposed_population, population, exposure_rate)`.
/// The verifier recomputes the exposure rate from the population columns and
/// rejects rows outside `[0, 1]`, so a corrupted overlay cannot pass.
pub fn verify_flood_exposure(rows: &[(String, u64, u64, f64)]) -> Result<bool, QueryError> {
    let conn = Connection::open_in_memory().map_err(|e| QueryError::DuckDb(e.to_string()))?;

    conn.execute_batch(
        "CREATE TABLE exposure (
            ward_name VARCHAR,
            exposed_population UBIGINT,
            population UBIGINT,
            exposure_rate DOUBLE
        );",
    )
    .map_err(|e| QueryError::DuckDb(e.to_string()))?;

    for (name, exposed, population, rate) in rows {
        conn.execute(
            "INSERT INTO exposure VALUES (?, ?, ?, ?)",
            duckdb::params![name, exposed, population, rate],
        )
        .map_err(|e| QueryError::DuckDb(e.to_string()))?;
    }

    let mut stmt = conn
        .prepare(
            "SELECT COUNT(*) FROM exposure
             WHERE population > 0
               AND ABS(exposure_rate - (CAST(exposed_population AS DOUBLE) / population)) < 1e-4
               AND exposure_rate >= 0.0
               AND exposure_rate <= 1.0
               AND exposed_population <= population",
        )
        .map_err(|e| QueryError::DuckDb(e.to_string()))?;

    let count: i64 = stmt
        .query_row([], |row| row.get(0))
        .map_err(|e| QueryError::DuckDb(e.to_string()))?;

    Ok(count as usize == rows.len())
}

/// Cross-check index (NDVI) zonal rows using DuckDB SQL.
///
/// Each row is `(ward_name, epoch_label, mean_index)`. The verifier enforces
/// the physical index range `[-1, 1]` on every row.
pub fn verify_index_values(rows: &[(String, String, f64)]) -> Result<bool, QueryError> {
    let conn = Connection::open_in_memory().map_err(|e| QueryError::DuckDb(e.to_string()))?;

    conn.execute_batch(
        "CREATE TABLE index_rows (
            ward_name VARCHAR,
            epoch VARCHAR,
            mean_index DOUBLE
        );",
    )
    .map_err(|e| QueryError::DuckDb(e.to_string()))?;

    for (name, epoch, value) in rows {
        conn.execute(
            "INSERT INTO index_rows VALUES (?, ?, ?)",
            duckdb::params![name, epoch, value],
        )
        .map_err(|e| QueryError::DuckDb(e.to_string()))?;
    }

    let mut stmt = conn
        .prepare(
            "SELECT COUNT(*) FROM index_rows
             WHERE mean_index >= -1.0 AND mean_index <= 1.0",
        )
        .map_err(|e| QueryError::DuckDb(e.to_string()))?;

    let count: i64 = stmt
        .query_row([], |row| row.get(0))
        .map_err(|e| QueryError::DuckDb(e.to_string()))?;

    Ok(count as usize == rows.len())
}

/// Cross-check change-class summaries using DuckDB SQL.
///
/// Each row is `(class, cell_count, mean_delta_m)`. Sign rules mirror the
/// classifier thresholds so a flipped or corrupted class summary cannot pass.
pub fn verify_volume_deltas(rows: &[(String, i64, f64)]) -> Result<bool, QueryError> {
    let conn = Connection::open_in_memory().map_err(|e| QueryError::DuckDb(e.to_string()))?;

    conn.execute_batch(
        "CREATE TABLE change_classes (
            class VARCHAR,
            cells UBIGINT,
            mean_delta DOUBLE
        );",
    )
    .map_err(|e| QueryError::DuckDb(e.to_string()))?;

    for (class, cells, delta) in rows {
        conn.execute(
            "INSERT INTO change_classes VALUES (?, ?, ?)",
            duckdb::params![class, cells, delta],
        )
        .map_err(|e| QueryError::DuckDb(e.to_string()))?;
    }

    let mut stmt = conn
        .prepare(
            "SELECT COUNT(*) FROM change_classes
             WHERE (class = 'building_added'     AND cells > 0 AND mean_delta >= 4.0)
                OR (class = 'building_removed'   AND cells > 0 AND mean_delta <= -4.0)
                OR (class = 'vegetation_growth'  AND cells > 0 AND mean_delta BETWEEN 0.8 AND 4.0)
                OR (class = 'vegetation_removal' AND cells > 0 AND mean_delta BETWEEN -4.0 AND -0.8)
                OR (class IN ('stable', 'subtle_change') AND ABS(mean_delta) < 4.0)",
        )
        .map_err(|e| QueryError::DuckDb(e.to_string()))?;

    let count: i64 = stmt
        .query_row([], |row| row.get(0))
        .map_err(|e| QueryError::DuckDb(e.to_string()))?;

    Ok(count as usize == rows.len())
}

/// Cross-check evacuation routing rows using DuckDB SQL.
///
/// Each row is `(ward_name, baseline_minutes, flooded_minutes)`. The verifier
/// recomputes the flood delay from the two travel-time columns and rejects
/// rows where the flooded route is faster than the clean baseline (a
/// non-negative edge penalty can only lengthen shortest paths).
pub fn verify_evacuation_delays(rows: &[(String, f64, f64)]) -> Result<bool, QueryError> {
    let conn = Connection::open_in_memory().map_err(|e| QueryError::DuckDb(e.to_string()))?;

    conn.execute_batch(
        "CREATE TABLE evacuation (
            ward_name VARCHAR,
            baseline_minutes DOUBLE,
            flooded_minutes DOUBLE
        );",
    )
    .map_err(|e| QueryError::DuckDb(e.to_string()))?;

    for (name, baseline, flooded) in rows {
        conn.execute(
            "INSERT INTO evacuation VALUES (?, ?, ?)",
            duckdb::params![name, baseline, flooded],
        )
        .map_err(|e| QueryError::DuckDb(e.to_string()))?;
    }

    let mut stmt = conn
        .prepare(
            "SELECT COUNT(*) FROM evacuation
             WHERE baseline_minutes >= 0.0
               AND flooded_minutes >= baseline_minutes - 1e-6",
        )
        .map_err(|e| QueryError::DuckDb(e.to_string()))?;

    let count: i64 = stmt
        .query_row([], |row| row.get(0))
        .map_err(|e| QueryError::DuckDb(e.to_string()))?;

    Ok(count as usize == rows.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duckdb_density_check() {
        let rows = vec![
            ("中区".into(), 92045, 25.0, 3681.8),
            ("港区".into(), 144304, 30.0, 4810.133),
        ];
        assert!(verify_nagoya_densities(&rows).expect("duckdb"));
    }

    #[test]
    fn duckdb_flood_exposure_check_accepts_consistent_rows() {
        let rows = vec![
            ("港区".into(), 72_152, 144_304, 0.5),
            ("中区".into(), 0, 92_045, 0.0),
        ];
        assert!(verify_flood_exposure(&rows).expect("duckdb"));
    }

    #[test]
    fn duckdb_flood_exposure_check_rejects_inconsistent_rows() {
        let rows = vec![("港区".into(), 100_000, 144_304, 0.5)];
        assert!(!verify_flood_exposure(&rows).expect("duckdb"));
        let out_of_range = vec![("港区".into(), 200_000, 144_304, 1.385)];
        assert!(!verify_flood_exposure(&out_of_range).expect("duckdb"));
    }

    #[test]
    fn duckdb_evacuation_delays_accept_monotone_rows() {
        let rows = vec![("港区".into(), 8.0, 21.5), ("中区".into(), 0.0, 3.0)];
        assert!(verify_evacuation_delays(&rows).expect("duckdb"));
    }

    #[test]
    fn duckdb_evacuation_delays_reject_negative_delay() {
        let rows = vec![("港区".into(), 20.0, 12.0)];
        assert!(!verify_evacuation_delays(&rows).expect("duckdb"));
        let negative_baseline = vec![("中区".into(), -1.0, 1.0)];
        assert!(!verify_evacuation_delays(&negative_baseline).expect("duckdb"));
    }
}
