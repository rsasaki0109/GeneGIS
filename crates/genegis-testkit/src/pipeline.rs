use genegis_analysis::run_ask_pipeline;

use crate::error::TestkitError;
use crate::harness::{summarize, time_iterations, BenchmarkSample, NORTH_STAR_PROMPT};

/// Benchmark the full north-star ask pipeline (plan → analyze → verify → export).
pub fn benchmark_pipeline(
    warmup: u32,
    iterations: u32,
) -> Result<BenchmarkSample, TestkitError> {
    let durations = time_iterations(warmup, iterations, || {
        let result = run_ask_pipeline(NORTH_STAR_PROMPT)
            .map_err(|err| TestkitError::Pipeline(err.to_string()))?;
        if !result.duckdb_verified {
            return Err(TestkitError::Pipeline(
                "DuckDB verification failed".into(),
            ));
        }
        Ok(())
    })?;

    Ok(summarize("pipeline", warmup, &durations))
}
