//! GeneGIS testkit — reproducible pipeline and render benchmarks.

#![deny(missing_docs)]

mod error;
mod harness;
mod pipeline;
mod render;

pub use error::TestkitError;
pub use harness::{
    time_iterations, BenchmarkReport, BenchmarkSample, DEFAULT_ITERATIONS, DEFAULT_VIEWPORT,
    DEFAULT_WARMUP, NORTH_STAR_PROMPT,
};
pub use pipeline::benchmark_pipeline;
pub use render::benchmark_render_mesh;

/// Run all north-star benchmarks and return a combined report.
pub fn run_all_benchmarks(
    warmup: u32,
    iterations: u32,
) -> Result<BenchmarkReport, TestkitError> {
    let pipeline = benchmark_pipeline(warmup, iterations)?;
    let render_mesh = benchmark_render_mesh(warmup, iterations)?;
    Ok(BenchmarkReport {
        samples: vec![pipeline, render_mesh],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_benchmark_smoke() {
        let sample = benchmark_pipeline(1, 1).expect("pipeline benchmark");
        assert_eq!(sample.name, "pipeline");
        assert!(sample.median_ns > 0);
    }

    #[test]
    fn render_benchmark_smoke() {
        let sample = benchmark_render_mesh(1, 1).expect("render benchmark");
        assert_eq!(sample.name, "render_mesh");
        assert!(sample.median_ns > 0);
    }

    #[test]
    fn all_benchmarks_smoke() {
        let report = run_all_benchmarks(1, 1).expect("all benchmarks");
        assert_eq!(report.samples.len(), 2);
    }
}
