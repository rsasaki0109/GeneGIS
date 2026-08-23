//! GeneGIS testkit — reproducible pipeline and render benchmarks.

#![deny(missing_docs)]

mod cloud_io;
mod equivalence;
mod error;
mod external_benchmark;
mod harness;
mod pipeline;
mod real_data;
mod render;
mod review_tasks;
mod trust_ux;

pub use cloud_io::{
    run_cloud_io_benchmark, run_cloud_selected_view_benchmark, run_full_cloud_io_benchmark,
    CloudFormatBenchmark, CloudIoBenchmarkReport, CloudSelectedViewBenchmark,
};
pub use equivalence::{run_cross_engine_equivalence, EquivalenceCase, EquivalenceReport};
pub use error::TestkitError;
pub use external_benchmark::{
    run_external_benchmark, ExternalBenchmarkCase, ExternalBenchmarkReport,
};
pub use harness::{
    time_iterations, BenchmarkReport, BenchmarkSample, DEFAULT_ITERATIONS, DEFAULT_VIEWPORT,
    DEFAULT_WARMUP, NORTH_STAR_PROMPT,
};
pub use pipeline::benchmark_pipeline;
pub use real_data::{run_real_data_corpus, DomainVerification, RealDataCorpusReport};
pub use render::benchmark_render_mesh;
pub use review_tasks::{
    review_median_seconds, review_task_corpus, ReviewTask, ReviewTaskResult, ReviewTimingReport,
};
pub use trust_ux::{
    aggregate_trust_ux_sessions, seal_trust_ux_session, trust_ux_corpus_digest,
    trust_ux_task_corpus, validate_trust_ux_session, TrustUxAggregateReport, TrustUxAnswerChoice,
    TrustUxEvidenceCard, TrustUxSessionKind, TrustUxSessionReport, TrustUxTask, TrustUxTaskResult,
    TRUST_UX_CORPUS_DIGEST, TRUST_UX_CORPUS_VERSION,
};

/// Run all north-star benchmarks and return a combined report.
pub fn run_all_benchmarks(warmup: u32, iterations: u32) -> Result<BenchmarkReport, TestkitError> {
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
