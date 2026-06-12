//! Query engine — DuckDB local analytics adapter.

pub mod duckdb;
pub mod error;

pub use duckdb::verify_nagoya_densities;
pub use error::QueryError;
