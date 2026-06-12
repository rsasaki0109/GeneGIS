use thiserror::Error;

/// Errors raised by the benchmark harness.
#[derive(Debug, Error)]
pub enum TestkitError {
    /// The north-star ask pipeline benchmark failed.
    #[error("pipeline benchmark failed: {0}")]
    Pipeline(String),
    /// The choropleth mesh render benchmark failed.
    #[error("render benchmark failed: {0}")]
    Render(String),
}
