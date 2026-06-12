use duckdb::Connection;

use crate::error::QueryError;

/// Cross-check density values using DuckDB SQL (MVP verification path).
pub fn verify_nagoya_densities(rows: &[(String, u64, f64, f64)]) -> Result<bool, QueryError> {
    let conn = Connection::open_in_memory()
        .map_err(|e| QueryError::DuckDb(e.to_string()))?;

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
}
