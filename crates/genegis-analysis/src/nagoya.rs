use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::{DateTime, Utc};
use genegis_catalog::{
    alpha_catalog, nagoya_wards_geojson_path, DatasetRecord, NAGOYA_WARDS_DENSITY_ID,
};
use genegis_contract::{
    CheckRequirement, IndependenceClass, QualityTolerance, VerificationEvidence, VerificationGraph,
    VerificationNode, VerificationPolicy, VerifierIdentity,
};
use genegis_core::{
    WorkflowExecution, WorkflowExecutionContext, WorkflowExecutionError, WorkflowExecutionEvent,
    WorkflowExecutor,
};
use genegis_crs::{ChecksumVerification, Crs, SourceMetadata};
use genegis_geometry::{polygon_parts_area_km2_for_crs, AreaMethod};
use genegis_query::verify_nagoya_densities;
use genegis_style::ChoroplethStyle;
use genegis_vector::{read_geojson_path, read_geoparquet_uri, VectorDataset};
use genegis_workflow::{nagoya_population_density_template, GeoWorkflow, ReviewStatus};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

use crate::error::AnalysisError;
use crate::export::{export_html_map, export_png_map};
use crate::result::{
    canonical_analysis_result_digest, AnalysisResult, Citation, DensityFeature, VerificationCheck,
    VerificationReport,
};

const DENSITY_UNIT: &str = "persons/km²";
const NAGOYA_ORACLE_JSON: &str =
    include_str!("../../../examples/nagoya-population-density/data/nagoya-oracle-2020.json");
#[cfg(test)]
const NAGOYA_SOURCE_MANIFEST_JSON: &str = include_str!(
    "../../../examples/nagoya-population-density/data/nagoya-source-manifest-2020.json"
);

#[derive(Debug, Clone, serde::Deserialize)]
struct NagoyaOracle {
    population_total: u64,
    area_total_km2: f64,
    wards: Vec<NagoyaOracleWard>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct NagoyaOracleWard {
    ward_code: String,
    #[allow(dead_code)]
    ward_name: String,
    population: u64,
    area_km2: f64,
}

fn nagoya_oracle() -> NagoyaOracle {
    serde_json::from_str(NAGOYA_ORACLE_JSON).expect("immutable Nagoya oracle fixture is valid")
}

/// Release policy for the canonical Nagoya proof-carrying result.
pub fn nagoya_verification_policy() -> VerificationPolicy {
    let mut policy = VerificationPolicy::new("genegis.nagoya-density.release-v1");
    policy.required_contracts = BTreeSet::from([
        "nagoya.boundary.2020".into(),
        "nagoya.population.2020".into(),
        "nagoya.population-density.2020".into(),
    ]);
    policy.required_checks = vec![
        check_requirement("source_checksum", IndependenceClass::DomainInvariant, None),
        check_requirement(
            "population_total_oracle",
            IndependenceClass::AuthoritativeExternalOracle,
            Some(0),
        ),
        check_requirement(
            "ward_coverage_oracle",
            IndependenceClass::AuthoritativeExternalOracle,
            Some(0),
        ),
        check_requirement(
            "area_oracle_relative_error",
            IndependenceClass::AuthoritativeExternalOracle,
            Some(5_000),
        ),
        check_requirement(
            "density_oracle",
            IndependenceClass::AuthoritativeExternalOracle,
            Some(5_000),
        ),
    ];
    policy.minimum_artifact_count = 2;
    policy
}

fn check_requirement(
    check_id: &str,
    independence: IndependenceClass,
    max_error_ppm: Option<u64>,
) -> CheckRequirement {
    CheckRequirement {
        check_id: check_id.into(),
        accepted_independence: BTreeSet::from([independence]),
        max_error_ppm,
    }
}

/// Explicit release-claim graph used by the Nagoya executor and receipt.
pub fn nagoya_verification_graph() -> VerificationGraph {
    let oracle = "fixture://nagoya-oracle-2020.json";
    let contracts = vec![
        "nagoya.boundary.2020".into(),
        "nagoya.population.2020".into(),
        "nagoya.population-density.2020".into(),
    ];
    let mut graph = VerificationGraph::new("genegis.nagoya-density.verification-v1");
    graph.nodes = vec![
        verification_node(
            "source_checksum",
            "The executed source bytes match the authorized snapshot",
            contracts.clone(),
            vec!["workflow-input-snapshots".into()],
            IndependenceClass::DomainInvariant,
            None,
            &[],
        ),
        verification_node(
            "population_total_oracle",
            "The ward populations conserve the official 2020 city total",
            contracts.clone(),
            vec![oracle.into()],
            IndependenceClass::AuthoritativeExternalOracle,
            Some(0),
            &["source_checksum"],
        ),
        verification_node(
            "ward_coverage_oracle",
            "Exactly the official 16 wards occur once with matching names",
            contracts.clone(),
            vec![oracle.into()],
            IndependenceClass::AuthoritativeExternalOracle,
            Some(0),
            &["source_checksum"],
        ),
        verification_node(
            "area_oracle_relative_error",
            "Computed ward areas agree with published municipal areas",
            contracts.clone(),
            vec![oracle.into()],
            IndependenceClass::AuthoritativeExternalOracle,
            Some(5_000),
            &["ward_coverage_oracle"],
        ),
        verification_node(
            "density_oracle",
            "Every density agrees with official population divided by published area",
            contracts,
            vec![oracle.into()],
            IndependenceClass::AuthoritativeExternalOracle,
            Some(5_000),
            &["population_total_oracle", "area_oracle_relative_error"],
        ),
    ];
    graph
}

fn verification_node(
    check_id: &str,
    claim: &str,
    subject_contracts: Vec<String>,
    evidence_inputs: Vec<String>,
    independence: IndependenceClass,
    tolerance_ppm: Option<u64>,
    dependencies: &[&str],
) -> VerificationNode {
    VerificationNode {
        check_id: check_id.into(),
        claim: claim.into(),
        subject_contracts,
        evidence_inputs,
        verifier: VerifierIdentity {
            verifier_id: format!("genegis.nagoya.{check_id}.v1"),
            engine: "genegis-analysis".into(),
            implementation: env!("CARGO_PKG_VERSION").into(),
            independence,
        },
        tolerance: tolerance_ppm.map(|max_error_ppm| QualityTolerance {
            metric: check_id.into(),
            max_error_ppm,
        }),
        depends_on: dependencies
            .iter()
            .map(|dependency| (*dependency).into())
            .collect(),
    }
}

/// Structured outcomes for every policy-required Nagoya verification claim.
pub fn nagoya_verification_observations(analysis: &AnalysisResult) -> Vec<VerificationEvidence> {
    let oracle = nagoya_oracle();
    let oracle_digest = digest_bytes(NAGOYA_ORACLE_JSON.as_bytes());
    let source_digest = analysis
        .verification
        .source
        .observed_checksum
        .clone()
        .or_else(|| analysis.verification.source.expected_checksum.clone());
    let graph = nagoya_verification_graph();
    let mut actual_by_code = std::collections::HashMap::new();
    for feature in &analysis.features {
        actual_by_code.insert(feature.ward_code.as_str(), feature);
    }
    let actual_population = analysis
        .features
        .iter()
        .map(|feature| feature.population)
        .sum::<u64>();
    let population_error_ppm =
        relative_error_ppm(actual_population as f64, oracle.population_total as f64);
    let coverage_error = if actual_by_code.len() == oracle.wards.len()
        && oracle.wards.iter().all(|ward| {
            actual_by_code
                .get(ward.ward_code.as_str())
                .is_some_and(|feature| feature.ward_name == ward.ward_name)
        }) {
        0
    } else {
        u64::MAX
    };
    let mut area_error_ppm = 0;
    let mut density_error_ppm = 0;
    for ward in &oracle.wards {
        let Some(actual) = actual_by_code.get(ward.ward_code.as_str()) else {
            area_error_ppm = u64::MAX;
            density_error_ppm = u64::MAX;
            break;
        };
        area_error_ppm = area_error_ppm.max(relative_error_ppm(actual.area_km2, ward.area_km2));
        let expected_density = ward.population as f64 / ward.area_km2;
        density_error_ppm =
            density_error_ppm.max(relative_error_ppm(actual.density_per_km2, expected_density));
    }

    graph
        .nodes
        .into_iter()
        .map(|node| {
            let check = analysis
                .verification
                .checks
                .iter()
                .find(|check| check.name == node.check_id);
            let (observed_error_ppm, evidence_digest) = match node.check_id.as_str() {
                "source_checksum" => (None, source_digest.clone()),
                "population_total_oracle" => {
                    (Some(population_error_ppm), Some(oracle_digest.clone()))
                }
                "ward_coverage_oracle" => (Some(coverage_error), Some(oracle_digest.clone())),
                "area_oracle_relative_error" => (Some(area_error_ppm), Some(oracle_digest.clone())),
                "density_oracle" => (Some(density_error_ppm), Some(oracle_digest.clone())),
                _ => (None, None),
            };
            VerificationEvidence {
                check_id: node.check_id,
                passed: check.is_some_and(|check| check.passed),
                independence: node.verifier.independence,
                observed_error_ppm,
                evidence_digest,
            }
        })
        .collect()
}

fn relative_error_ppm(actual: f64, expected: f64) -> u64 {
    if !actual.is_finite() || !expected.is_finite() || expected.abs() <= f64::EPSILON {
        return u64::MAX;
    }
    let ppm = ((actual - expected).abs() / expected.abs() * 1_000_000.0).ceil();
    if ppm >= u64::MAX as f64 {
        u64::MAX
    } else {
        ppm as u64
    }
}

/// Render artifacts emitted by the verified Nagoya executor. The optional
/// digests are empty when verification fails, which makes it impossible for a
/// caller to accidentally publish a render produced before verification.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NagoyaArtifactDigests {
    pub html_sha256: Option<String>,
    pub png_sha256: Option<String>,
    pub html_bytes: usize,
    pub png_bytes: usize,
}

/// Complete north-star data-plane output. This is serialized through the core
/// execution boundary and is consumed by the pipeline without re-running the
/// analysis, verifier, or renderer.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NagoyaExecutionOutput {
    pub analysis: AnalysisResult,
    pub verification_passed: bool,
    pub html: String,
    pub png_base64: String,
    pub artifact_digests: NagoyaArtifactDigests,
}

pub fn default_nagoya_data_path() -> &'static str {
    nagoya_wards_geojson_path()
}

pub fn default_nagoya_dataset_id() -> &'static str {
    NAGOYA_WARDS_DENSITY_ID
}

/// Build the runtime-resolved Nagoya graph before an executor is invoked.
pub fn nagoya_population_density_workflow_for_dataset(
    record: &DatasetRecord,
) -> Result<GeoWorkflow, AnalysisError> {
    let input_crs = record
        .parsed_crs()
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    input_crs
        .require_known()
        .map_err(|error| AnalysisError::Message(error.to_string()))?;
    build_nagoya_workflow(&input_crs, record.source_metadata())
}

/// Resolve the static north-star template against the exact source snapshot
/// and CRS that the catalog selected. Both the dispatcher and the optimized
/// analysis kernel use this builder so their authorized graph digests cannot
/// drift apart.
fn build_nagoya_workflow(
    input_crs: &Crs,
    source: SourceMetadata,
) -> Result<GeoWorkflow, AnalysisError> {
    let area_method = area_method_for_crs(input_crs);
    let mut workflow = nagoya_population_density_template();

    // Promote the runtime-resolved dataset snapshot into the typed DAG input
    // contracts. The legacy JSON input remains for existing consumers, while
    // graph validation and stable digests use these contracts.
    for contract_name in ["boundary", "population"] {
        if let Some(contract) = workflow
            .input_contracts
            .iter_mut()
            .find(|contract| contract.name == contract_name)
        {
            contract.crs = Some(input_crs.clone());
            contract.coordinate_unit = Some(input_crs.coordinate_unit());
            contract.source_snapshot = Some(source.clone());
            if let Some(geo_contract) = contract.geo_contract.as_mut() {
                if let Some(spatial) = geo_contract.spatial.as_mut() {
                    spatial.crs = Some(input_crs.clone());
                    spatial.coordinate_unit = input_crs.coordinate_unit();
                }
                if let Some(source_contract) = geo_contract.source.as_mut() {
                    source_contract.snapshot = source.clone();
                }
            }
        }
    }
    workflow.inputs.push(serde_json::json!({
        "crs": input_crs.identifier(),
        "coordinate_units": input_crs.coordinate_unit().as_str(),
        "area_crs": input_crs.identifier(),
        "area_method": area_method_name(area_method),
        "area_unit": "km²",
        "density_unit": DENSITY_UNIT,
        "source": source.clone(),
    }));
    workflow.outputs.push(serde_json::json!({
        "crs": input_crs.identifier(),
        "coordinate_units": input_crs.coordinate_unit().as_str(),
        "area_unit": "km²",
        "density_unit": DENSITY_UNIT,
        "area_method": area_method_name(area_method),
        "source": source,
    }));
    workflow.citations = default_citations()
        .iter()
        .map(|citation| genegis_workflow::Citation {
            title: citation.title.clone(),
            url: Some(citation.url.clone()),
            license: Some(citation.license.clone()),
            retrieved_at: None,
        })
        .collect();
    workflow
        .validate()
        .map_err(|error| AnalysisError::Message(format!("invalid workflow graph: {error}")))?;
    Ok(workflow)
}

/// Executor for the north-star graph. The core dispatcher performs all
/// authorization checks before this implementation reads the dataset.
#[derive(Debug, Clone)]
pub struct NagoyaWorkflowExecutor {
    pub dataset_id: String,
}

impl NagoyaWorkflowExecutor {
    pub fn new(dataset_id: impl Into<String>) -> Self {
        Self {
            dataset_id: dataset_id.into(),
        }
    }
}

impl WorkflowExecutor for NagoyaWorkflowExecutor {
    fn execute(
        &self,
        workflow: &GeoWorkflow,
        context: &WorkflowExecutionContext,
    ) -> Result<WorkflowExecution, WorkflowExecutionError> {
        let order = workflow
            .topological_order()
            .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        let order = order
            .iter()
            .map(|node| node.as_str().to_string())
            .collect::<Vec<_>>();
        if order
            .iter()
            .map(String::as_str)
            .ne(NAGOYA_TOPOLOGICAL_ORDER.iter().copied())
        {
            return Err(WorkflowExecutionError::Failed(format!(
                "Nagoya executor received unexpected topological order: {order:?}"
            )));
        }

        // The operator performs the actual source load, CRS-aware area
        // integration, population join, density calculation, and style
        // generation. The graph order above is the authorization order for
        // those concrete stages, not a bookkeeping-only node count.
        let mut analysis = run_nagoya_population_density_for_dataset(&self.dataset_id)
            .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        let executed_digest = analysis
            .workflow
            .stable_digest()
            .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        if executed_digest != context.workflow_digest.as_str() {
            return Err(WorkflowExecutionError::Failed(
                "executor output workflow digest differs from authorized graph".into(),
            ));
        }
        // Keep the serialized result tied to the graph UUID that core
        // authorized. UUID is runtime metadata and does not affect the
        // stable graph/result digests, but it must not point at an unrelated
        // freshly-created graph in the receipt.
        analysis.workflow.id = workflow.id;
        // This verifier is intentionally independent of the analysis kernel's
        // per-feature checks. Rendering is not attempted until it passes.
        let verification_passed = verify_nagoya_analysis(&analysis)
            .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        let (html, png_base64, artifact_digests) = if verification_passed {
            let html = export_html_map(&analysis, "名古屋市 人口密度");
            let png = export_png_map(&analysis, "名古屋市 人口密度")
                .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
            let html_sha256 = digest_bytes(html.as_bytes());
            let png_sha256 = digest_bytes(&png);
            let html_bytes = html.len();
            let png_bytes = png.len();
            (
                html,
                STANDARD.encode(png),
                NagoyaArtifactDigests {
                    html_sha256: Some(html_sha256),
                    png_sha256: Some(png_sha256),
                    html_bytes,
                    png_bytes,
                },
            )
        } else {
            (
                String::new(),
                String::new(),
                NagoyaArtifactDigests {
                    html_sha256: None,
                    png_sha256: None,
                    html_bytes: 0,
                    png_bytes: 0,
                },
            )
        };
        let verification_policy = nagoya_verification_policy();
        let verification_graph = nagoya_verification_graph();
        verification_graph
            .validate_against_policy(&verification_policy)
            .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        let verification_graph_digest = verification_graph
            .stable_digest()
            .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        let evidence = serde_json::json!({
            "topological_order": order,
            "stages": nagoya_stage_evidence(&analysis, &artifact_digests),
            "verification": analysis.verification,
            "verification_policy": verification_policy,
            "verification_graph": verification_graph,
            "verification_graph_digest": verification_graph_digest,
            "verification_passed": verification_passed,
            "feature_count": analysis.features.len(),
            "output_fields": [
                "ward_code", "ward_name", "population", "area_km2",
                "density_per_km2", "rings", "color"
            ],
            "artifact_digests": artifact_digests,
        });
        let output = NagoyaExecutionOutput {
            analysis,
            verification_passed,
            html,
            png_base64,
            artifact_digests,
        };
        let result_digest = canonical_nagoya_execution_digest(&output, &evidence);
        let output = serde_json::to_value(&output)
            .map_err(|error| WorkflowExecutionError::Failed(error.to_string()))?;
        let mut observed_sources = context.source_snapshots.clone();
        for input in &context.input_snapshots {
            if !observed_sources
                .iter()
                .any(|source| same_source_identity(source, &input.source))
            {
                observed_sources.push(input.source.clone());
            }
        }
        let events = observed_sources
            .iter()
            .map(|source| WorkflowExecutionEvent {
                kind: "source_read".into(),
                source_uri: Some(source.uri.clone()),
                observed_at: source_event_time(source, context.command_timestamp),
                details: serde_json::json!({
                    "stable_identity": stable_source_identity(source),
                    "retrieved_at": source.retrieved_at,
                    "checksum_status": source.checksum_status,
                }),
            })
            .collect();
        Ok(WorkflowExecution {
            result_digest,
            output,
            evidence,
            events,
        })
    }
}

const NAGOYA_TOPOLOGICAL_ORDER: &[&str] = &[
    "resolve-place",
    "find-boundary",
    "find-population",
    "load-boundary",
    "load-population",
    "normalize-schema",
    "reproject-for-area",
    "calculate-area-km2",
    "join-population-to-geometry",
    "calculate-density",
    "generate-choropleth",
    "verify-units",
    "render-map",
    "attach-sources",
];

/// Independent verification used by both the executor and the public
/// analysis verifier. It re-checks metadata and recomputes the density rows
/// through the query verifier rather than trusting the kernel's checks.
pub fn verify_nagoya_analysis(analysis: &AnalysisResult) -> Result<bool, AnalysisError> {
    let metadata_valid = Crs::parse(&analysis.verification.crs)
        .ok()
        .and_then(|crs| crs.require_known().ok().map(|_| crs))
        .map(|crs| {
            analysis.verification.coordinate_unit == crs.coordinate_unit().as_str()
                && analysis.verification.area_unit == "km²"
                && analysis.verification.area_method != "planar_wgs84_approx"
                && !analysis.verification.source.uri.trim().is_empty()
                && analysis.verification.source.checksum_status == ChecksumVerification::Verified
        })
        .unwrap_or(false);
    let rows: Vec<(String, u64, f64, f64)> = analysis
        .features
        .iter()
        .map(|feature| {
            (
                feature.ward_name.clone(),
                feature.population,
                feature.area_km2,
                feature.density_per_km2,
            )
        })
        .collect();
    let checks_valid = analysis
        .verification
        .checks
        .iter()
        .all(|check| check.passed);
    verify_nagoya_densities(&rows)
        .map(|verified| verified && metadata_valid && checks_valid)
        .map_err(|error| AnalysisError::Message(error.to_string()))
}

fn nagoya_stage_evidence(
    analysis: &AnalysisResult,
    artifacts: &NagoyaArtifactDigests,
) -> serde_json::Value {
    let area_digest = digest_bytes(
        &analysis
            .features
            .iter()
            .flat_map(|feature| feature.area_km2.to_bits().to_le_bytes())
            .collect::<Vec<_>>(),
    );
    let density_digest = digest_bytes(
        &analysis
            .features
            .iter()
            .flat_map(|feature| feature.density_per_km2.to_bits().to_le_bytes())
            .collect::<Vec<_>>(),
    );
    serde_json::json!({
        "load-boundary": {
            "status": "completed",
            "feature_count": analysis.features.len(),
            "source_uri": analysis.verification.source.uri,
        },
        "load-population": {
            "status": "completed",
            "row_count": analysis.features.len(),
            "population_total": analysis.features.iter().map(|feature| feature.population).sum::<u64>(),
        },
        "calculate-area-km2": {
            "status": "completed",
            "area_digest": area_digest,
        },
        "calculate-density": {
            "status": "completed",
            "density_digest": density_digest,
        },
        "generate-choropleth": {
            "status": "completed",
            "style": analysis.style,
        },
        "verify-units": {
            "status": "completed",
            "checks": analysis.verification.checks,
        },
        "render-map": {
            "status": if artifacts.html_sha256.is_some() { "completed" } else { "blocked" },
            "html_sha256": artifacts.html_sha256,
            "png_sha256": artifacts.png_sha256,
        },
    })
}

pub fn canonical_nagoya_execution_digest(
    output: &NagoyaExecutionOutput,
    evidence: &serde_json::Value,
) -> String {
    let actual_png_digest = if output.png_base64.is_empty() {
        None
    } else {
        // Keep the canonical function infallible for callers that use it as a
        // digest primitive. Invalid base64 is still represented by a digest
        // of the exact serialized payload; the pipeline's typed decoder
        // rejects it before publishing an ask result.
        Some(
            STANDARD
                .decode(&output.png_base64)
                .map(|bytes| digest_bytes(&bytes))
                .unwrap_or_else(|_| digest_bytes(output.png_base64.as_bytes())),
        )
    };
    let mut document = serde_json::json!({
        "analysis_result_digest": canonical_analysis_result_digest(&output.analysis),
        "verification_passed": output.verification_passed,
        "artifact_digests": output.artifact_digests,
        "actual_artifact_digests": {
            "html_sha256": (!output.html.is_empty()).then(|| digest_bytes(output.html.as_bytes())),
            "png_sha256": actual_png_digest,
        },
        "evidence": evidence,
    });
    normalize_float_noise(&mut document);
    strip_runtime_events(&mut document);
    let canonical = canonical_json(&document);
    format!("sha256:{:x}", Sha256::digest(canonical.as_bytes()))
}

fn normalize_float_noise(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Number(number) if number.is_f64() => {
            if let Some(value) = number.as_f64() {
                *number = format!("{value:.9}")
                    .parse::<serde_json::Number>()
                    .expect("finite serialized execution float");
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

fn digest_bytes(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn strip_runtime_events(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            map.remove("retrieved_at");
            map.remove("observed_at");
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

pub fn run_nagoya_population_density_for_dataset(
    dataset_id: &str,
) -> Result<AnalysisResult, AnalysisError> {
    let catalog = alpha_catalog();
    let record = catalog
        .require(dataset_id)
        .map_err(|e| AnalysisError::Message(e.to_string()))?;
    let source = record.source_metadata();
    if record.format.kind == "geoparquet" {
        run_nagoya_population_density_geoparquet_with_source(&record.uri, source)
    } else {
        let dataset = read_geojson_path(&record.uri)?;
        run_nagoya_population_density_from_vector_with_source(dataset, source)
    }
}

pub fn run_nagoya_population_density_from_catalog() -> Result<AnalysisResult, AnalysisError> {
    run_nagoya_population_density_for_dataset(default_nagoya_dataset_id())
}

pub fn run_nagoya_population_density_geoparquet(
    data_path: &str,
) -> Result<AnalysisResult, AnalysisError> {
    let dataset =
        read_geoparquet_uri(data_path).map_err(|err| AnalysisError::Message(err.to_string()))?;
    run_nagoya_population_density_from_vector_with_source(dataset, source_metadata(data_path))
}

pub fn run_nagoya_population_density(data_path: &str) -> Result<AnalysisResult, AnalysisError> {
    let dataset = read_geojson_path(data_path)?;
    run_nagoya_population_density_from_vector_with_source(dataset, source_metadata(data_path))
}

pub fn run_nagoya_population_density_from_vector(
    dataset: VectorDataset,
) -> Result<AnalysisResult, AnalysisError> {
    let source = source_metadata(&dataset.name);
    run_nagoya_population_density_from_vector_with_source(dataset, source)
}

fn run_nagoya_population_density_geoparquet_with_source(
    data_path: &str,
    source: SourceMetadata,
) -> Result<AnalysisResult, AnalysisError> {
    let dataset =
        read_geoparquet_uri(data_path).map_err(|err| AnalysisError::Message(err.to_string()))?;
    run_nagoya_population_density_from_vector_with_source(dataset, source)
}

fn run_nagoya_population_density_from_vector_with_source(
    dataset: VectorDataset,
    source: SourceMetadata,
) -> Result<AnalysisResult, AnalysisError> {
    let input_crs =
        Crs::parse(&dataset.crs).map_err(|err| AnalysisError::Message(err.to_string()))?;
    input_crs
        .require_known()
        .map_err(|err| AnalysisError::Message(err.to_string()))?;

    let mut workflow = build_nagoya_workflow(&input_crs, source.clone())?;
    let mut densities = Vec::new();
    let mut features = Vec::new();

    for feature in &dataset.features {
        let ward_name = feature
            .properties
            .get("ward_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AnalysisError::Message("missing ward_name".into()))?
            .to_string();
        let ward_code = feature
            .properties
            .get("ward_code")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let population = feature
            .properties
            .get("population")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| AnalysisError::Message(format!("missing population for {ward_name}")))?;

        for ring in &feature.rings {
            for (x, y) in ring.exterior() {
                input_crs
                    .validate_coordinate(*x, *y)
                    .map_err(|err| AnalysisError::Message(err.to_string()))?;
            }
            for hole in ring.holes() {
                for (x, y) in hole {
                    input_crs
                        .validate_coordinate(*x, *y)
                        .map_err(|err| AnalysisError::Message(err.to_string()))?;
                }
            }
        }
        let area_km2 = polygon_parts_area_km2_for_crs(&feature.rings, &input_crs)
            .map_err(|err| AnalysisError::Message(err.to_string()))?;
        let density = if area_km2 > 0.0 {
            population as f64 / area_km2
        } else {
            0.0
        };
        densities.push(density);

        features.push(DensityFeature {
            ward_code,
            ward_name,
            population,
            area_km2,
            density_per_km2: density,
            rings: feature.rings.clone(),
            color: genegis_style::ColorRgba::new(0.5, 0.5, 0.5, 1.0),
        });
    }

    let style = ChoroplethStyle::equal_interval("density_per_km2", DENSITY_UNIT, &densities, 5);
    for (feature, density) in features.iter_mut().zip(densities.iter()) {
        feature.color = style.color_for(*density);
    }

    let verification = build_verification(&input_crs, &features, source);
    let citations = default_citations();
    workflow.review_status = if verification.checks.iter().all(|c| c.passed) {
        ReviewStatus::Executed
    } else {
        ReviewStatus::PendingReview
    };

    Ok(AnalysisResult {
        workflow,
        features,
        style,
        verification,
        citations,
    })
}

fn build_verification(
    crs: &Crs,
    features: &[DensityFeature],
    source: SourceMetadata,
) -> VerificationReport {
    let area_method = area_method_for_crs(crs);
    let mut checks = Vec::new();

    checks.push(VerificationCheck {
        name: "crs_declared".into(),
        passed: crs.require_known().is_ok(),
        detail: format!("CRS = {crs}"),
    });

    checks.push(VerificationCheck {
        name: "area_method_recorded".into(),
        passed: area_method != AreaMethod::PlanarWgs84Approx,
        detail: area_method_name(area_method).into(),
    });

    checks.push(VerificationCheck {
        name: "population_positive".into(),
        passed: features.iter().all(|f| f.population > 0),
        detail: format!("{} wards", features.len()),
    });

    checks.push(VerificationCheck {
        name: "density_unit".into(),
        passed: true,
        detail: DENSITY_UNIT.into(),
    });

    checks.push(VerificationCheck {
        name: "coordinate_unit_declared".into(),
        passed: crs.coordinate_unit().as_str() != "unknown",
        detail: crs.coordinate_unit().to_string(),
    });

    checks.push(VerificationCheck {
        name: "feature_count".into(),
        passed: features.len() == 16,
        detail: "Nagoya has 16 wards".into(),
    });

    checks.push(VerificationCheck {
        name: "boundary_source".into(),
        passed: true,
        detail: "国土数値情報 N03 行政区域 (via JapanCityGeoJson)".into(),
    });

    checks.push(VerificationCheck {
        name: "source_uri_declared".into(),
        passed: !source.uri.trim().is_empty(),
        detail: source.uri.clone(),
    });

    checks.push(VerificationCheck {
        name: "source_checksum".into(),
        passed: source.checksum_status == ChecksumVerification::Verified,
        detail: format!(
            "expected={} observed={} ({})",
            source.expected_checksum.as_deref().unwrap_or("unknown"),
            source.observed_checksum.as_deref().unwrap_or("unknown"),
            source.checksum_status
        ),
    });

    let oracle = nagoya_oracle();
    let actual_population_total = features
        .iter()
        .map(|feature| feature.population)
        .sum::<u64>();
    let population_delta = actual_population_total as i128 - oracle.population_total as i128;
    checks.push(VerificationCheck {
        name: "population_total_oracle".into(),
        passed: population_delta == 0,
        detail: format!(
            "actual={} expected={} delta={}",
            actual_population_total, oracle.population_total, population_delta
        ),
    });

    let mut actual_by_code = std::collections::HashMap::new();
    for feature in features {
        actual_by_code
            .entry(feature.ward_code.as_str())
            .or_insert_with(Vec::new)
            .push(feature);
    }
    let coverage_valid = oracle.wards.iter().all(|ward| {
        actual_by_code
            .get(ward.ward_code.as_str())
            .is_some_and(|matches| matches.len() == 1 && matches[0].ward_name == ward.ward_name)
    }) && actual_by_code.len() == oracle.wards.len();
    checks.push(VerificationCheck {
        name: "ward_coverage_oracle".into(),
        passed: coverage_valid,
        detail: format!(
            "actual_unique_codes={} expected_unique_codes={}",
            actual_by_code.len(),
            oracle.wards.len()
        ),
    });

    let mut area_errors = Vec::new();
    let mut density_errors = Vec::new();
    for ward in &oracle.wards {
        let Some(matches) = actual_by_code.get(ward.ward_code.as_str()) else {
            area_errors.push(format!("{}=missing", ward.ward_code));
            density_errors.push(format!("{}=missing", ward.ward_code));
            continue;
        };
        if matches.len() != 1 {
            area_errors.push(format!("{}=duplicate", ward.ward_code));
            density_errors.push(format!("{}=duplicate", ward.ward_code));
            continue;
        }
        let actual = matches[0];
        let area_relative_error =
            (actual.area_km2 - ward.area_km2).abs() / ward.area_km2.max(f64::EPSILON);
        let expected_density = ward.population as f64 / ward.area_km2;
        let density_relative_error =
            (actual.density_per_km2 - expected_density).abs() / expected_density.max(f64::EPSILON);
        area_errors.push(format!(
            "{}={:.6}%",
            ward.ward_code,
            area_relative_error * 100.0
        ));
        density_errors.push(format!(
            "{}={:.6}%",
            ward.ward_code,
            density_relative_error * 100.0
        ));
    }
    let area_total = features.iter().map(|feature| feature.area_km2).sum::<f64>();
    let area_total_relative_error =
        (area_total - oracle.area_total_km2).abs() / oracle.area_total_km2;
    let area_valid = coverage_valid
        && area_total_relative_error <= 0.005
        && oracle.wards.iter().all(|ward| {
            actual_by_code
                .get(ward.ward_code.as_str())
                .and_then(|matches| (matches.len() == 1).then_some(matches[0]))
                .map(|actual| {
                    (actual.area_km2 - ward.area_km2).abs() / ward.area_km2.max(f64::EPSILON)
                        <= 0.005
                })
                .unwrap_or(false)
        });
    checks.push(VerificationCheck {
        name: "area_oracle_relative_error".into(),
        passed: area_valid,
        detail: format!(
            "total_actual={area_total:.6} total_expected={:.2} total_relative_error={:.6}% threshold=0.5%; per_ward={}",
            oracle.area_total_km2,
            area_total_relative_error * 100.0,
            area_errors.join(", ")
        ),
    });
    let density_valid = coverage_valid
        && oracle.wards.iter().all(|ward| {
            actual_by_code
                .get(ward.ward_code.as_str())
                .and_then(|matches| (matches.len() == 1).then_some(matches[0]))
                .map(|actual| {
                    let expected_density = ward.population as f64 / ward.area_km2;
                    (actual.population == ward.population)
                        && (actual.density_per_km2 - expected_density).abs()
                            / expected_density.max(f64::EPSILON)
                            <= 0.005
                })
                .unwrap_or(false)
        });
    checks.push(VerificationCheck {
        name: "density_oracle".into(),
        passed: density_valid,
        detail: format!(
            "formula=population/area_km2 threshold=0.5%; per_ward={}",
            density_errors.join(", ")
        ),
    });

    VerificationReport {
        crs: crs.identifier(),
        coordinate_unit: crs.coordinate_unit().as_str().into(),
        area_unit: "km²".into(),
        area_method: area_method_name(area_method).into(),
        density_unit: DENSITY_UNIT.into(),
        source,
        checks,
    }
}

fn area_method_for_crs(crs: &Crs) -> AreaMethod {
    if crs.is_projected() {
        AreaMethod::PlanarProjected
    } else {
        AreaMethod::EllipsoidalWgs84
    }
}

fn area_method_name(method: AreaMethod) -> &'static str {
    match method {
        AreaMethod::PlanarWgs84Approx => "planar_wgs84_approx",
        AreaMethod::EllipsoidalWgs84 => "ellipsoidal_wgs84",
        AreaMethod::PlanarProjected => "planar_projected",
    }
}

fn source_metadata(uri: &str) -> SourceMetadata {
    SourceMetadata::from_uri(uri, None, None)
}

/// Resolve an adapter-provided retrieval instant for an observation event.
/// Local reads do not invent a source identity timestamp; the command event
/// time is used as the deterministic observation fallback instead.
fn source_event_time(source: &SourceMetadata, fallback: DateTime<Utc>) -> DateTime<Utc> {
    source
        .retrieved_at
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or(fallback)
}

/// Return only the stable source identity for digest/evidence documents.
/// Retrieval time remains on the surrounding observation event.
fn stable_source_identity(source: &SourceMetadata) -> serde_json::Value {
    let mut stable = source.clone();
    stable.retrieved_at = None;
    serde_json::to_value(stable).expect("source metadata is serializable")
}

fn same_source_identity(left: &SourceMetadata, right: &SourceMetadata) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    left.retrieved_at = None;
    right.retrieved_at = None;
    left == right
}

fn default_citations() -> Vec<Citation> {
    vec![
        Citation {
            title: "国土数値情報 行政区域 (N03) — 愛知県 名古屋市区".into(),
            url: "https://nlftp.mlit.go.jp/ksj/gml/datalist/KsjTmplt-N03.html".into(),
            license: "国土交通省 国土数値情報".into(),
        },
        Citation {
            title: "JapanCityGeoJson — N03 derived ward boundaries".into(),
            url: "https://github.com/niiyz/JapanCityGeoJson".into(),
            license: "Processed from MLIT N03 open data".into(),
        },
        Citation {
            title: "名古屋市 — 令和2年国勢調査 確定値（区別人口）".into(),
            url: "https://www.city.nagoya.jp/shisei/toukei/1003703/1003773/1003809/1034253/1003818.html".into(),
            license: "名古屋市オープンデータ利用規約（政府標準利用規約2.0準拠）".into(),
        },
        Citation {
            title: "名古屋市 — 令和2年国勢調査 統計表 Excel".into(),
            url: "https://www.city.nagoya.jp/_res/projects/default_project/_page_/001/003/818/toukeihyo.xlsx".into(),
            license: "名古屋市オープンデータ利用規約（政府標準利用規約2.0準拠）".into(),
        },
        Citation {
            title: "名古屋市オープンデータカタログ".into(),
            url: "https://www.data-nagoya.jp/".into(),
            license: "City of Nagoya Open Data".into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use genegis_core::{InputSnapshot, WorkflowDigest};

    #[test]
    fn runs_nagoya_demo() {
        let result = run_nagoya_population_density(default_nagoya_data_path()).expect("run");
        assert_eq!(result.features.len(), 16);
        assert!(result.verification.checks.iter().all(|c| c.passed));
        assert!(result.features[0].density_per_km2 > 0.0);
        assert_eq!(result.verification.crs, "EPSG:4326");
        assert_eq!(result.verification.coordinate_unit, "degrees");
        assert_eq!(result.verification.area_unit, "km²");
        assert_eq!(result.verification.area_method, "ellipsoidal_wgs84");
        assert_eq!(
            result.verification.source.uri,
            default_nagoya_data_path().to_string()
        );
        assert_eq!(
            result.verification.source.checksum_status,
            ChecksumVerification::Verified
        );
        assert_eq!(
            result
                .verification
                .source
                .source_version
                .as_ref()
                .map(|v| v.as_str()),
            None
        );
        assert!(result.verification.source.retrieved_at.is_none());
    }

    #[test]
    fn matches_immutable_nagoya_population_and_area_oracle() {
        let result = run_nagoya_population_density(default_nagoya_data_path()).expect("run");
        assert!(result.verification.checks.iter().all(|check| check.passed));
        let population_total = result
            .features
            .iter()
            .map(|feature| feature.population)
            .sum::<u64>();
        assert_eq!(population_total, 2_332_176);
        assert_eq!(result.features.len(), 16);
        let expected_areas = [
            ("23101", 18.18),
            ("23102", 7.71),
            ("23103", 17.53),
            ("23104", 17.93),
            ("23105", 16.30),
            ("23106", 9.38),
            ("23107", 10.94),
            ("23108", 11.22),
            ("23109", 8.20),
            ("23110", 32.02),
            ("23111", 45.69),
            ("23112", 18.46),
            ("23113", 34.01),
            ("23114", 37.91),
            ("23115", 19.45),
            ("23116", 21.58),
        ];
        for (code, expected) in expected_areas {
            let feature = result
                .features
                .iter()
                .find(|feature| feature.ward_code == code)
                .expect("oracle ward");
            let relative_error = (feature.area_km2 - expected).abs() / expected;
            assert!(
                relative_error <= 0.005,
                "{code} area={} expected={expected} relative_error={relative_error}",
                feature.area_km2
            );
        }
        assert_eq!(
            result
                .verification
                .checks
                .iter()
                .filter(|check| check.name == "density_oracle")
                .count(),
            1
        );
        assert!(result.citations.iter().any(|citation| {
            citation.url
                == "https://www.city.nagoya.jp/shisei/toukei/1003703/1003773/1003809/1034253/1003818.html"
        }));
        assert!(result.citations.iter().any(|citation| {
            citation.url
                == "https://www.city.nagoya.jp/_res/projects/default_project/_page_/001/003/818/toukeihyo.xlsx"
        }));
    }

    #[test]
    fn immutable_source_manifest_matches_fixture_bytes() {
        let manifest: serde_json::Value =
            serde_json::from_str(NAGOYA_SOURCE_MANIFEST_JSON).expect("source manifest");
        assert_eq!(manifest["immutable"], true);
        let population_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/nagoya-population-density/data/nagoya-population-2020.json"
        );
        let population = SourceMetadata::from_uri(population_path, None, None);
        assert_eq!(
            manifest["population"]["sha256"],
            population.observed_checksum.as_deref().unwrap()
        );
        let boundary = SourceMetadata::from_uri(default_nagoya_data_path(), None, None);
        assert_eq!(
            manifest["boundary"]["sha256"],
            boundary.observed_checksum.as_deref().unwrap()
        );
        assert_eq!(manifest["oracle"]["area_total_km2"], 326.50);
        assert_eq!(manifest["integrity"]["required_ward_count"], 16);
    }

    #[test]
    fn catalog_source_snapshot_reaches_workflow_input_and_output() {
        let result = run_nagoya_population_density_from_catalog().expect("run");
        assert_eq!(
            result
                .verification
                .source
                .source_version
                .as_ref()
                .map(|v| v.as_str()),
            Some("nagoya-2020-census-final-n03-v2")
        );
        let source_input = result
            .workflow
            .inputs
            .last()
            .and_then(|value| value.get("source"))
            .expect("source workflow input");
        assert_eq!(source_input["checksum_status"], "verified");
        assert_eq!(
            source_input["source_version"],
            "nagoya-2020-census-final-n03-v2"
        );
        assert_eq!(
            source_input["expected_checksum"],
            "sha256:d0f8958813fe28e9428169ca7c638a0ea3b3ed7ae526750156d3f94e1308d30e"
        );
        assert_eq!(
            source_input["observed_checksum"],
            source_input["expected_checksum"]
        );
        let source_output = result
            .workflow
            .outputs
            .last()
            .and_then(|value| value.get("source"))
            .expect("source workflow output");
        assert_eq!(source_output["checksum_status"], "verified");
        assert_eq!(
            source_output["source_version"],
            "nagoya-2020-census-final-n03-v2"
        );
        assert_eq!(
            source_output["expected_checksum"],
            source_input["expected_checksum"]
        );
        assert_eq!(
            source_output["observed_checksum"],
            source_input["observed_checksum"]
        );
    }

    #[test]
    fn rejects_unknown_crs_before_calculating_density() {
        let mut dataset = read_geojson_path(default_nagoya_data_path()).expect("dataset");
        dataset.crs = "EPSG:999999".into();
        let error = run_nagoya_population_density_from_vector(dataset).expect_err("unknown CRS");
        assert!(error.to_string().contains("unsupported CRS"));
    }

    #[test]
    fn fixture_preserves_all_ward_parts_and_property_join_keys() {
        let dataset = read_geojson_path(default_nagoya_data_path()).expect("fixture");
        assert_eq!(dataset.features.len(), 16);
        for feature in &dataset.features {
            let code = feature
                .properties
                .get("ward_code")
                .and_then(|value| value.as_str())
                .expect("ward code");
            let name = feature
                .properties
                .get("ward_name")
                .and_then(|value| value.as_str())
                .expect("ward name");
            assert!(!code.is_empty());
            assert!(!name.is_empty());
            assert!(!feature.rings.is_empty());
        }
        let minato = dataset
            .features
            .iter()
            .find(|feature| feature.properties["ward_code"] == "23111")
            .expect("Minato ward");
        assert_eq!(minato.rings.len(), 4, "all source polygon parts retained");
        assert!(minato.rings.iter().all(|ring| ring.holes().is_empty()));
    }

    #[test]
    fn rejects_missing_population_join_property() {
        let mut dataset = read_geojson_path(default_nagoya_data_path()).expect("fixture");
        dataset.features[0].properties["population"] = serde_json::Value::Null;
        let error = run_nagoya_population_density_from_vector(dataset)
            .expect_err("missing population must fail closed");
        assert!(error.to_string().contains("missing population"));
    }

    #[test]
    fn runs_nagoya_geoparquet_density() {
        let path = genegis_catalog::nagoya_wards_geoparquet_path();
        if !std::path::Path::new(path).exists() {
            return;
        }
        let result = run_nagoya_population_density_geoparquet(path).expect("run");
        assert_eq!(result.features.len(), 16);
        assert!(result.verification.checks.iter().all(|c| c.passed));
    }

    #[test]
    fn geoparquet_dataset_id_resolves_to_parquet_path() {
        assert_eq!(
            alpha_catalog()
                .require(genegis_catalog::NAGOYA_WARDS_GEOPARQUET_ID)
                .expect("record")
                .format
                .kind,
            "geoparquet"
        );
    }

    #[test]
    fn retrieval_event_is_separate_from_stable_source_and_result_identity() {
        let record = alpha_catalog()
            .require(default_nagoya_dataset_id())
            .expect("catalog record")
            .clone();
        let workflow = nagoya_population_density_workflow_for_dataset(&record).expect("workflow");
        let digest = WorkflowDigest::new(workflow.stable_digest().expect("digest"));
        let mut input_snapshots = Vec::new();
        for contract in &workflow.input_contracts {
            if let Some(source) = &contract.source_snapshot {
                input_snapshots.push(InputSnapshot::new(contract.name.clone(), source.clone()));
            }
        }
        let retrieved_at = "2026-08-23T01:02:03Z";
        let source = record.source_metadata().with_retrieved_at(retrieved_at);
        let context = WorkflowExecutionContext {
            workflow_digest: digest,
            command_id: uuid::Uuid::nil(),
            command_timestamp: DateTime::parse_from_rfc3339("2026-08-23T00:00:00Z")
                .expect("timestamp")
                .with_timezone(&Utc),
            source_snapshots: vec![source],
            input_snapshots,
        };
        let execution = NagoyaWorkflowExecutor::new(record.id.clone())
            .execute(&workflow, &context)
            .expect("execute");
        assert_eq!(
            execution.events[0].observed_at,
            DateTime::parse_from_rfc3339(retrieved_at)
                .expect("retrieval timestamp")
                .with_timezone(&Utc)
        );
        assert!(execution.events[0].details["stable_identity"]["retrieved_at"].is_null());
        let mut later_context = context.clone();
        later_context.source_snapshots[0].retrieved_at = Some("2026-08-23T02:03:04Z".into());
        let later_execution = NagoyaWorkflowExecutor::new(record.id.clone())
            .execute(&workflow, &later_context)
            .expect("execute with later observation");
        assert_ne!(
            execution.events[0].observed_at,
            later_execution.events[0].observed_at
        );
        assert_eq!(execution.result_digest, later_execution.result_digest);
        let output: NagoyaExecutionOutput =
            serde_json::from_value(execution.output.clone()).expect("typed output");
        assert_eq!(
            execution.result_digest,
            canonical_nagoya_execution_digest(&output, &execution.evidence)
        );
        let mut changed = output.clone();
        changed.artifact_digests.png_sha256 = Some("sha256:tampered".into());
        assert_ne!(
            canonical_nagoya_execution_digest(&changed, &execution.evidence),
            execution.result_digest
        );
        let mut changed_render = output;
        changed_render.html.push_str("<!-- tampered -->");
        assert_ne!(
            canonical_nagoya_execution_digest(&changed_render, &execution.evidence),
            execution.result_digest
        );
    }
}
