//! Query engine — DuckDB local analytics adapter.

pub mod duckdb;
pub mod error;

pub use duckdb::{
    verify_evacuation_delays, verify_flood_exposure, verify_index_values, verify_nagoya_densities,
    verify_volume_deltas,
};
pub use error::QueryError;
