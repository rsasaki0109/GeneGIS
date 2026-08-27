//! Reproducible multi-dimensional performance profiles and regression gates.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::DeploymentClass;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerformanceDimension {
    Cpu,
    Gpu,
    Dataset,
    Network,
    Concurrency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricComparator {
    AtMost,
    AtLeast,
}

/// One required metric and its regression budget.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerformanceMetricBudget {
    pub id: String,
    pub dimension: PerformanceDimension,
    pub unit: String,
    pub comparator: MetricComparator,
    pub threshold: f64,
    pub required: bool,
    /// Exact fixture for this metric when it differs from the profile's primary dataset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixture_digest: Option<String>,
}

/// Published deployment/dataset profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerformanceMatrixProfile {
    pub schema_version: String,
    pub id: String,
    pub deployment_class: DeploymentClass,
    pub dataset_id: String,
    pub dataset_digest: String,
    pub build_digest: String,
    pub metrics: Vec<PerformanceMetricBudget>,
}

/// Exact runtime identity attached to every measurement set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerformanceEnvironment {
    pub os: String,
    pub cpu: String,
    pub gpu: Option<String>,
    pub network_profile: String,
    pub logical_concurrency: u32,
    pub build_digest: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum PerformanceMeasurementStatus {
    Measured { value: f64, iterations: u32 },
    NotMeasured { reason: String },
}

/// One metric observation over the profile's pinned fixture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerformanceMeasurement {
    pub metric_id: String,
    pub fixture_digest: String,
    /// Digest of the sealed evidence set used to derive this measurement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_digest: Option<String>,
    pub observed_at: String,
    pub environment: PerformanceEnvironment,
    pub status: PerformanceMeasurementStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerformanceMatrixVerdict {
    Pass,
    Fail,
    Pending,
}

/// Sealed regression-gate outcome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerformanceMatrixReceipt {
    pub schema_version: String,
    pub profile_id: String,
    pub profile_digest: String,
    pub verdict: PerformanceMatrixVerdict,
    pub regressions: Vec<String>,
    pub pending: Vec<String>,
    pub measurements: Vec<PerformanceMeasurement>,
    pub receipt_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PerformanceMatrixError {
    #[error("invalid performance matrix: {0}")]
    Invalid(String),
    #[error("performance matrix serialization failed: {0}")]
    Serialization(String),
}

/// Evaluate exact metric coverage and fail/pending/pass without treating missing data as success.
pub fn evaluate_performance_matrix(
    profile: &PerformanceMatrixProfile,
    measurements: Vec<PerformanceMeasurement>,
) -> Result<PerformanceMatrixReceipt, PerformanceMatrixError> {
    validate_profile(profile)?;
    let profile_digest = digest(profile)?;
    let budgets = profile
        .metrics
        .iter()
        .map(|budget| (budget.id.as_str(), budget))
        .collect::<BTreeMap<_, _>>();
    let measurement_ids = measurements
        .iter()
        .map(|measurement| measurement.metric_id.as_str())
        .collect::<BTreeSet<_>>();
    if measurement_ids.len() != measurements.len()
        || measurement_ids.len() != budgets.len()
        || measurements.iter().any(|measurement| {
            !budgets.contains_key(measurement.metric_id.as_str())
                || measurement.fixture_digest
                    != budget_fixture_digest(profile, budgets[measurement.metric_id.as_str()])
                || measurement
                    .evidence_digest
                    .as_deref()
                    .is_some_and(|digest| !valid_digest(digest))
                || measurement.environment.build_digest != profile.build_digest
                || measurement.observed_at.trim().is_empty()
                || measurement.environment.os.trim().is_empty()
                || measurement.environment.cpu.trim().is_empty()
                || measurement.environment.network_profile.trim().is_empty()
                || measurement.environment.logical_concurrency == 0
        })
    {
        return Err(PerformanceMatrixError::Invalid(
            "measurement coverage, fixture, build, or environment identity is invalid".into(),
        ));
    }
    let mut regressions = Vec::new();
    let mut pending = Vec::new();
    for measurement in &measurements {
        let budget = budgets[measurement.metric_id.as_str()];
        match &measurement.status {
            PerformanceMeasurementStatus::Measured { value, iterations } => {
                if !value.is_finite() || *value < 0.0 || *iterations == 0 {
                    return Err(PerformanceMatrixError::Invalid(
                        "measured value or iteration count is invalid".into(),
                    ));
                }
                let passed = match budget.comparator {
                    MetricComparator::AtMost => *value <= budget.threshold,
                    MetricComparator::AtLeast => *value >= budget.threshold,
                };
                if !passed {
                    regressions.push(measurement.metric_id.clone());
                }
            }
            PerformanceMeasurementStatus::NotMeasured { reason } => {
                if reason.trim().is_empty() {
                    return Err(PerformanceMatrixError::Invalid(
                        "not-measured reason is required".into(),
                    ));
                }
                if budget.required {
                    pending.push(measurement.metric_id.clone());
                }
            }
        }
    }
    regressions.sort();
    pending.sort();
    let verdict = if !regressions.is_empty() {
        PerformanceMatrixVerdict::Fail
    } else if !pending.is_empty() {
        PerformanceMatrixVerdict::Pending
    } else {
        PerformanceMatrixVerdict::Pass
    };
    let mut receipt = PerformanceMatrixReceipt {
        schema_version: "1.0.0".into(),
        profile_id: profile.id.clone(),
        profile_digest,
        verdict,
        regressions,
        pending,
        measurements,
        receipt_digest: String::new(),
    };
    receipt.receipt_digest = receipt_digest(&receipt)?;
    Ok(receipt)
}

fn validate_profile(profile: &PerformanceMatrixProfile) -> Result<(), PerformanceMatrixError> {
    let ids = profile
        .metrics
        .iter()
        .map(|metric| metric.id.as_str())
        .collect::<BTreeSet<_>>();
    let dimensions = profile
        .metrics
        .iter()
        .map(|metric| metric.dimension)
        .collect::<BTreeSet<_>>();
    let required_dimensions = BTreeSet::from([
        PerformanceDimension::Cpu,
        PerformanceDimension::Gpu,
        PerformanceDimension::Dataset,
        PerformanceDimension::Network,
        PerformanceDimension::Concurrency,
    ]);
    if profile.schema_version != "1.0.0"
        || profile.id.trim().is_empty()
        || profile.dataset_id.trim().is_empty()
        || !valid_digest(&profile.dataset_digest)
        || !valid_digest(&profile.build_digest)
        || profile.metrics.is_empty()
        || ids.len() != profile.metrics.len()
        || !required_dimensions.is_subset(&dimensions)
        || profile.metrics.iter().any(|metric| {
            metric.id.trim().is_empty()
                || metric.unit.trim().is_empty()
                || !metric.threshold.is_finite()
                || metric.threshold < 0.0
                || metric
                    .fixture_digest
                    .as_deref()
                    .is_some_and(|digest| !valid_digest(digest))
        })
    {
        return Err(PerformanceMatrixError::Invalid(
            "profile identity, dimension coverage, or budget is invalid".into(),
        ));
    }
    Ok(())
}

fn budget_fixture_digest<'a>(
    profile: &'a PerformanceMatrixProfile,
    budget: &'a PerformanceMetricBudget,
) -> &'a str {
    budget
        .fixture_digest
        .as_deref()
        .unwrap_or(&profile.dataset_digest)
}

fn receipt_digest(receipt: &PerformanceMatrixReceipt) -> Result<String, PerformanceMatrixError> {
    let mut semantic = receipt.clone();
    semantic.receipt_digest.clear();
    digest(&semantic)
}

fn digest<T: Serialize>(value: &T) -> Result<String, PerformanceMatrixError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| PerformanceMatrixError::Serialization(error.to_string()))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn valid_digest(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> PerformanceMatrixProfile {
        serde_json::from_str(include_str!(
            "../../../benchmarks/profiles/local-first-nagoya.json"
        ))
        .expect("profile")
    }
    fn measurements(gpu: PerformanceMeasurementStatus) -> Vec<PerformanceMeasurement> {
        let profile = profile();
        profile
            .metrics
            .iter()
            .map(|budget| PerformanceMeasurement {
                metric_id: budget.id.clone(),
                fixture_digest: budget_fixture_digest(&profile, budget).to_string(),
                evidence_digest: None,
                observed_at: "2026-08-26T10:00:00Z".into(),
                environment: PerformanceEnvironment {
                    os: "test".into(),
                    cpu: "test-cpu".into(),
                    gpu: Some("test-gpu".into()),
                    network_profile: "offline-fixture".into(),
                    logical_concurrency: 4,
                    build_digest: profile.build_digest.clone(),
                },
                status: if budget.dimension == PerformanceDimension::Gpu {
                    gpu.clone()
                } else {
                    PerformanceMeasurementStatus::Measured {
                        value: budget.threshold,
                        iterations: 3,
                    }
                },
            })
            .collect()
    }

    #[test]
    fn gates_all_dimensions_and_never_turns_missing_gpu_green() {
        let profile = profile();
        let pending = evaluate_performance_matrix(
            &profile,
            measurements(PerformanceMeasurementStatus::NotMeasured {
                reason: "hardware receipt required".into(),
            }),
        )
        .expect("pending");
        assert_eq!(pending.verdict, PerformanceMatrixVerdict::Pending);
        assert_eq!(
            pending.pending,
            vec!["gpu.first_frame_p95_ms", "gpu.steady_state_fps"]
        );
        let passed = evaluate_performance_matrix(
            &profile,
            measurements(PerformanceMeasurementStatus::Measured {
                value: 100.0,
                iterations: 3,
            }),
        )
        .expect("pass");
        assert_eq!(passed.verdict, PerformanceMatrixVerdict::Pass);
    }

    #[test]
    fn regression_or_fixture_mutation_fails_closed() {
        let profile = profile();
        let mut values = measurements(PerformanceMeasurementStatus::Measured {
            value: 100.0,
            iterations: 3,
        });
        if let PerformanceMeasurementStatus::Measured { value, .. } = &mut values[0].status {
            *value = profile.metrics[0].threshold + 1.0;
        }
        assert_eq!(
            evaluate_performance_matrix(&profile, values)
                .expect("receipt")
                .verdict,
            PerformanceMatrixVerdict::Fail
        );
        let mut wrong = measurements(PerformanceMeasurementStatus::Measured {
            value: 100.0,
            iterations: 3,
        });
        wrong[0].fixture_digest = format!("sha256:{}", "f".repeat(64));
        assert!(evaluate_performance_matrix(&profile, wrong).is_err());
    }
}
