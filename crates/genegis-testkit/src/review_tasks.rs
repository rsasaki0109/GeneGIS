//! Human root-cause review task corpus and timing report types.

use serde::{Deserialize, Serialize};

/// One seeded no-raw-JSON root-cause identification task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewTask {
    /// Stable task identity.
    pub task_id: String,
    /// Machine-readable failure predicate.
    pub failure_code: String,
    /// Affected evidence subject.
    pub subject: String,
    /// Reviewer-facing evidence failure.
    pub detail: String,
    /// Workflow nodes offered as possible root causes.
    pub choices: Vec<String>,
    /// Node the task oracle expects the reviewer to select.
    pub expected_node: String,
}

/// Timed response to one review task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewTaskResult {
    /// Stable task identity.
    pub task_id: String,
    /// Selected Workflow node.
    pub selected_node: String,
    /// Expected Workflow node.
    pub expected_node: String,
    /// Wall-clock review duration.
    pub elapsed_seconds: f64,
    /// Whether the selected root cause was correct.
    pub correct: bool,
}

/// Human review timing evidence for the Phase-11 Gate B metric.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewTimingReport {
    /// Report schema version.
    pub schema_version: String,
    /// Reviewer identity supplied before the session.
    pub reviewer: String,
    /// Binary/scorer identity.
    pub runner_identity: String,
    /// Exact task-corpus version.
    pub corpus_version: String,
    /// Per-task answers and durations.
    pub results: Vec<ReviewTaskResult>,
    /// Median duration across all tasks.
    pub median_seconds: f64,
    /// Number of correct root causes.
    pub correct: usize,
    /// Total task count.
    pub total: usize,
    /// Gate result: all answers correct and median at most 120 seconds.
    pub passed: bool,
}

/// Return the fixed Phase-11 failure diagnosis corpus.
pub fn review_task_corpus() -> Vec<ReviewTask> {
    let choices = [
        "load-boundary",
        "load-population",
        "normalize-schema",
        "calculate-area-km2",
        "join-population-to-geometry",
        "calculate-density",
        "render-map",
    ];
    [
        (
            "source-drift",
            "source_not_verified",
            "nagoya-boundary-2020",
            "Executed boundary bytes differ from the authorized source snapshot.",
            "load-boundary",
        ),
        (
            "population-total",
            "check_failed",
            "population_total_oracle",
            "The joined ward populations do not conserve the official 2020 city total.",
            "join-population-to-geometry",
        ),
        (
            "ward-coverage",
            "check_failed",
            "ward_coverage_oracle",
            "One official ward key is missing and another occurs twice after normalization.",
            "normalize-schema",
        ),
        (
            "area-tolerance",
            "check_tolerance_exceeded",
            "area_oracle_relative_error",
            "Observed ward area error is 8,200 ppm; policy permits 5,000 ppm.",
            "calculate-area-km2",
        ),
        (
            "density-tolerance",
            "check_tolerance_exceeded",
            "density_oracle",
            "Population and area pass independently, but derived density exceeds tolerance.",
            "calculate-density",
        ),
        (
            "render-divergence",
            "artifact_result_mismatch",
            "artifacts/map.html",
            "The numeric result contains 16 wards while the rendered artifact exposes 15.",
            "render-map",
        ),
    ]
    .into_iter()
    .map(
        |(task_id, failure_code, subject, detail, expected_node)| ReviewTask {
            task_id: task_id.into(),
            failure_code: failure_code.into(),
            subject: subject.into(),
            detail: detail.into(),
            choices: choices.iter().map(|choice| (*choice).into()).collect(),
            expected_node: expected_node.into(),
        },
    )
    .collect()
}

/// Compute the median for a non-empty set of task durations.
pub fn review_median_seconds(results: &[ReviewTaskResult]) -> f64 {
    let mut values = results
        .iter()
        .map(|result| result.elapsed_seconds)
        .collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len() % 2 == 0 {
        (values[middle - 1] + values[middle]) / 2.0
    } else {
        values[middle]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_corpus_has_resolvable_distinct_failures() {
        let tasks = review_task_corpus();
        assert!(tasks.len() >= 5);
        for task in tasks {
            assert!(task.choices.contains(&task.expected_node));
            assert!(!task.detail.contains('{'));
        }
    }
}
