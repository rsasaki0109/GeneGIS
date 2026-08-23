//! Revisioned vector editing state used only through the command boundary.

use genegis_crs::{CoordinateUnit, Crs};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;
use uuid::Uuid;

/// Scalar attribute values supported by the core edit store.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum AttributeValue {
    /// Explicit database-style null.
    Null,
    /// Boolean value.
    Boolean(bool),
    /// Signed integer value.
    Integer(i64),
    /// Finite floating-point value.
    Number(f64),
    /// UTF-8 text value.
    Text(String),
}

/// Attribute storage type in an editable layer schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributeType {
    /// Boolean value.
    Boolean,
    /// Signed 64-bit integer.
    Integer,
    /// Finite 64-bit number.
    Number,
    /// UTF-8 text.
    Text,
}

/// One field in a closed editable schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttributeField {
    /// Stable field name.
    pub name: String,
    /// Required scalar type.
    pub value_type: AttributeType,
    /// Whether an explicit null is accepted.
    pub nullable: bool,
}

/// Closed attribute schema; undeclared fields are rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureSchema {
    /// Stable primary-key field, which must be non-null text or integer.
    pub primary_key: String,
    /// Declared fields in canonical name order.
    pub fields: Vec<AttributeField>,
}

/// Geometry family enforced for every feature in an editable layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditableGeometryKind {
    /// Points only.
    Point,
    /// Line strings only.
    LineString,
    /// Polygon and multipolygon geometry.
    Polygon,
}

/// Format-independent editable geometry with explicit coordinate arrays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "coordinates", rename_all = "snake_case")]
pub enum FeatureGeometry {
    /// One XY coordinate.
    Point([f64; 2]),
    /// At least two XY coordinates.
    LineString(Vec<[f64; 2]>),
    /// Rings, exterior first and holes afterwards.
    Polygon(Vec<Vec<[f64; 2]>>),
    /// Polygons, each containing exterior then interior rings.
    MultiPolygon(Vec<Vec<Vec<[f64; 2]>>>),
}

impl FeatureGeometry {
    fn kind(&self) -> EditableGeometryKind {
        match self {
            Self::Point(_) => EditableGeometryKind::Point,
            Self::LineString(_) => EditableGeometryKind::LineString,
            Self::Polygon(_) | Self::MultiPolygon(_) => EditableGeometryKind::Polygon,
        }
    }

    fn coordinates(&self) -> Vec<[f64; 2]> {
        match self {
            Self::Point(point) => vec![*point],
            Self::LineString(points) => points.clone(),
            Self::Polygon(rings) => rings.iter().flatten().copied().collect(),
            Self::MultiPolygon(polygons) => polygons.iter().flatten().flatten().copied().collect(),
        }
    }

    fn area(&self) -> f64 {
        fn ring_area(ring: &[[f64; 2]]) -> f64 {
            ring.windows(2)
                .map(|edge| edge[0][0] * edge[1][1] - edge[1][0] * edge[0][1])
                .sum::<f64>()
                .abs()
                / 2.0
        }
        fn polygon_area(rings: &[Vec<[f64; 2]>]) -> f64 {
            rings.first().map_or(0.0, |ring| ring_area(ring))
                - rings
                    .iter()
                    .skip(1)
                    .map(|ring| ring_area(ring))
                    .sum::<f64>()
        }
        match self {
            Self::Polygon(rings) => polygon_area(rings),
            Self::MultiPolygon(polygons) => polygons.iter().map(|rings| polygon_area(rings)).sum(),
            Self::Point(_) | Self::LineString(_) => 0.0,
        }
    }

    fn repair(self) -> Self {
        fn repair_ring(mut ring: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
            ring.dedup();
            if let Some(first) = ring.first().copied() {
                if ring.last().copied() != Some(first) {
                    ring.push(first);
                }
            }
            ring
        }
        match self {
            Self::Polygon(rings) => Self::Polygon(rings.into_iter().map(repair_ring).collect()),
            Self::MultiPolygon(polygons) => Self::MultiPolygon(
                polygons
                    .into_iter()
                    .map(|rings| rings.into_iter().map(repair_ring).collect())
                    .collect(),
            ),
            other => other,
        }
    }

    fn polygons(&self) -> Option<Vec<Vec<Vec<[f64; 2]>>>> {
        match self {
            Self::Polygon(rings) => Some(vec![rings.clone()]),
            Self::MultiPolygon(polygons) => Some(polygons.clone()),
            Self::Point(_) | Self::LineString(_) => None,
        }
    }
}

/// One revisioned feature.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditableFeature {
    /// Stable feature identity independent of attribute primary keys.
    pub id: String,
    /// Optimistic-concurrency revision.
    pub revision: u64,
    /// Validated geometry.
    pub geometry: FeatureGeometry,
    /// Closed-schema scalar attributes.
    pub attributes: BTreeMap<String, AttributeValue>,
}

/// A draft used when creating or replacing a feature.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureDraft {
    /// Stable feature identity.
    pub id: String,
    /// Proposed geometry.
    pub geometry: FeatureGeometry,
    /// Proposed attributes.
    pub attributes: BTreeMap<String, AttributeValue>,
}

/// Deterministic, typed editing operations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum FeatureEdit {
    /// Insert a new feature.
    Create { feature: FeatureDraft },
    /// Replace geometry and attributes after checking the feature revision.
    Update {
        feature: FeatureDraft,
        expected_feature_revision: u64,
    },
    /// Delete a feature after checking the feature revision.
    Delete {
        feature_id: String,
        expected_feature_revision: u64,
    },
    /// Replace one polygon by explicitly supplied area-preserving parts.
    Split {
        feature_id: String,
        expected_feature_revision: u64,
        parts: Vec<FeatureDraft>,
    },
    /// Collect polygon members into one multipolygon without silently dissolving topology.
    Merge {
        feature_ids: Vec<String>,
        expected_feature_revisions: Vec<u64>,
        merged: FeatureDraft,
    },
    /// Close rings and remove consecutive duplicate vertices.
    Repair {
        feature_id: String,
        expected_feature_revision: u64,
    },
}

impl FeatureEdit {
    /// Stable workflow operation required to authorize this edit.
    pub fn workflow_operation(&self) -> &'static str {
        match self {
            Self::Create { .. } => "EditFeatureCreate",
            Self::Update { .. } => "EditFeatureUpdate",
            Self::Delete { .. } => "EditFeatureDelete",
            Self::Split { .. } => "EditFeatureSplit",
            Self::Merge { .. } => "EditFeatureMerge",
            Self::Repair { .. } => "EditFeatureRepair",
        }
    }
}

/// Binding from a project mutation to its reviewed workflow node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MutationWorkflowBinding {
    /// Registered workflow execution identity.
    pub workflow_id: Uuid,
    /// Stable node ID inside the graph.
    pub node_id: String,
}

/// Revisioned feature state attached to a semantic project layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditableLayer {
    /// Existing project layer identity.
    pub layer_id: Uuid,
    /// Explicit CRS for all geometry.
    pub crs: Crs,
    /// Coordinate unit derived from the CRS.
    pub coordinate_unit: CoordinateUnit,
    /// Accepted geometry family.
    pub geometry_kind: EditableGeometryKind,
    /// Closed feature schema.
    pub schema: FeatureSchema,
    /// Layer-level optimistic-concurrency revision.
    pub revision: u64,
    /// Features sorted by stable identity.
    pub features: BTreeMap<String, EditableFeature>,
}

/// Summary written into command provenance after an edit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditReceipt {
    /// Layer revision before mutation.
    pub before_revision: u64,
    /// Layer revision after mutation.
    pub after_revision: u64,
    /// Sorted identities changed by the operation.
    pub changed_features: Vec<String>,
}

/// Fail-closed edit validation failure.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum EditError {
    /// Layer and feature revisions prevent lost updates.
    #[error("stale revision: expected {expected}, actual {actual}")]
    StaleRevision { expected: u64, actual: u64 },
    /// A requested feature does not exist.
    #[error("feature not found: {0}")]
    FeatureNotFound(String),
    /// A proposed identity already exists or occurs more than once.
    #[error("duplicate feature identity: {0}")]
    DuplicateFeature(String),
    /// Geometry violates CRS, family, ring, or area requirements.
    #[error("invalid geometry: {0}")]
    Geometry(String),
    /// Attributes violate the closed layer schema.
    #[error("invalid attributes: {0}")]
    Attributes(String),
    /// Split/merge conservation checks failed.
    #[error("topology conservation failed: {0}")]
    Conservation(String),
    /// Layer schema or CRS is invalid.
    #[error("invalid editable layer: {0}")]
    Layer(String),
}

impl EditableLayer {
    /// Construct an empty editable layer after validating CRS and schema.
    pub fn new(
        layer_id: Uuid,
        crs: Crs,
        geometry_kind: EditableGeometryKind,
        schema: FeatureSchema,
    ) -> Result<Self, EditError> {
        crs.require_known()
            .map_err(|error| EditError::Layer(error.to_string()))?;
        validate_schema(&schema)?;
        Ok(Self {
            layer_id,
            coordinate_unit: crs.coordinate_unit(),
            crs,
            geometry_kind,
            schema,
            revision: 0,
            features: BTreeMap::new(),
        })
    }

    /// Apply one validated edit with optimistic concurrency.
    pub fn apply(
        &mut self,
        expected_layer_revision: u64,
        edit: FeatureEdit,
    ) -> Result<EditReceipt, EditError> {
        if self.revision != expected_layer_revision {
            return Err(EditError::StaleRevision {
                expected: expected_layer_revision,
                actual: self.revision,
            });
        }
        let before_revision = self.revision;
        let changed_features = match edit {
            FeatureEdit::Create { feature } => {
                self.validate_draft(&feature)?;
                if self.features.contains_key(&feature.id) {
                    return Err(EditError::DuplicateFeature(feature.id));
                }
                let id = feature.id.clone();
                self.features.insert(id.clone(), into_feature(feature, 1));
                vec![id]
            }
            FeatureEdit::Update {
                feature,
                expected_feature_revision,
            } => {
                self.validate_draft(&feature)?;
                let previous =
                    self.feature_with_revision(&feature.id, expected_feature_revision)?;
                let id = feature.id.clone();
                self.features.insert(
                    id.clone(),
                    into_feature(feature, previous.revision.saturating_add(1)),
                );
                vec![id]
            }
            FeatureEdit::Delete {
                feature_id,
                expected_feature_revision,
            } => {
                self.feature_with_revision(&feature_id, expected_feature_revision)?;
                self.features.remove(&feature_id);
                vec![feature_id]
            }
            FeatureEdit::Split {
                feature_id,
                expected_feature_revision,
                parts,
            } => self.apply_split(&feature_id, expected_feature_revision, parts)?,
            FeatureEdit::Merge {
                feature_ids,
                expected_feature_revisions,
                mut merged,
            } => self.apply_merge(feature_ids, expected_feature_revisions, &mut merged)?,
            FeatureEdit::Repair {
                feature_id,
                expected_feature_revision,
            } => {
                let previous = self
                    .feature_with_revision(&feature_id, expected_feature_revision)?
                    .clone();
                let repaired = FeatureDraft {
                    id: previous.id.clone(),
                    geometry: previous.geometry.repair(),
                    attributes: previous.attributes,
                };
                self.validate_draft(&repaired)?;
                self.features.insert(
                    feature_id.clone(),
                    into_feature(repaired, previous.revision.saturating_add(1)),
                );
                vec![feature_id]
            }
        };
        self.revision = self.revision.saturating_add(1);
        Ok(EditReceipt {
            before_revision,
            after_revision: self.revision,
            changed_features,
        })
    }

    fn feature_with_revision(
        &self,
        id: &str,
        expected: u64,
    ) -> Result<&EditableFeature, EditError> {
        let feature = self
            .features
            .get(id)
            .ok_or_else(|| EditError::FeatureNotFound(id.into()))?;
        if feature.revision != expected {
            return Err(EditError::StaleRevision {
                expected,
                actual: feature.revision,
            });
        }
        Ok(feature)
    }

    fn validate_draft(&self, draft: &FeatureDraft) -> Result<(), EditError> {
        if draft.id.trim().is_empty() || draft.id.len() > 256 {
            return Err(EditError::DuplicateFeature(draft.id.clone()));
        }
        validate_geometry(&draft.geometry, self.geometry_kind, &self.crs)?;
        validate_attributes(&draft.attributes, &self.schema)
    }

    fn apply_split(
        &mut self,
        feature_id: &str,
        expected_revision: u64,
        parts: Vec<FeatureDraft>,
    ) -> Result<Vec<String>, EditError> {
        let source = self
            .feature_with_revision(feature_id, expected_revision)?
            .clone();
        if parts.len() < 2 {
            return Err(EditError::Conservation(
                "a split requires at least two output features".into(),
            ));
        }
        let mut ids = BTreeSet::new();
        for part in &parts {
            self.validate_draft(part)?;
            if !ids.insert(part.id.clone())
                || (part.id != feature_id && self.features.contains_key(&part.id))
            {
                return Err(EditError::DuplicateFeature(part.id.clone()));
            }
        }
        let output_area = parts.iter().map(|part| part.geometry.area()).sum::<f64>();
        require_area_conservation(source.geometry.area(), output_area)?;
        self.features.remove(feature_id);
        for part in parts {
            self.features
                .insert(part.id.clone(), into_feature(part, source.revision + 1));
        }
        Ok(ids.into_iter().collect())
    }

    fn apply_merge(
        &mut self,
        feature_ids: Vec<String>,
        expected_revisions: Vec<u64>,
        merged: &mut FeatureDraft,
    ) -> Result<Vec<String>, EditError> {
        if feature_ids.len() < 2 || feature_ids.len() != expected_revisions.len() {
            return Err(EditError::Conservation(
                "merge requires at least two identities and one revision per identity".into(),
            ));
        }
        let ids = feature_ids.iter().cloned().collect::<BTreeSet<_>>();
        if ids.len() != feature_ids.len() {
            return Err(EditError::DuplicateFeature("merge input".into()));
        }
        let mut polygons = Vec::new();
        let mut input_area = 0.0;
        let mut max_revision = 0;
        for (id, revision) in feature_ids.iter().zip(expected_revisions) {
            let feature = self.feature_with_revision(id, revision)?;
            let members = feature.geometry.polygons().ok_or_else(|| {
                EditError::Conservation("merge supports polygonal geometry only".into())
            })?;
            polygons.extend(members);
            input_area += feature.geometry.area();
            max_revision = max_revision.max(feature.revision);
        }
        merged.geometry = FeatureGeometry::MultiPolygon(polygons);
        self.validate_draft(merged)?;
        require_area_conservation(input_area, merged.geometry.area())?;
        if !ids.contains(&merged.id) && self.features.contains_key(&merged.id) {
            return Err(EditError::DuplicateFeature(merged.id.clone()));
        }
        for id in &ids {
            self.features.remove(id);
        }
        self.features.insert(
            merged.id.clone(),
            into_feature(merged.clone(), max_revision.saturating_add(1)),
        );
        let mut changed = ids.into_iter().collect::<Vec<_>>();
        if !changed.contains(&merged.id) {
            changed.push(merged.id.clone());
            changed.sort();
        }
        Ok(changed)
    }
}

fn into_feature(draft: FeatureDraft, revision: u64) -> EditableFeature {
    EditableFeature {
        id: draft.id,
        revision,
        geometry: draft.geometry,
        attributes: draft.attributes,
    }
}

fn validate_schema(schema: &FeatureSchema) -> Result<(), EditError> {
    if schema.primary_key.trim().is_empty() || schema.fields.is_empty() {
        return Err(EditError::Layer(
            "schema needs a primary key and at least one field".into(),
        ));
    }
    let names = schema
        .fields
        .iter()
        .map(|field| field.name.as_str())
        .collect::<BTreeSet<_>>();
    if names.len() != schema.fields.len() || !names.contains(schema.primary_key.as_str()) {
        return Err(EditError::Layer(
            "schema field names must be unique and include the primary key".into(),
        ));
    }
    let primary_key = schema
        .fields
        .iter()
        .find(|field| field.name == schema.primary_key)
        .expect("checked primary key");
    if primary_key.nullable
        || !matches!(
            primary_key.value_type,
            AttributeType::Integer | AttributeType::Text
        )
    {
        return Err(EditError::Layer(
            "primary key must be non-null text or integer".into(),
        ));
    }
    Ok(())
}

fn validate_attributes(
    values: &BTreeMap<String, AttributeValue>,
    schema: &FeatureSchema,
) -> Result<(), EditError> {
    if values.len() != schema.fields.len() {
        return Err(EditError::Attributes(
            "attributes must exactly match the closed schema".into(),
        ));
    }
    for field in &schema.fields {
        let value = values
            .get(&field.name)
            .ok_or_else(|| EditError::Attributes(format!("missing field {}", field.name)))?;
        let matches = match value {
            AttributeValue::Null => field.nullable,
            AttributeValue::Boolean(_) => field.value_type == AttributeType::Boolean,
            AttributeValue::Integer(_) => field.value_type == AttributeType::Integer,
            AttributeValue::Number(number) => {
                field.value_type == AttributeType::Number && number.is_finite()
            }
            AttributeValue::Text(_) => field.value_type == AttributeType::Text,
        };
        if !matches {
            return Err(EditError::Attributes(format!(
                "field {} violates type or null policy",
                field.name
            )));
        }
    }
    Ok(())
}

fn validate_geometry(
    geometry: &FeatureGeometry,
    required_kind: EditableGeometryKind,
    crs: &Crs,
) -> Result<(), EditError> {
    if geometry.kind() != required_kind {
        return Err(EditError::Geometry("geometry family mismatch".into()));
    }
    let coordinates = geometry.coordinates();
    if coordinates.is_empty()
        || coordinates.iter().any(|point| {
            !point[0].is_finite()
                || !point[1].is_finite()
                || crs.validate_coordinate(point[0], point[1]).is_err()
        })
    {
        return Err(EditError::Geometry(
            "coordinates are empty, non-finite, or outside the CRS domain".into(),
        ));
    }
    match geometry {
        FeatureGeometry::Point(_) => {}
        FeatureGeometry::LineString(points) if points.len() >= 2 => {}
        FeatureGeometry::LineString(_) => {
            return Err(EditError::Geometry(
                "line strings require at least two coordinates".into(),
            ));
        }
        FeatureGeometry::Polygon(rings) => validate_rings(rings)?,
        FeatureGeometry::MultiPolygon(polygons) if !polygons.is_empty() => {
            for rings in polygons {
                validate_rings(rings)?;
            }
        }
        FeatureGeometry::MultiPolygon(_) => {
            return Err(EditError::Geometry("multipolygon is empty".into()));
        }
    }
    Ok(())
}

fn validate_rings(rings: &[Vec<[f64; 2]>]) -> Result<(), EditError> {
    if rings.is_empty()
        || rings
            .iter()
            .any(|ring| ring.len() < 4 || ring.first() != ring.last())
    {
        return Err(EditError::Geometry(
            "polygon rings must be closed and contain at least four coordinates".into(),
        ));
    }
    let geometry = FeatureGeometry::Polygon(rings.to_vec());
    if geometry.area() <= 0.0 || !geometry.area().is_finite() {
        return Err(EditError::Geometry(
            "polygon area must be finite and positive after holes".into(),
        ));
    }
    Ok(())
}

fn require_area_conservation(input: f64, output: f64) -> Result<(), EditError> {
    let relative = (input - output).abs() / input.abs().max(output.abs()).max(f64::EPSILON);
    if relative > 1e-6 {
        return Err(EditError::Conservation(format!(
            "area changed from {input} to {output}; tolerance is 1 ppm"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> FeatureSchema {
        FeatureSchema {
            primary_key: "code".into(),
            fields: vec![AttributeField {
                name: "code".into(),
                value_type: AttributeType::Text,
                nullable: false,
            }],
        }
    }

    fn feature(id: &str, min_x: f64, max_x: f64) -> FeatureDraft {
        FeatureDraft {
            id: id.into(),
            geometry: FeatureGeometry::Polygon(vec![vec![
                [min_x, 0.0],
                [max_x, 0.0],
                [max_x, 10.0],
                [min_x, 10.0],
                [min_x, 0.0],
            ]]),
            attributes: BTreeMap::from([("code".into(), AttributeValue::Text(id.into()))]),
        }
    }

    #[test]
    fn create_split_merge_and_stale_revision_fail_closed() {
        let mut layer = EditableLayer::new(
            Uuid::nil(),
            Crs::nagoya_projected(),
            EditableGeometryKind::Polygon,
            schema(),
        )
        .unwrap();
        layer
            .apply(
                0,
                FeatureEdit::Create {
                    feature: feature("a", 0.0, 10.0),
                },
            )
            .unwrap();
        let split = FeatureEdit::Split {
            feature_id: "a".into(),
            expected_feature_revision: 1,
            parts: vec![feature("a1", 0.0, 5.0), feature("a2", 5.0, 10.0)],
        };
        layer.apply(1, split).unwrap();
        assert!(matches!(
            layer.apply(
                1,
                FeatureEdit::Delete {
                    feature_id: "a1".into(),
                    expected_feature_revision: 2
                }
            ),
            Err(EditError::StaleRevision { .. })
        ));
        layer
            .apply(
                2,
                FeatureEdit::Merge {
                    feature_ids: vec!["a1".into(), "a2".into()],
                    expected_feature_revisions: vec![2, 2],
                    merged: feature("a", 0.0, 10.0),
                },
            )
            .unwrap();
        assert!(matches!(
            layer.features["a"].geometry,
            FeatureGeometry::MultiPolygon(_)
        ));
    }

    #[test]
    fn invalid_ring_schema_null_and_nonconserving_split_are_rejected() {
        let mut layer = EditableLayer::new(
            Uuid::nil(),
            Crs::nagoya_projected(),
            EditableGeometryKind::Polygon,
            schema(),
        )
        .unwrap();
        let mut invalid = feature("bad", 0.0, 10.0);
        let FeatureGeometry::Polygon(rings) = &mut invalid.geometry else {
            unreachable!()
        };
        rings[0].pop();
        assert!(matches!(
            layer.apply(0, FeatureEdit::Create { feature: invalid }),
            Err(EditError::Geometry(_))
        ));
        layer
            .apply(
                0,
                FeatureEdit::Create {
                    feature: feature("a", 0.0, 10.0),
                },
            )
            .unwrap();
        let bad_split = FeatureEdit::Split {
            feature_id: "a".into(),
            expected_feature_revision: 1,
            parts: vec![feature("a1", 0.0, 4.0), feature("a2", 5.0, 10.0)],
        };
        assert!(matches!(
            layer.apply(1, bad_split),
            Err(EditError::Conservation(_))
        ));
    }
}
