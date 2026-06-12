use thiserror::Error;

#[derive(Debug, Error)]
pub enum QueryError {
    #[error("duckdb error: {0}")]
    DuckDb(String),
}
