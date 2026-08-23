//! Versioned semantic contracts for geospatial workflow values.
//!
//! A [`GeoContract`] records the meaning required to admit a value into a
//! workflow. It complements physical schemas with spatial, measure, temporal,
//! coverage, source, and quality semantics. Unknown semantics remain explicit
//! and compatibility never turns missing information into a match.

#![deny(missing_docs)]

use genegis_crs::{CoordinateUnit, Crs, SourceSnapshot};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;

mod policy;
mod source_assurance;
mod verification;

pub use policy::{
    AttestationEvidence, CheckRequirement, ContractEvidence, IndependenceClass, ReplayEvidence,
    SourceEvidence, TrustAssessment, TrustEvidence, TrustFailure, TrustGate, TrustLevel,
    VerificationEvidence, VerificationPolicy, VERIFICATION_POLICY_SCHEMA_VERSION,
};
pub use source_assurance::{
    AssuranceCheck, AssuranceCheckKind, AssuranceFailure, AssuranceLevel, AssurancePolicy,
    AssuranceReport, AuthorityClass, CorroborationEvidence, CorroborationIndependence,
    DisputeRecord, DisputeStatus, SourceAssurance, SourceUncertainty,
    SOURCE_ASSURANCE_SCHEMA_VERSION,
};
pub use verification::{
    VerificationGraph, VerificationGraphError, VerificationNode, VerifierIdentity,
    VERIFICATION_GRAPH_SCHEMA_VERSION,
};

/// Schema version emitted by this crate.
pub const GEO_CONTRACT_SCHEMA_VERSION: &str = "0.1.0";

/// Stable identifier of the bundled JSON Schema.
pub const GEO_CONTRACT_SCHEMA_ID: &str =
    "https://genegis.org/schemas/geo-contract/0.1.0/schema.json";

/// Return the committed JSON Schema for [`GeoContract`].
pub fn geo_contract_json_schema() -> serde_json::Value {
    serde_json::from_str(include_str!("../schema/geo-contract-v0.schema.json"))
        .expect("bundled GeoContract schema must be valid JSON")
}

fn default_schema_version() -> String {
    GEO_CONTRACT_SCHEMA_VERSION.to_string()
}

/// Semantic contract attached to a workflow input or output value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeoContract {
    /// Version of the contract document shape and compatibility rules.
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    /// Stable contract identifier within a workflow or adapter manifest.
    pub id: String,
    /// Spatial meaning, when the value contains geometry or a coverage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spatial: Option<SpatialContract>,
    /// Meaning and unit of a measured or classified value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measure: Option<MeasureContract>,
    /// Reference period and observation-time semantics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temporal: Option<TemporalContract>,
    /// Expected geographic/record coverage and join behavior.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage: Option<CoverageContract>,
    /// Immutable input-source identity and authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceContract>,
    /// Declared uncertainty and acceptance tolerances.
    #[serde(default)]
    pub quality: QualityContract,
}

impl GeoContract {
    /// Construct an empty versioned contract with a stable identifier.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            schema_version: default_schema_version(),
            id: id.into(),
            spatial: None,
            measure: None,
            temporal: None,
            coverage: None,
            source: None,
            quality: QualityContract::default(),
        }
    }

    /// Attach spatial semantics.
    pub fn with_spatial(mut self, spatial: SpatialContract) -> Self {
        self.spatial = Some(spatial);
        self
    }

    /// Attach measure semantics.
    pub fn with_measure(mut self, measure: MeasureContract) -> Self {
        self.measure = Some(measure);
        self
    }

    /// Attach temporal semantics.
    pub fn with_temporal(mut self, temporal: TemporalContract) -> Self {
        self.temporal = Some(temporal);
        self
    }

    /// Attach coverage and join semantics.
    pub fn with_coverage(mut self, coverage: CoverageContract) -> Self {
        self.coverage = Some(coverage);
        self
    }

    /// Attach source semantics.
    pub fn with_source(mut self, source: SourceContract) -> Self {
        self.source = Some(source);
        self
    }

    /// Attach quality requirements.
    pub fn with_quality(mut self, quality: QualityContract) -> Self {
        self.quality = quality;
        self
    }

    /// Validate the document and fail on missing or unknown required meaning.
    pub fn validate(&self) -> Result<(), ContractValidationError> {
        validate_contract(self)
    }

    /// Compare this provided contract with a required contract.
    ///
    /// Missing required/provided semantics produce `indeterminate`, never a
    /// successful match. Known disagreements produce `incompatible`.
    pub fn compatibility_with(&self, required: &Self) -> CompatibilityReport {
        compare_contracts(self, required)
    }
}

/// Geometry family represented by a spatial value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeometryKind {
    /// Point or multi-point geometry.
    Point,
    /// Line string or multi-line geometry.
    Line,
    /// Polygon or multi-polygon geometry.
    Polygon,
    /// Raster or gridded coverage.
    Raster,
    /// Point-cloud coverage.
    PointCloud,
    /// A deliberately heterogeneous collection.
    Mixed,
    /// Geometry kind is not known.
    Unknown,
}

/// Coordinate-axis interpretation in the serialized value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AxisOrder {
    /// Generic X followed by Y.
    Xy,
    /// Longitude followed by latitude.
    LongitudeLatitude,
    /// Latitude followed by longitude.
    LatitudeLongitude,
    /// Axis interpretation is not known.
    Unknown,
}

/// Spatial extent in the declared CRS.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpatialExtent {
    /// Minimum X or longitude coordinate.
    pub min_x: f64,
    /// Minimum Y or latitude coordinate.
    pub min_y: f64,
    /// Maximum X or longitude coordinate.
    pub max_x: f64,
    /// Maximum Y or latitude coordinate.
    pub max_y: f64,
}

/// Spatial resolution in coordinate-axis units.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpatialResolution {
    /// Resolution along the first coordinate axis.
    pub x: f64,
    /// Resolution along the second coordinate axis.
    pub y: f64,
}

/// CRS, geometry, extent, and resolution contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpatialContract {
    /// Geometry family.
    pub geometry_kind: GeometryKind,
    /// CRS authority identifier; absent means explicitly unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crs: Option<Crs>,
    /// Axis order used by the serialized coordinates.
    pub axis_order: AxisOrder,
    /// Coordinate-axis unit.
    pub coordinate_unit: CoordinateUnit,
    /// Optional declared bounding box.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extent: Option<SpatialExtent>,
    /// Optional grid or sampling resolution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<SpatialResolution>,
}

impl SpatialContract {
    /// Construct known spatial semantics and derive the coordinate unit.
    pub fn known(geometry_kind: GeometryKind, crs: Crs, axis_order: AxisOrder) -> Self {
        Self {
            geometry_kind,
            coordinate_unit: crs.coordinate_unit(),
            crs: Some(crs),
            axis_order,
            extent: None,
            resolution: None,
        }
    }
}

/// Semantic family of a measured value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasureKind {
    /// Discrete count.
    Count,
    /// Area measurement.
    Area,
    /// Length measurement.
    Length,
    /// Count or amount divided by area.
    Density,
    /// Rate over time or another exposure.
    Rate,
    /// Dimensionless ratio.
    Ratio,
    /// Categorical value.
    Category,
    /// Identifier rather than a measurement.
    Identifier,
    /// Geometry-valued field marker.
    Geometry,
    /// Measure meaning is not known.
    Unknown,
}

/// Aggregation semantics used to derive a measure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregationBasis {
    /// Values are summed.
    Sum,
    /// Arithmetic mean.
    Mean,
    /// Count of records or entities.
    Count,
    /// Ratio of separately aggregated terms.
    RatioOfSums,
    /// Value is not aggregated.
    None,
    /// Aggregation basis is not known.
    Unknown,
}

/// One numerator or denominator term in a derived measure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasureTerm {
    /// Semantic kind of the term.
    pub kind: MeasureKind,
    /// UCUM-like or documented stable unit spelling.
    pub unit: String,
}

/// Value semantics, units, and aggregation basis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasureContract {
    /// Semantic measure family.
    pub kind: MeasureKind,
    /// Stable unit spelling, for example `person` or `person/km2`.
    pub unit: String,
    /// Numerator semantics for a ratio, rate, or density.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub numerator: Option<MeasureTerm>,
    /// Denominator semantics for a ratio, rate, or density.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub denominator: Option<MeasureTerm>,
    /// How values are aggregated.
    pub aggregation: AggregationBasis,
    /// Population/universe represented by the measure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub population_universe: Option<String>,
}

impl MeasureContract {
    /// Construct a simple measured value without ratio terms.
    pub fn simple(
        kind: MeasureKind,
        unit: impl Into<String>,
        aggregation: AggregationBasis,
    ) -> Self {
        Self {
            kind,
            unit: unit.into(),
            numerator: None,
            denominator: None,
            aggregation,
            population_universe: None,
        }
    }
}

/// Granularity of a temporal reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemporalGranularity {
    /// Calendar year.
    Year,
    /// Calendar month.
    Month,
    /// Calendar date.
    Day,
    /// Timestamp/instant.
    Instant,
    /// Explicit interval.
    Interval,
    /// Temporal granularity is not known.
    Unknown,
}

/// Reference-time and observation-time semantics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemporalContract {
    /// Stable reference period such as `2020` or `2020-10-01/2020-10-01`.
    pub reference_period: String,
    /// Temporal granularity of the reference period.
    pub granularity: TemporalGranularity,
    /// Observation/publication timestamp or date when distinct from reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<String>,
}

/// Expected uniqueness of coverage/join keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyUniqueness {
    /// Every key occurs exactly once.
    Unique,
    /// Duplicate keys are explicitly allowed.
    NonUnique,
    /// Key uniqueness is not known.
    Unknown,
}

/// Policy for null values in contract-critical fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NullPolicy {
    /// Null values are rejected.
    Reject,
    /// Null values are retained and must remain visible.
    Preserve,
    /// Null values may be omitted from the derived result.
    Drop,
    /// Null behavior is not known.
    Unknown,
}

/// Geographic/record coverage and join-cardinality semantics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageContract {
    /// Human- and machine-stable scope identifier.
    pub scope: String,
    /// Expected feature/entity count when authoritative.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_feature_count: Option<u64>,
    /// Fields that identify entities across joins.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub join_keys: Vec<String>,
    /// Required uniqueness of the join keys.
    pub key_uniqueness: KeyUniqueness,
    /// Handling of null contract-critical values.
    pub null_policy: NullPolicy,
}

/// Immutable source snapshot plus authority/freshness semantics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceContract {
    /// Source snapshot identity and checksum state.
    pub snapshot: SourceSnapshot,
    /// Publishing or responsible authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority: Option<String>,
    /// Maximum acceptable age in whole days; absent means policy-defined.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_age_days: Option<u32>,
}

impl SourceContract {
    /// Construct source semantics from an immutable snapshot.
    pub fn new(snapshot: SourceSnapshot) -> Self {
        Self {
            snapshot,
            authority: None,
            max_age_days: None,
        }
    }
}

/// Declared uncertainty of a value or verification oracle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Uncertainty {
    /// Method or source used to estimate uncertainty.
    pub method: String,
    /// Absolute uncertainty in the value unit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub absolute: Option<f64>,
    /// Relative uncertainty in parts per million.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative_ppm: Option<u64>,
}

/// One machine-checkable quality tolerance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualityTolerance {
    /// Stable metric identifier, for example `area_relative_error`.
    pub metric: String,
    /// Maximum accepted error in parts per million.
    pub max_error_ppm: u64,
}

/// Uncertainty and quality acceptance contract.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualityContract {
    /// Declared uncertainty, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uncertainty: Option<Uncertainty>,
    /// Required verifier tolerances.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tolerances: Vec<QualityTolerance>,
}

/// Structural or semantic validation failure.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ContractValidationError {
    /// Contract uses an unsupported schema version.
    #[error("unsupported GeoContract schema version: {0}")]
    UnsupportedSchemaVersion(String),
    /// A required field is empty.
    #[error("GeoContract field is empty: {0}")]
    EmptyField(&'static str),
    /// A required semantic is explicitly unknown.
    #[error("GeoContract semantic is unknown: {0}")]
    UnknownSemantic(&'static str),
    /// CRS and coordinate unit disagree.
    #[error("coordinate unit {actual} is incompatible with {crs}; expected {expected}")]
    CoordinateUnitMismatch {
        /// CRS identifier.
        crs: String,
        /// Unit derived from the CRS.
        expected: CoordinateUnit,
        /// Unit declared by the contract.
        actual: CoordinateUnit,
    },
    /// Extent coordinates are non-finite or inverted.
    #[error("invalid spatial extent")]
    InvalidExtent,
    /// Resolution is non-finite or non-positive.
    #[error("invalid spatial resolution")]
    InvalidResolution,
    /// Density/rate/ratio lacks numerator or denominator semantics.
    #[error("derived measure requires numerator and denominator")]
    MissingMeasureTerms,
    /// A list contains duplicate identifiers.
    #[error("duplicate GeoContract value in {field}: {value}")]
    DuplicateValue {
        /// Field containing the duplicate.
        field: &'static str,
        /// Duplicate value.
        value: String,
    },
    /// A numeric quality value is invalid.
    #[error("invalid quality value: {0}")]
    InvalidQuality(&'static str),
    /// CRS identifier is unsupported.
    #[error("invalid spatial CRS: {0}")]
    InvalidCrs(String),
}

/// Overall result of comparing provided and required semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityStatus {
    /// All declared required semantics match.
    Compatible,
    /// At least one known semantic contradicts a requirement.
    Incompatible,
    /// No contradiction is known, but required information is missing/unknown.
    Indeterminate,
}

/// One field-level compatibility finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompatibilityFinding {
    /// Stable semantic field path.
    pub field: String,
    /// Field-level compatibility status.
    pub status: CompatibilityStatus,
    /// Human-readable explanation suitable for a structured error view.
    pub detail: String,
}

/// Deterministic compatibility report between two contracts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompatibilityReport {
    /// Overall worst-case status.
    pub status: CompatibilityStatus,
    /// Ordered field-level findings.
    pub findings: Vec<CompatibilityFinding>,
}

impl CompatibilityReport {
    /// Return true only for a fully known compatible result.
    pub fn is_compatible(&self) -> bool {
        self.status == CompatibilityStatus::Compatible
    }
}

fn validate_contract(contract: &GeoContract) -> Result<(), ContractValidationError> {
    if contract.schema_version != GEO_CONTRACT_SCHEMA_VERSION {
        return Err(ContractValidationError::UnsupportedSchemaVersion(
            contract.schema_version.clone(),
        ));
    }
    require_text(&contract.id, "id")?;

    if let Some(spatial) = &contract.spatial {
        if spatial.geometry_kind == GeometryKind::Unknown {
            return Err(ContractValidationError::UnknownSemantic(
                "spatial.geometry_kind",
            ));
        }
        if spatial.axis_order == AxisOrder::Unknown {
            return Err(ContractValidationError::UnknownSemantic(
                "spatial.axis_order",
            ));
        }
        if spatial.coordinate_unit == CoordinateUnit::Unknown {
            return Err(ContractValidationError::UnknownSemantic(
                "spatial.coordinate_unit",
            ));
        }
        let crs = spatial
            .crs
            .as_ref()
            .ok_or(ContractValidationError::UnknownSemantic("spatial.crs"))?;
        let definition = crs
            .require_known()
            .map_err(|error| ContractValidationError::InvalidCrs(error.to_string()))?;
        if definition.unit != spatial.coordinate_unit {
            return Err(ContractValidationError::CoordinateUnitMismatch {
                crs: crs.identifier(),
                expected: definition.unit,
                actual: spatial.coordinate_unit,
            });
        }
        if let Some(extent) = spatial.extent {
            if ![extent.min_x, extent.min_y, extent.max_x, extent.max_y]
                .into_iter()
                .all(f64::is_finite)
                || extent.min_x > extent.max_x
                || extent.min_y > extent.max_y
            {
                return Err(ContractValidationError::InvalidExtent);
            }
        }
        if let Some(resolution) = spatial.resolution {
            if !resolution.x.is_finite()
                || !resolution.y.is_finite()
                || resolution.x <= 0.0
                || resolution.y <= 0.0
            {
                return Err(ContractValidationError::InvalidResolution);
            }
        }
    }

    if let Some(measure) = &contract.measure {
        if measure.kind == MeasureKind::Unknown {
            return Err(ContractValidationError::UnknownSemantic("measure.kind"));
        }
        require_text(&measure.unit, "measure.unit")?;
        if measure.aggregation == AggregationBasis::Unknown {
            return Err(ContractValidationError::UnknownSemantic(
                "measure.aggregation",
            ));
        }
        if matches!(
            measure.kind,
            MeasureKind::Density | MeasureKind::Rate | MeasureKind::Ratio
        ) && (measure.numerator.is_none() || measure.denominator.is_none())
        {
            return Err(ContractValidationError::MissingMeasureTerms);
        }
        for (field, term) in [
            ("measure.numerator.unit", measure.numerator.as_ref()),
            ("measure.denominator.unit", measure.denominator.as_ref()),
        ] {
            if let Some(term) = term {
                if term.kind == MeasureKind::Unknown {
                    return Err(ContractValidationError::UnknownSemantic(field));
                }
                require_text(&term.unit, field)?;
            }
        }
        if let Some(universe) = &measure.population_universe {
            require_text(universe, "measure.population_universe")?;
        }
    }

    if let Some(temporal) = &contract.temporal {
        require_text(&temporal.reference_period, "temporal.reference_period")?;
        if temporal.granularity == TemporalGranularity::Unknown {
            return Err(ContractValidationError::UnknownSemantic(
                "temporal.granularity",
            ));
        }
        if let Some(observed_at) = &temporal.observed_at {
            require_text(observed_at, "temporal.observed_at")?;
        }
    }

    if let Some(coverage) = &contract.coverage {
        require_text(&coverage.scope, "coverage.scope")?;
        if coverage.key_uniqueness == KeyUniqueness::Unknown {
            return Err(ContractValidationError::UnknownSemantic(
                "coverage.key_uniqueness",
            ));
        }
        if coverage.null_policy == NullPolicy::Unknown {
            return Err(ContractValidationError::UnknownSemantic(
                "coverage.null_policy",
            ));
        }
        if coverage.expected_feature_count == Some(0) {
            return Err(ContractValidationError::InvalidQuality(
                "coverage.expected_feature_count",
            ));
        }
        let mut keys = BTreeSet::new();
        for key in &coverage.join_keys {
            require_text(key, "coverage.join_keys")?;
            if !keys.insert(key) {
                return Err(ContractValidationError::DuplicateValue {
                    field: "coverage.join_keys",
                    value: key.clone(),
                });
            }
        }
    }

    if let Some(source) = &contract.source {
        require_text(&source.snapshot.uri, "source.snapshot.uri")?;
        if let Some(authority) = &source.authority {
            require_text(authority, "source.authority")?;
        }
        if source.max_age_days == Some(0) {
            return Err(ContractValidationError::InvalidQuality(
                "source.max_age_days",
            ));
        }
    }

    if let Some(uncertainty) = &contract.quality.uncertainty {
        require_text(&uncertainty.method, "quality.uncertainty.method")?;
        if let Some(absolute) = uncertainty.absolute {
            if !absolute.is_finite() || absolute < 0.0 {
                return Err(ContractValidationError::InvalidQuality(
                    "quality.uncertainty.absolute",
                ));
            }
        }
    }
    let mut metrics = BTreeSet::new();
    for tolerance in &contract.quality.tolerances {
        require_text(&tolerance.metric, "quality.tolerances.metric")?;
        if !metrics.insert(&tolerance.metric) {
            return Err(ContractValidationError::DuplicateValue {
                field: "quality.tolerances.metric",
                value: tolerance.metric.clone(),
            });
        }
    }

    Ok(())
}

fn require_text(value: &str, field: &'static str) -> Result<(), ContractValidationError> {
    if value.trim().is_empty() || value.trim().eq_ignore_ascii_case("unknown") {
        Err(ContractValidationError::EmptyField(field))
    } else {
        Ok(())
    }
}

fn compare_contracts(provided: &GeoContract, required: &GeoContract) -> CompatibilityReport {
    let mut findings = Vec::new();
    compare_value(
        "schema_version",
        Some(&provided.schema_version),
        Some(&required.schema_version),
        &mut findings,
    );
    compare_spatial(&provided.spatial, &required.spatial, &mut findings);
    compare_measure(&provided.measure, &required.measure, &mut findings);
    compare_temporal(&provided.temporal, &required.temporal, &mut findings);
    compare_coverage(&provided.coverage, &required.coverage, &mut findings);

    if let Some(required_source) = &required.source {
        match &provided.source {
            None => findings.push(indeterminate("source", "provided source is missing")),
            Some(provided_source) => {
                compare_value(
                    "source.snapshot.uri",
                    Some(&provided_source.snapshot.uri),
                    Some(&required_source.snapshot.uri),
                    &mut findings,
                );
                compare_optional(
                    "source.snapshot.source_version",
                    &provided_source.snapshot.source_version,
                    &required_source.snapshot.source_version,
                    &mut findings,
                );
                compare_optional(
                    "source.snapshot.expected_checksum",
                    &provided_source.snapshot.expected_checksum,
                    &required_source.snapshot.expected_checksum,
                    &mut findings,
                );
                compare_optional(
                    "source.authority",
                    &provided_source.authority,
                    &required_source.authority,
                    &mut findings,
                );
            }
        }
    }

    for required_tolerance in &required.quality.tolerances {
        let field = format!("quality.tolerances.{}", required_tolerance.metric);
        match provided
            .quality
            .tolerances
            .iter()
            .find(|candidate| candidate.metric == required_tolerance.metric)
        {
            None => findings.push(indeterminate(&field, "required tolerance is missing")),
            Some(candidate) if candidate.max_error_ppm <= required_tolerance.max_error_ppm => {
                findings.push(CompatibilityFinding {
                    field,
                    status: CompatibilityStatus::Compatible,
                    detail: "provided tolerance is at least as strict as required".into(),
                });
            }
            Some(candidate) => findings.push(CompatibilityFinding {
                field,
                status: CompatibilityStatus::Incompatible,
                detail: format!(
                    "provided maximum {} ppm exceeds required {} ppm",
                    candidate.max_error_ppm, required_tolerance.max_error_ppm
                ),
            }),
        }
    }

    let status = if findings
        .iter()
        .any(|finding| finding.status == CompatibilityStatus::Incompatible)
    {
        CompatibilityStatus::Incompatible
    } else if findings
        .iter()
        .any(|finding| finding.status == CompatibilityStatus::Indeterminate)
    {
        CompatibilityStatus::Indeterminate
    } else {
        CompatibilityStatus::Compatible
    };
    CompatibilityReport { status, findings }
}

fn compare_spatial(
    provided: &Option<SpatialContract>,
    required: &Option<SpatialContract>,
    findings: &mut Vec<CompatibilityFinding>,
) {
    let Some(required) = required else { return };
    let Some(provided) = provided else {
        findings.push(indeterminate(
            "spatial",
            "provided spatial semantic is missing",
        ));
        return;
    };
    compare_value(
        "spatial.geometry_kind",
        Some(&provided.geometry_kind),
        Some(&required.geometry_kind),
        findings,
    );
    compare_optional("spatial.crs", &provided.crs, &required.crs, findings);
    compare_value(
        "spatial.axis_order",
        Some(&provided.axis_order),
        Some(&required.axis_order),
        findings,
    );
    compare_value(
        "spatial.coordinate_unit",
        Some(&provided.coordinate_unit),
        Some(&required.coordinate_unit),
        findings,
    );
    compare_optional(
        "spatial.extent",
        &provided.extent,
        &required.extent,
        findings,
    );
    compare_optional(
        "spatial.resolution",
        &provided.resolution,
        &required.resolution,
        findings,
    );
}

fn compare_measure(
    provided: &Option<MeasureContract>,
    required: &Option<MeasureContract>,
    findings: &mut Vec<CompatibilityFinding>,
) {
    let Some(required) = required else { return };
    let Some(provided) = provided else {
        findings.push(indeterminate(
            "measure",
            "provided measure semantic is missing",
        ));
        return;
    };
    compare_value(
        "measure.kind",
        Some(&provided.kind),
        Some(&required.kind),
        findings,
    );
    compare_value(
        "measure.unit",
        Some(&provided.unit),
        Some(&required.unit),
        findings,
    );
    compare_value(
        "measure.aggregation",
        Some(&provided.aggregation),
        Some(&required.aggregation),
        findings,
    );
    compare_optional(
        "measure.numerator",
        &provided.numerator,
        &required.numerator,
        findings,
    );
    compare_optional(
        "measure.denominator",
        &provided.denominator,
        &required.denominator,
        findings,
    );
    compare_optional(
        "measure.population_universe",
        &provided.population_universe,
        &required.population_universe,
        findings,
    );
}

fn compare_temporal(
    provided: &Option<TemporalContract>,
    required: &Option<TemporalContract>,
    findings: &mut Vec<CompatibilityFinding>,
) {
    let Some(required) = required else { return };
    let Some(provided) = provided else {
        findings.push(indeterminate(
            "temporal",
            "provided temporal semantic is missing",
        ));
        return;
    };
    compare_value(
        "temporal.reference_period",
        Some(&provided.reference_period),
        Some(&required.reference_period),
        findings,
    );
    compare_value(
        "temporal.granularity",
        Some(&provided.granularity),
        Some(&required.granularity),
        findings,
    );
    compare_optional(
        "temporal.observed_at",
        &provided.observed_at,
        &required.observed_at,
        findings,
    );
}

fn compare_coverage(
    provided: &Option<CoverageContract>,
    required: &Option<CoverageContract>,
    findings: &mut Vec<CompatibilityFinding>,
) {
    let Some(required) = required else { return };
    let Some(provided) = provided else {
        findings.push(indeterminate(
            "coverage",
            "provided coverage semantic is missing",
        ));
        return;
    };
    compare_value(
        "coverage.scope",
        Some(&provided.scope),
        Some(&required.scope),
        findings,
    );
    compare_optional(
        "coverage.expected_feature_count",
        &provided.expected_feature_count,
        &required.expected_feature_count,
        findings,
    );
    if !required.join_keys.is_empty() {
        compare_value(
            "coverage.join_keys",
            Some(&provided.join_keys),
            Some(&required.join_keys),
            findings,
        );
    }
    compare_value(
        "coverage.key_uniqueness",
        Some(&provided.key_uniqueness),
        Some(&required.key_uniqueness),
        findings,
    );
    compare_value(
        "coverage.null_policy",
        Some(&provided.null_policy),
        Some(&required.null_policy),
        findings,
    );
}

fn compare_optional<T: PartialEq + std::fmt::Debug>(
    field: &str,
    provided: &Option<T>,
    required: &Option<T>,
    findings: &mut Vec<CompatibilityFinding>,
) {
    if required.is_none() {
        return;
    }
    match provided {
        None => findings.push(indeterminate(field, "provided semantic is missing")),
        Some(provided) => compare_value(field, Some(provided), required.as_ref(), findings),
    }
}

fn compare_value<T: PartialEq + std::fmt::Debug>(
    field: &str,
    provided: Option<&T>,
    required: Option<&T>,
    findings: &mut Vec<CompatibilityFinding>,
) {
    let Some(required) = required else { return };
    match provided {
        None => findings.push(indeterminate(field, "provided semantic is missing")),
        Some(provided) if provided == required => findings.push(CompatibilityFinding {
            field: field.to_string(),
            status: CompatibilityStatus::Compatible,
            detail: "provided semantic matches requirement".into(),
        }),
        Some(provided) => findings.push(CompatibilityFinding {
            field: field.to_string(),
            status: CompatibilityStatus::Incompatible,
            detail: format!("provided {provided:?} does not match required {required:?}"),
        }),
    }
}

fn indeterminate(field: &str, detail: &str) -> CompatibilityFinding {
    CompatibilityFinding {
        field: field.to_string(),
        status: CompatibilityStatus::Indeterminate,
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> SourceContract {
        let mut source = SourceContract::new(SourceSnapshot::from_uri(
            "https://example.test/nagoya.geojson",
            Some("sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            Some("2020-final"),
        ));
        source.authority = Some("Nagoya City".into());
        source
    }

    fn boundary_contract() -> GeoContract {
        GeoContract::new("nagoya.boundary.2020")
            .with_spatial(SpatialContract::known(
                GeometryKind::Polygon,
                Crs::wgs84(),
                AxisOrder::LongitudeLatitude,
            ))
            .with_temporal(TemporalContract {
                reference_period: "2020-10-01".into(),
                granularity: TemporalGranularity::Day,
                observed_at: None,
            })
            .with_coverage(CoverageContract {
                scope: "JP-23/Nagoya wards".into(),
                expected_feature_count: Some(16),
                join_keys: vec!["ward_code".into()],
                key_uniqueness: KeyUniqueness::Unique,
                null_policy: NullPolicy::Reject,
            })
            .with_source(source())
    }

    fn density_contract() -> GeoContract {
        GeoContract::new("nagoya.population-density.2020")
            .with_measure(MeasureContract {
                kind: MeasureKind::Density,
                unit: "person/km2".into(),
                numerator: Some(MeasureTerm {
                    kind: MeasureKind::Count,
                    unit: "person".into(),
                }),
                denominator: Some(MeasureTerm {
                    kind: MeasureKind::Area,
                    unit: "km2".into(),
                }),
                aggregation: AggregationBasis::RatioOfSums,
                population_universe: Some("2020 census usual residents".into()),
            })
            .with_temporal(TemporalContract {
                reference_period: "2020".into(),
                granularity: TemporalGranularity::Year,
                observed_at: None,
            })
            .with_quality(QualityContract {
                uncertainty: None,
                tolerances: vec![QualityTolerance {
                    metric: "density_relative_error".into(),
                    max_error_ppm: 5_000,
                }],
            })
    }

    #[test]
    fn valid_contract_round_trips_without_losing_meaning() {
        let contract = boundary_contract();
        contract.validate().expect("valid contract");
        let json = serde_json::to_value(&contract).expect("serialize");
        let decoded: GeoContract = serde_json::from_value(json).expect("deserialize");
        assert_eq!(decoded, contract);
        assert_eq!(decoded.schema_version, GEO_CONTRACT_SCHEMA_VERSION);
    }

    #[test]
    fn rejects_unknown_spatial_and_temporal_semantics() {
        let mut contract = boundary_contract();
        contract.spatial.as_mut().expect("spatial").crs = None;
        assert_eq!(
            contract.validate(),
            Err(ContractValidationError::UnknownSemantic("spatial.crs"))
        );

        let mut contract = boundary_contract();
        contract.temporal.as_mut().expect("temporal").granularity = TemporalGranularity::Unknown;
        assert_eq!(
            contract.validate(),
            Err(ContractValidationError::UnknownSemantic(
                "temporal.granularity"
            ))
        );
    }

    #[test]
    fn rejects_crs_unit_mismatch_and_invalid_extent() {
        let mut contract = boundary_contract();
        contract.spatial.as_mut().expect("spatial").coordinate_unit = CoordinateUnit::Metres;
        assert!(matches!(
            contract.validate(),
            Err(ContractValidationError::CoordinateUnitMismatch { .. })
        ));

        let mut contract = boundary_contract();
        contract.spatial.as_mut().expect("spatial").extent = Some(SpatialExtent {
            min_x: 10.0,
            min_y: 0.0,
            max_x: -10.0,
            max_y: 1.0,
        });
        assert_eq!(
            contract.validate(),
            Err(ContractValidationError::InvalidExtent)
        );
    }

    #[test]
    fn rejects_incomplete_density_and_duplicate_join_keys() {
        let mut density = density_contract();
        density.measure.as_mut().expect("measure").denominator = None;
        assert_eq!(
            density.validate(),
            Err(ContractValidationError::MissingMeasureTerms)
        );

        let mut boundary = boundary_contract();
        boundary.coverage.as_mut().expect("coverage").join_keys =
            vec!["ward_code".into(), "ward_code".into()];
        assert!(matches!(
            boundary.validate(),
            Err(ContractValidationError::DuplicateValue { .. })
        ));
    }

    #[test]
    fn compatibility_truth_table_is_fail_closed() {
        let required = density_contract();

        let matching = required.clone();
        assert_eq!(
            matching.compatibility_with(&required).status,
            CompatibilityStatus::Compatible
        );

        let mut missing = required.clone();
        missing.temporal = None;
        assert_eq!(
            missing.compatibility_with(&required).status,
            CompatibilityStatus::Indeterminate
        );

        let mut different = required.clone();
        different
            .temporal
            .as_mut()
            .expect("temporal")
            .reference_period = "2021".into();
        assert_eq!(
            different.compatibility_with(&required).status,
            CompatibilityStatus::Incompatible
        );

        let mut stricter = required.clone();
        stricter.quality.tolerances[0].max_error_ppm = 1_000;
        assert_eq!(
            stricter.compatibility_with(&required).status,
            CompatibilityStatus::Compatible
        );

        let mut weaker = required.clone();
        weaker.quality.tolerances[0].max_error_ppm = 10_000;
        assert_eq!(
            weaker.compatibility_with(&required).status,
            CompatibilityStatus::Incompatible
        );
    }

    #[test]
    fn unspecified_requirement_is_not_a_constraint() {
        let provided = boundary_contract();
        let required = GeoContract::new("any-input");
        assert_eq!(
            provided.compatibility_with(&required).status,
            CompatibilityStatus::Compatible
        );
    }

    #[test]
    fn schema_document_identifies_versioned_contract_and_six_domains() {
        let schema = geo_contract_json_schema();
        assert_eq!(schema["$id"], GEO_CONTRACT_SCHEMA_ID);
        for field in [
            "spatial", "measure", "temporal", "coverage", "source", "quality",
        ] {
            assert!(schema["properties"][field].is_object(), "missing {field}");
        }
    }

    #[test]
    fn rejects_unknown_json_fields() {
        let value = serde_json::json!({
            "schema_version": GEO_CONTRACT_SCHEMA_VERSION,
            "id": "bad",
            "surprise": true
        });
        assert!(serde_json::from_value::<GeoContract>(value).is_err());
    }
}
