//! Typed read-only PostGIS execution with proof-oriented receipts.

use crate::{
    admit, AdapterInvocation, AdapterManifest, AdapterOperation, AdmissionFailure, BackendFamily,
    BackendIdentity, Capability, CapabilityPolicy, Determinism, EvidenceHook,
    ADAPTER_MANIFEST_SCHEMA_VERSION,
};
use genegis_contract::{
    AggregationBasis, AxisOrder, GeoContract, GeometryKind, MeasureContract, MeasureKind,
    SpatialContract,
};
use genegis_crs::Crs;
use postgres::{IsolationLevel, NoTls};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;
use thiserror::Error;

/// Official image tag used to resolve the pinned PostGIS conformance image.
pub const POSTGIS_IMAGE_REFERENCE: &str = "docker.io/postgis/postgis:18-3.6";

/// Registry digest observed and executed by the Phase 12 conformance harness.
pub const POSTGIS_IMAGE_DIGEST: &str =
    "sha256:db8c151a4e1f4686b1a985a3490cf96f9f8c8c2725f58a46ef7a57e52f167cc3";

const ADAPTER_ID: &str = "org.genegis.postgis.read-only";
const ADAPTER_VERSION: &str = "0.1.0";

/// Typed operations supported by the initial PostGIS adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum PostgisOperation {
    /// Calculate planar polygon area in the declared projected CRS.
    ProjectedArea {
        /// Polygon WKT.
        wkt: String,
        /// Projected EPSG code.
        srid: i32,
    },
    /// Transform a point between two declared CRSs.
    TransformPoint {
        /// X or longitude coordinate.
        x: f64,
        /// Y or latitude coordinate.
        y: f64,
        /// Source EPSG code.
        source_srid: i32,
        /// Target EPSG code.
        target_srid: i32,
    },
    /// Test whether two geometries intersect in one CRS.
    Intersects {
        /// Left geometry WKT.
        left_wkt: String,
        /// Right geometry WKT.
        right_wkt: String,
        /// Shared EPSG code.
        srid: i32,
    },
    /// Count atomic geometry parts without dropping multipart members.
    MultipartPartCount {
        /// Multipart geometry WKT.
        wkt: String,
        /// EPSG code.
        srid: i32,
    },
    /// Canonically order nullable integer values with nulls last.
    OrderNullableValues {
        /// Values supplied as one typed SQL array.
        values: Vec<Option<i64>>,
    },
}

impl PostgisOperation {
    fn operation_id(&self) -> &'static str {
        match self {
            Self::ProjectedArea { .. } => "postgis.area.projected",
            Self::TransformPoint { .. } => "postgis.transform.point",
            Self::Intersects { .. } => "postgis.intersects",
            Self::MultipartPartCount { .. } => "postgis.multipart.part-count",
            Self::OrderNullableValues { .. } => "postgis.null-ordering",
        }
    }

    fn parameters(&self) -> Value {
        serde_json::to_value(self).expect("PostGIS operation serialization is infallible")
    }
}

/// Content-addressed evidence emitted by one successful PostGIS operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PostgisReceipt {
    /// Adapter manifest digest admitted before connecting.
    pub manifest_digest: String,
    /// Semantic operation identity.
    pub operation_id: String,
    /// Exact backend identity reviewed by policy.
    pub backend: BackendIdentity,
    /// Runtime PostgreSQL version string.
    pub runtime_postgresql_version: String,
    /// Runtime PostGIS/GEOS/PROJ identity string.
    pub runtime_postgis_full_version: String,
    /// Normalized typed operation parameters.
    pub parameters: Value,
    /// Digest of `EXPLAIN (FORMAT JSON)` output.
    pub query_plan_digest: String,
    /// Canonical operation result.
    pub output: Value,
    /// Digest of the canonical operation result.
    pub output_digest: String,
    /// PostgreSQL transaction mode enforced by the adapter.
    pub transaction_read_only: bool,
    /// PostgreSQL isolation level enforced by the adapter.
    pub isolation_level: String,
    /// Measured wall time including admission, connection, and query.
    pub elapsed_ns: u64,
}

/// Failure at admission, connection, execution, or runtime identity validation.
#[derive(Debug, Error)]
pub enum PostgisError {
    /// Capability or manifest policy rejected execution before connection.
    #[error("PostGIS adapter admission failed: {0:?}")]
    Admission(Vec<AdmissionFailure>),
    /// Typed parameters disagree with the operation's reviewed GeoContract.
    #[error("PostGIS operation contract failed: {0}")]
    Contract(String),
    /// PostgreSQL returned an execution error.
    #[error("PostGIS execution failed: {0}")]
    Database(#[from] postgres::Error),
    /// Runtime engine version differs from the pinned manifest.
    #[error("PostGIS runtime identity mismatch: {0}")]
    RuntimeIdentity(String),
    /// Receipt serialization failed.
    #[error("PostGIS receipt serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Read-only executor configured with an exact manifest and capability policy.
#[derive(Debug, Clone)]
pub struct PostgisAdapter {
    manifest: AdapterManifest,
    policy: CapabilityPolicy,
}

impl Default for PostgisAdapter {
    fn default() -> Self {
        let manifest = postgis_manifest();
        let policy = CapabilityPolicy::read_only_database(&manifest.adapter_id);
        Self { manifest, policy }
    }
}

impl PostgisAdapter {
    /// Construct an adapter with an explicitly reviewed manifest and policy.
    pub fn new(manifest: AdapterManifest, policy: CapabilityPolicy) -> Self {
        Self { manifest, policy }
    }

    /// Return the reviewed manifest.
    pub fn manifest(&self) -> &AdapterManifest {
        &self.manifest
    }

    /// Execute one typed operation through a read-only repeatable-read transaction.
    ///
    /// The connection string is used only to connect and is never copied into
    /// the receipt, preventing credential disclosure.
    pub fn execute(
        &self,
        connection_string: &str,
        operation: &PostgisOperation,
    ) -> Result<PostgisReceipt, PostgisError> {
        let started = Instant::now();
        validate_operation_contract(operation)?;
        let invocation = AdapterInvocation {
            adapter_id: self.manifest.adapter_id.clone(),
            adapter_version: self.manifest.adapter_version.clone(),
            operation_id: operation.operation_id().into(),
            operation_version: "1.0.0".into(),
            backend: self.manifest.backend.clone(),
            requested_capabilities: BTreeSet::from([Capability::DatabaseRead]),
        };
        let admission = admit(&self.manifest, &invocation, &self.policy);
        if !admission.admitted || !admission.verification_eligible {
            return Err(PostgisError::Admission(admission.failures));
        }

        let mut client = postgres::Client::connect(connection_string, NoTls)?;
        let mut transaction = client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()?;
        let version = transaction.query_one(
            "SELECT current_setting('server_version'), postgis_full_version()",
            &[],
        )?;
        let postgresql_version: String = version.get(0);
        let postgis_full_version: String = version.get(1);
        validate_runtime_identity(
            &self.manifest.backend,
            &postgresql_version,
            &postgis_full_version,
        )?;

        let (output, plan) = execute_operation(&mut transaction, operation)?;
        transaction.commit()?;

        Ok(PostgisReceipt {
            manifest_digest: self.manifest.digest()?,
            operation_id: operation.operation_id().into(),
            backend: self.manifest.backend.clone(),
            runtime_postgresql_version: postgresql_version,
            runtime_postgis_full_version: postgis_full_version,
            parameters: operation.parameters(),
            query_plan_digest: digest_json(&plan)?,
            output_digest: digest_json(&output)?,
            output,
            transaction_read_only: true,
            isolation_level: "repeatable_read".into(),
            elapsed_ns: started.elapsed().as_nanos().min(u64::MAX as u128) as u64,
        })
    }
}

fn validate_operation_contract(operation: &PostgisOperation) -> Result<(), PostgisError> {
    let valid = match operation {
        PostgisOperation::ProjectedArea { srid, .. }
        | PostgisOperation::Intersects { srid, .. }
        | PostgisOperation::MultipartPartCount { srid, .. } => *srid == 6675,
        PostgisOperation::TransformPoint {
            source_srid,
            target_srid,
            ..
        } => *source_srid == 4326 && *target_srid == 6675,
        PostgisOperation::OrderNullableValues { .. } => true,
    };
    if valid {
        Ok(())
    } else {
        Err(PostgisError::Contract(format!(
            "{} parameters disagree with its reviewed CRS contract",
            operation.operation_id()
        )))
    }
}

fn execute_operation(
    transaction: &mut postgres::Transaction<'_>,
    operation: &PostgisOperation,
) -> Result<(Value, Value), postgres::Error> {
    match operation {
        PostgisOperation::ProjectedArea { wkt, srid } => {
            const SQL: &str = "SELECT ST_Area(ST_GeomFromText($1, $2::integer))";
            const PLAN: &str =
                "EXPLAIN (FORMAT JSON) SELECT ST_Area(ST_GeomFromText($1, $2::integer))";
            let area: f64 = transaction.query_one(SQL, &[wkt, srid])?.get(0);
            let plan: Value = transaction.query_one(PLAN, &[wkt, srid])?.get(0);
            Ok((json!({ "area_square_metres": area }), plan))
        }
        PostgisOperation::TransformPoint {
            x,
            y,
            source_srid,
            target_srid,
        } => {
            const SQL: &str = "SELECT ST_X(g), ST_Y(g) FROM (SELECT ST_Transform(ST_SetSRID(ST_MakePoint($1, $2), $3::integer), $4::integer) AS g) q";
            const PLAN: &str = "EXPLAIN (FORMAT JSON) SELECT ST_X(g), ST_Y(g) FROM (SELECT ST_Transform(ST_SetSRID(ST_MakePoint($1, $2), $3::integer), $4::integer) AS g) q";
            let row = transaction.query_one(SQL, &[x, y, source_srid, target_srid])?;
            let output_x: f64 = row.get(0);
            let output_y: f64 = row.get(1);
            let plan: Value = transaction
                .query_one(PLAN, &[x, y, source_srid, target_srid])?
                .get(0);
            Ok((json!({ "x": output_x, "y": output_y }), plan))
        }
        PostgisOperation::Intersects {
            left_wkt,
            right_wkt,
            srid,
        } => {
            const SQL: &str = "SELECT ST_Intersects(ST_GeomFromText($1, $3::integer), ST_GeomFromText($2, $3::integer))";
            const PLAN: &str = "EXPLAIN (FORMAT JSON) SELECT ST_Intersects(ST_GeomFromText($1, $3::integer), ST_GeomFromText($2, $3::integer))";
            let intersects: bool = transaction
                .query_one(SQL, &[left_wkt, right_wkt, srid])?
                .get(0);
            let plan: Value = transaction
                .query_one(PLAN, &[left_wkt, right_wkt, srid])?
                .get(0);
            Ok((json!({ "intersects": intersects }), plan))
        }
        PostgisOperation::MultipartPartCount { wkt, srid } => {
            const SQL: &str =
                "SELECT count(*)::bigint FROM ST_Dump(ST_GeomFromText($1, $2::integer))";
            const PLAN: &str = "EXPLAIN (FORMAT JSON) SELECT count(*)::bigint FROM ST_Dump(ST_GeomFromText($1, $2::integer))";
            let parts: i64 = transaction.query_one(SQL, &[wkt, srid])?.get(0);
            let plan: Value = transaction.query_one(PLAN, &[wkt, srid])?.get(0);
            Ok((json!({ "part_count": parts }), plan))
        }
        PostgisOperation::OrderNullableValues { values } => {
            const SQL: &str =
                "SELECT value FROM unnest($1::bigint[]) AS value ORDER BY value ASC NULLS LAST";
            const PLAN: &str = "EXPLAIN (FORMAT JSON) SELECT value FROM unnest($1::bigint[]) AS value ORDER BY value ASC NULLS LAST";
            let rows = transaction.query(SQL, &[values])?;
            let ordered = rows
                .iter()
                .map(|row| row.get::<_, Option<i64>>(0))
                .collect::<Vec<_>>();
            let plan: Value = transaction.query_one(PLAN, &[values])?.get(0);
            Ok((json!({ "values": ordered }), plan))
        }
    }
}

fn validate_runtime_identity(
    backend: &BackendIdentity,
    postgresql_version: &str,
    postgis_full_version: &str,
) -> Result<(), PostgisError> {
    if !postgresql_version.starts_with(&backend.engine_version) {
        return Err(PostgisError::RuntimeIdentity(format!(
            "expected PostgreSQL {}, observed {postgresql_version}",
            backend.engine_version
        )));
    }
    for (component, version) in &backend.components {
        let found = match component.as_str() {
            "postgis" => postgis_full_version.contains(&format!("POSTGIS=\"{version}")),
            "geos" => postgis_full_version.contains(&format!("GEOS=\"{version}")),
            "proj" => postgis_full_version.contains(&format!("PROJ=\"{version}")),
            _ => true,
        };
        if !found {
            return Err(PostgisError::RuntimeIdentity(format!(
                "expected {component} {version}, observed {postgis_full_version}"
            )));
        }
    }
    Ok(())
}

fn digest_json(value: &Value) -> Result<String, serde_json::Error> {
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(value)?)
    ))
}

/// Return the reviewed manifest for PostgreSQL 18.6 and PostGIS 3.6.4.
pub fn postgis_manifest() -> AdapterManifest {
    let hooks = BTreeSet::from([
        EvidenceHook::InputDigests,
        EvidenceHook::OutputDigests,
        EvidenceHook::Parameters,
        EvidenceHook::QueryPlanDigest,
        EvidenceHook::ComponentIdentity,
        EvidenceHook::EnvironmentDigest,
        EvidenceHook::TransactionContext,
    ]);
    let operation = |operation_id: &str, inputs: Vec<GeoContract>, outputs: Vec<GeoContract>| {
        AdapterOperation {
            operation_id: operation_id.into(),
            operation_version: "1.0.0".into(),
            inputs,
            outputs,
            capabilities: BTreeSet::from([Capability::DatabaseRead]),
            determinism: Determinism::ToleranceBounded,
            evidence_hooks: hooks.clone(),
            opaque: false,
        }
    };
    AdapterManifest {
        schema_version: ADAPTER_MANIFEST_SCHEMA_VERSION.into(),
        adapter_id: ADAPTER_ID.into(),
        adapter_version: ADAPTER_VERSION.into(),
        backend: BackendIdentity {
            family: BackendFamily::Postgis,
            engine_version: "18.6".into(),
            build_digest: POSTGIS_IMAGE_DIGEST.into(),
            components: BTreeMap::from([
                ("postgis".into(), "3.6.4".into()),
                ("geos".into(), "3.13.1".into()),
                ("proj".into(), "9.6.0".into()),
            ]),
        },
        license: "PostgreSQL AND GPL-2.0-or-later".into(),
        operations: vec![
            operation(
                "postgis.area.projected",
                vec![polygon_contract(
                    "postgis.area.input",
                    Crs::nagoya_projected(),
                )],
                vec![measure_contract(
                    "postgis.area.output",
                    MeasureKind::Area,
                    "m2",
                )],
            ),
            operation(
                "postgis.transform.point",
                vec![point_contract("postgis.transform.input", Crs::wgs84())],
                vec![point_contract(
                    "postgis.transform.output",
                    Crs::nagoya_projected(),
                )],
            ),
            operation(
                "postgis.intersects",
                vec![
                    polygon_contract("postgis.intersects.left", Crs::nagoya_projected()),
                    polygon_contract("postgis.intersects.right", Crs::nagoya_projected()),
                ],
                vec![measure_contract(
                    "postgis.intersects.output",
                    MeasureKind::Category,
                    "boolean",
                )],
            ),
            operation(
                "postgis.multipart.part-count",
                vec![polygon_contract(
                    "postgis.multipart.input",
                    Crs::nagoya_projected(),
                )],
                vec![measure_contract(
                    "postgis.multipart.output",
                    MeasureKind::Count,
                    "parts",
                )],
            ),
            operation(
                "postgis.null-ordering",
                vec![measure_contract(
                    "postgis.null-ordering.input",
                    MeasureKind::Count,
                    "integer",
                )],
                vec![measure_contract(
                    "postgis.null-ordering.output",
                    MeasureKind::Count,
                    "integer",
                )],
            ),
        ],
    }
}

fn polygon_contract(id: &str, crs: Crs) -> GeoContract {
    GeoContract::new(id)
        .with_spatial(SpatialContract::known(
            GeometryKind::Polygon,
            crs,
            AxisOrder::Xy,
        ))
        .with_measure(MeasureContract::simple(
            MeasureKind::Geometry,
            "geometry",
            AggregationBasis::None,
        ))
}

fn point_contract(id: &str, crs: Crs) -> GeoContract {
    let axis_order = if crs == Crs::wgs84() {
        AxisOrder::LongitudeLatitude
    } else {
        AxisOrder::Xy
    };
    GeoContract::new(id)
        .with_spatial(SpatialContract::known(GeometryKind::Point, crs, axis_order))
        .with_measure(MeasureContract::simple(
            MeasureKind::Geometry,
            "geometry",
            AggregationBasis::None,
        ))
}

fn measure_contract(id: &str, kind: MeasureKind, unit: &str) -> GeoContract {
    GeoContract::new(id).with_measure(MeasureContract::simple(kind, unit, AggregationBasis::None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_postgis_manifest_has_five_typed_read_only_operations() {
        let manifest = postgis_manifest();
        assert!(manifest.validate().is_empty());
        assert_eq!(manifest.operations.len(), 5);
        assert!(manifest.operations.iter().all(|operation| {
            operation.capabilities == BTreeSet::from([Capability::DatabaseRead])
                && !operation.opaque
                && operation
                    .evidence_hooks
                    .contains(&EvidenceHook::QueryPlanDigest)
        }));
        assert!(manifest.digest().unwrap().starts_with("sha256:"));
    }

    #[test]
    fn operation_json_rejects_arbitrary_sql_field() {
        let value = json!({
            "operation": "projected_area",
            "wkt": "POLYGON((0 0,1 0,1 1,0 1,0 0))",
            "srid": 6675,
            "sql": "DROP TABLE important"
        });
        assert!(serde_json::from_value::<PostgisOperation>(value).is_err());
    }
}
