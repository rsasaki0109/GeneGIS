//! Strict artifact scoring for a legally reusable external GIS-agent task.

use crate::{TestkitError, NORTH_STAR_PROMPT};
use genegis_analysis::{
    canonical_analysis_result_digest, run_ask_pipeline, AnalysisResult, DensityFeature,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

const SOURCE_URL: &str = "https://github.com/solirinai/geobenchx";
const SOURCE_REVISION: &str = "bb3cd88f6834dee8004a2add6f9f0c150053788d";
const SOURCE_TASK: &str = "TASK_250309_135125_908870";
const SOURCE_PROMPT: &str = "Visualize total population distribution by country.";
const TOLERANCE_PPM: u64 = 5_000;

/// Result of independently scoring one candidate spatial artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalBenchmarkCase {
    /// Stable scoring-fixture identity.
    pub case_id: String,
    /// Whether the fixture is intended to satisfy the output contract.
    pub expected_valid: bool,
    /// Whether strict scoring accepted the candidate.
    pub accepted: bool,
    /// Candidate content identity.
    pub candidate_digest: String,
    /// Machine-readable failed predicates.
    pub failures: Vec<String>,
    /// Whether actual and expected outcomes agree.
    pub passed: bool,
}

/// Reproducible report for the GeoBenchX-derived Nagoya adapter slice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalBenchmarkReport {
    /// Adapter schema version.
    pub schema_version: String,
    /// External benchmark and repository.
    pub benchmark: String,
    /// Upstream license governing the reused task metadata.
    pub license: String,
    /// Exact upstream revision inspected by this adapter.
    pub source_revision: String,
    /// Exact upstream task identity.
    pub source_task_id: String,
    /// Exact upstream task prompt.
    pub source_prompt: String,
    /// Geographic recast used locally; this is not an official GeoBenchX score.
    pub adaptation: String,
    /// Agent/executor identity whose artifact was scored.
    pub runner_identity: String,
    /// Independent scorer identity.
    pub scorer_identity: String,
    /// Expected artifact content identity.
    pub expected_digest: String,
    /// Numeric tolerance applied to area and density fields.
    pub tolerance_ppm: u64,
    /// End-to-end local execution and scoring duration.
    pub elapsed_ms: u64,
    /// Strict valid and invalid scoring fixtures.
    pub cases: Vec<ExternalBenchmarkCase>,
    /// Count of correctly classified cases.
    pub passed: usize,
    /// Invalid artifacts incorrectly accepted.
    pub false_accepts: usize,
}

/// Execute a GeoBenchX-derived task recast to Nagoya and score real artifacts.
///
/// GeoBenchX's MIT-licensed task asks for a population choropleth using load,
/// merge, and visualization operations. The local recast retains that intent
/// but changes the geography and adds a strict output contract. Consequently
/// this report evaluates adapter/scorer behavior and must not be compared with
/// the official GeoBenchX leaderboard.
pub fn run_external_benchmark() -> Result<ExternalBenchmarkReport, TestkitError> {
    let started = Instant::now();
    let run = run_ask_pipeline(NORTH_STAR_PROMPT)
        .map_err(|error| TestkitError::Pipeline(error.to_string()))?;
    let expected = run
        .analysis
        .ok_or_else(|| TestkitError::Pipeline("north-star run returned no analysis".into()))?;
    let expected_digest = canonical_analysis_result_digest(&expected);

    let mut fixtures = Vec::new();
    fixtures.push(("exact", true, expected.clone()));

    let mut reordered = expected.clone();
    reordered.features.reverse();
    fixtures.push(("row-order-independent", true, reordered));

    let mut within_tolerance = expected.clone();
    within_tolerance.features[0].density_per_km2 *= 1.004;
    fixtures.push(("density-within-tolerance", true, within_tolerance));

    let mut missing_ward = expected.clone();
    missing_ward.features.pop();
    fixtures.push(("missing-row", false, missing_ward));

    let mut density_outside = expected.clone();
    density_outside.features[0].density_per_km2 *= 1.006;
    fixtures.push(("density-outside-tolerance", false, density_outside));

    let mut geometry_changed = expected.clone();
    geometry_changed.features[0].rings[0].coords[0].0 += 0.001;
    fixtures.push(("geometry-changed", false, geometry_changed));

    let mut wrong_crs = expected.clone();
    wrong_crs.verification.crs = "EPSG:3857".into();
    fixtures.push(("wrong-crs", false, wrong_crs));

    let cases = fixtures
        .into_iter()
        .map(|(case_id, expected_valid, candidate)| {
            let failures = score_artifact(&expected, &candidate);
            let accepted = failures.is_empty();
            ExternalBenchmarkCase {
                case_id: case_id.into(),
                expected_valid,
                accepted,
                candidate_digest: canonical_analysis_result_digest(&candidate),
                passed: accepted == expected_valid,
                failures,
            }
        })
        .collect::<Vec<_>>();
    let passed = cases.iter().filter(|case| case.passed).count();
    let false_accepts = cases
        .iter()
        .filter(|case| !case.expected_valid && case.accepted)
        .count();

    Ok(ExternalBenchmarkReport {
        schema_version: "0.1.0".into(),
        benchmark: format!("GeoBenchX adapter slice ({SOURCE_URL})"),
        license: "MIT; Copyright (c) 2025 Varvara Krechetova".into(),
        source_revision: SOURCE_REVISION.into(),
        source_task_id: SOURCE_TASK.into(),
        source_prompt: SOURCE_PROMPT.into(),
        adaptation: "Country population choropleth recast to Nagoya ward population-density choropleth; not an official GeoBenchX score".into(),
        runner_identity: format!("genegis-analysis/{} rule-planner", env!("CARGO_PKG_VERSION")),
        scorer_identity: format!("genegis-testkit/{} strict-artifact-v1", env!("CARGO_PKG_VERSION")),
        expected_digest,
        tolerance_ppm: TOLERANCE_PPM,
        elapsed_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
        cases,
        passed,
        false_accepts,
    })
}

fn score_artifact(expected: &AnalysisResult, candidate: &AnalysisResult) -> Vec<String> {
    let mut failures = Vec::new();
    if candidate.verification.crs != expected.verification.crs {
        failures.push("crs_mismatch".into());
    }
    if candidate.verification.area_unit != expected.verification.area_unit
        || candidate.verification.density_unit != expected.verification.density_unit
    {
        failures.push("unit_mismatch".into());
    }

    let expected_rows = index_features(&expected.features, &mut failures, "expected");
    let candidate_rows = index_features(&candidate.features, &mut failures, "candidate");
    let expected_keys = expected_rows.keys().copied().collect::<BTreeSet<_>>();
    let candidate_keys = candidate_rows.keys().copied().collect::<BTreeSet<_>>();
    if expected_keys != candidate_keys {
        failures.push("row_set_mismatch".into());
    }
    for code in expected_keys.intersection(&candidate_keys) {
        let expected = expected_rows[code];
        let candidate = candidate_rows[code];
        if expected.ward_name != candidate.ward_name || expected.population != candidate.population
        {
            failures.push(format!("attribute_mismatch:{code}"));
        }
        if relative_error_ppm(expected.area_km2, candidate.area_km2) > TOLERANCE_PPM {
            failures.push(format!("area_tolerance:{code}"));
        }
        if relative_error_ppm(expected.density_per_km2, candidate.density_per_km2) > TOLERANCE_PPM {
            failures.push(format!("density_tolerance:{code}"));
        }
        if serde_json::to_value(&expected.rings).ok() != serde_json::to_value(&candidate.rings).ok()
        {
            failures.push(format!("geometry_mismatch:{code}"));
        }
    }
    failures.sort();
    failures.dedup();
    failures
}

fn index_features<'a>(
    features: &'a [DensityFeature],
    failures: &mut Vec<String>,
    subject: &str,
) -> BTreeMap<&'a str, &'a DensityFeature> {
    let mut indexed = BTreeMap::new();
    for feature in features {
        if indexed
            .insert(feature.ward_code.as_str(), feature)
            .is_some()
        {
            failures.push(format!("duplicate_key:{subject}:{}", feature.ward_code));
        }
    }
    indexed
}

fn relative_error_ppm(expected: f64, actual: f64) -> u64 {
    if !expected.is_finite() || !actual.is_finite() || expected == 0.0 {
        return if expected == actual { 0 } else { u64::MAX };
    }
    (((actual - expected).abs() / expected.abs()) * 1_000_000.0).round() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_adapter_scores_artifacts_and_rejects_faults() {
        let report = run_external_benchmark().expect("external benchmark");
        assert_eq!(report.cases.len(), 7);
        assert_eq!(report.passed, 7);
        assert_eq!(report.false_accepts, 0);
        assert!(report.cases.iter().any(|case| !case.expected_valid));
    }
}
