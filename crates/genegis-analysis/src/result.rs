use chrono::{DateTime, Utc};
use genegis_contract::{TrustAssessment, TrustEvidence, VerificationGraph, VerificationPolicy};
use genegis_core::{InputSnapshot, WorkflowDigest, WorkflowExecutionEvent};
use genegis_crs::{Crs, SourceMetadata};
use genegis_geometry::PolygonRing;
use genegis_style::{ChoroplethStyle, ColorRgba};
use genegis_workflow::GeoWorkflow;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DensityFeature {
    pub ward_code: String,
    pub ward_name: String,
    pub population: u64,
    pub area_km2: f64,
    pub density_per_km2: f64,
    pub rings: Vec<PolygonRing>,
    pub color: ColorRgba,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationCheck {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

/// Build and engine identity recorded with every executed workflow receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineIdentity {
    /// Stable engine/package name.
    pub name: String,
    /// Cargo package version.
    pub version: String,
    /// Optional deployment build identifier.
    pub build: String,
}

impl Default for EngineIdentity {
    fn default() -> Self {
        Self {
            name: "genegis-analysis".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            build: option_env!("GENEGIS_BUILD_ID")
                .unwrap_or(env!("CARGO_PKG_VERSION"))
                .into(),
        }
    }
}

/// Evidence-first receipt for one Command + Workflow Graph execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionReceipt {
    /// Command envelope identity and event time.
    pub command_id: Uuid,
    pub command_timestamp: DateTime<Utc>,
    /// Stable workflow graph identity.
    pub workflow_id: Uuid,
    pub workflow_digest: WorkflowDigest,
    /// Source and named input snapshots authorized by the command.
    #[serde(default)]
    pub source_snapshots: Vec<SourceMetadata>,
    #[serde(default)]
    pub input_snapshots: Vec<InputSnapshot>,
    /// CRS and units used by the operator.
    pub crs: Option<Crs>,
    pub coordinate_unit: String,
    pub value_unit: String,
    pub area_method: String,
    /// Verification evidence attached to the result.
    pub verifier: String,
    pub verification_passed: bool,
    pub checks: Vec<VerificationCheck>,
    /// Versioned release policy executed for this result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_policy: Option<VerificationPolicy>,
    /// Explicit claim/verifier graph bound into execution evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_graph: Option<VerificationGraph>,
    /// Canonical identity of the verification graph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_graph_digest: Option<String>,
    /// Highest trust level derived from policy and execution evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_assessment: Option<TrustAssessment>,
    /// Normalized evidence used to reproduce `trust_assessment` offline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_evidence: Option<TrustEvidence>,
    /// Evidence returned by the workflow executor. Retrieval timestamps are
    /// kept separately in `retrieval_events` and are not part of the stable
    /// result digest.
    #[serde(default)]
    pub evidence: serde_json::Value,
    /// Retrieval/observation events, explicitly separated from source
    /// identity snapshots.
    #[serde(default)]
    pub retrieval_events: Vec<WorkflowExecutionEvent>,
    /// Engine/build identity for replay diagnostics.
    pub engine: EngineIdentity,
    /// Canonical semantic project state digest after dispatch.
    pub state_digest: String,
    /// Canonical output/evidence digest independent of runtime command UUIDs.
    pub result_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReport {
    /// CRS of the input geometry.
    pub crs: String,
    /// Unit of input coordinate axes, derived from the CRS contract.
    #[serde(default)]
    pub coordinate_unit: String,
    /// Unit of the calculated area.
    #[serde(default)]
    pub area_unit: String,
    /// CRS-aware area algorithm used by the operation.
    pub area_method: String,
    /// Unit of the density value.
    pub density_unit: String,
    /// Input source and attribution carried into the result.
    #[serde(default)]
    pub source: SourceMetadata,
    pub checks: Vec<VerificationCheck>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
    pub workflow: GeoWorkflow,
    pub features: Vec<DensityFeature>,
    pub style: ChoroplethStyle,
    pub verification: VerificationReport,
    pub citations: Vec<Citation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Citation {
    pub title: String,
    pub url: String,
    pub license: String,
}

/// Hash the actual analysis output and verification evidence.
///
/// Runtime workflow UUIDs and source retrieval timestamps are deliberately
/// omitted. Geometry rings, styles, all density fields, source identity, and
/// every verification check remain part of the canonical document, so a
/// change to one output value changes the digest.
pub fn canonical_analysis_result_digest(result: &AnalysisResult) -> String {
    let mut value = serde_json::json!({
        "features": result.features,
        "style": result.style,
        "verification": result.verification,
        "citations": result.citations,
    });
    if let Some(features) = value
        .get_mut("features")
        .and_then(serde_json::Value::as_array_mut)
    {
        features.sort_by(|left, right| {
            left.get("ward_code")
                .and_then(serde_json::Value::as_str)
                .cmp(&right.get("ward_code").and_then(serde_json::Value::as_str))
        });
    }
    // JSON parsers may choose the adjacent IEEE-754 value for long decimal
    // spellings. Normalize below sub-millimetre coordinate precision so a
    // portable JSON round-trip retains semantic identity while material
    // numeric changes still alter the digest.
    normalize_float_noise(&mut value);
    strip_runtime_events(&mut value);
    let canonical = canonical_json(&value);
    format!("sha256:{:x}", Sha256::digest(canonical.as_bytes()))
}

fn normalize_float_noise(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Number(number) if number.is_f64() => {
            if let Some(value) = number.as_f64() {
                let normalized = format!("{value:.9}")
                    .parse::<serde_json::Number>()
                    .expect("finite serialized analysis float");
                *number = normalized;
            }
        }
        serde_json::Value::Object(map) => {
            for child in map.values_mut() {
                normalize_float_noise(child);
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                normalize_float_noise(child);
            }
        }
        _ => {}
    }
}

fn strip_runtime_events(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            map.remove("retrieved_at");
            for child in map.values_mut() {
                strip_runtime_events(child);
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                strip_runtime_events(child);
            }
        }
        _ => {}
    }
}

fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            let mut output = String::from("{");
            for (index, (key, child)) in entries.into_iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(&serde_json::to_string(key).expect("JSON key serialization"));
                output.push(':');
                output.push_str(&canonical_json(child));
            }
            output.push('}');
            output
        }
        serde_json::Value::Array(values) => {
            let values = values.iter().map(canonical_json).collect::<Vec<_>>();
            format!("[{}]", values.join(","))
        }
        _ => serde_json::to_string(value).expect("JSON scalar serialization"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nagoya::{default_nagoya_data_path, run_nagoya_population_density};

    #[test]
    fn canonical_digest_is_deterministic_and_covers_actual_output_values() {
        let result = run_nagoya_population_density(default_nagoya_data_path()).expect("analysis");
        let expected = canonical_analysis_result_digest(&result);
        for _ in 0..10 {
            let repeated =
                run_nagoya_population_density(default_nagoya_data_path()).expect("repeat analysis");
            assert_eq!(canonical_analysis_result_digest(&repeated), expected);
        }

        let mut changed = result.clone();
        changed.features[0].population += 1;
        assert_ne!(canonical_analysis_result_digest(&changed), expected);
    }
}
