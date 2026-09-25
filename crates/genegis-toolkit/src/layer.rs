//! General-purpose feature layer with explicit CRS, field units, and lineage.

use std::collections::BTreeMap;

use geo::BoundingRect;
use geo_types::Geometry;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::Result;
use crate::proj::{self, CrsInfo};
use crate::wkb;

/// How the layer's CRS became known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrsStatus {
    /// Declared by the source file (`.prj`, GeoPackage SRS, GeoParquet metadata).
    Declared,
    /// Implied by the format specification (RFC 7946 GeoJSON is EPSG:4326).
    FormatDefault,
    /// Inferred from coordinate ranges; the user should confirm it.
    Inferred,
    /// Supplied explicitly by the user at import time.
    UserSupplied,
    /// Produced by a toolkit operation from inputs with known CRS.
    Derived,
}

impl CrsStatus {
    /// Whether the CRS should be confirmed by a person before relying on it.
    pub fn needs_confirmation(self) -> bool {
        matches!(self, Self::Inferred)
    }
}

/// Attribute value type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    /// 64-bit integer.
    Integer,
    /// 64-bit float.
    Float,
    /// Boolean.
    Boolean,
    /// UTF-8 text.
    Text,
    /// Nested JSON (arrays/objects), stored as text in columnar formats.
    Json,
}

/// Attribute schema entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field {
    /// Column name.
    pub name: String,
    /// Inferred or declared type.
    #[serde(rename = "type")]
    pub field_type: FieldType,
    /// Measurement unit, when known (for example `m²` or `persons`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

/// One feature. Geometry may be absent for pure attribute tables.
#[derive(Debug, Clone, PartialEq)]
pub struct Feature {
    /// Stable feature ID within the layer.
    pub id: u64,
    /// Geometry in the layer CRS.
    pub geometry: Option<Geometry<f64>>,
    /// Attribute values keyed by field name.
    pub properties: BTreeMap<String, Value>,
}

/// Source identity and lineage of a layer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LayerProvenance {
    /// Where the bytes came from (`upload://file.csv`, `https://…`, `layer://…`).
    pub source_uri: String,
    /// `sha256:` digest of the original source bytes, when imported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_sha256: Option<String>,
    /// Declared license or attribution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Attribution text that must accompany maps and exports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution: Option<String>,
    /// Source format (`geojson`, `csv`, `shapefile`, …) or `derived`.
    pub format: String,
    /// Retrieval or import instant (RFC 3339). Not part of the content digest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retrieved_at: Option<String>,
    /// Digests of parent layers for derived results.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parents: Vec<String>,
    /// Operation that produced a derived layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    /// Workflow digest that produced a derived layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_digest: Option<String>,
    /// Assumptions and normalisation notes made while importing or deriving.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// A feature layer.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    /// Display name.
    pub name: String,
    /// `EPSG:<code>` identifier of a supported CRS.
    pub crs: String,
    /// How the CRS became known.
    pub crs_status: CrsStatus,
    /// Attribute schema in column order.
    pub fields: Vec<Field>,
    /// Features in stable order.
    pub features: Vec<Feature>,
    /// Source and lineage.
    pub provenance: LayerProvenance,
}

/// Geometry family summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeometryKind {
    /// Points or multipoints only.
    Point,
    /// Lines or multilines only.
    Line,
    /// Polygons or multipolygons only.
    Polygon,
    /// Mixed families.
    Mixed,
    /// No geometry (attribute table).
    None,
}

/// Serializable layer summary (no feature payload).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerSummary {
    /// Content-derived layer ID.
    pub id: String,
    /// Content digest.
    pub digest: String,
    /// Display name.
    pub name: String,
    /// CRS identifier.
    pub crs: String,
    /// CRS name.
    pub crs_name: String,
    /// Axis unit.
    pub crs_unit: proj::AxisUnit,
    /// How the CRS became known.
    pub crs_status: CrsStatus,
    /// Whether a person should confirm the CRS.
    pub crs_needs_confirmation: bool,
    /// Geometry family.
    pub geometry_kind: GeometryKind,
    /// Number of features.
    pub feature_count: usize,
    /// IDs of features whose polygons are invalid (self-intersections,
    /// crossing rings); repair them with the `make_valid` operation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub invalid_feature_ids: Vec<u64>,
    /// `[min_x, min_y, max_x, max_y]` in the layer CRS.
    pub bbox: Option<[f64; 4]>,
    /// `[min_lon, min_lat, max_lon, max_lat]` in EPSG:4326.
    pub bbox_wgs84: Option<[f64; 4]>,
    /// Attribute schema.
    pub fields: Vec<Field>,
    /// Source and lineage.
    pub provenance: LayerProvenance,
}

impl Layer {
    /// Construct an empty layer.
    pub fn new(name: impl Into<String>, crs: impl Into<String>, crs_status: CrsStatus) -> Self {
        Self {
            name: name.into(),
            crs: crs.into(),
            crs_status,
            fields: Vec::new(),
            features: Vec::new(),
            provenance: LayerProvenance::default(),
        }
    }

    /// Resolve the layer CRS against the built-in registry.
    pub fn crs_info(&self) -> Result<CrsInfo> {
        proj::lookup(&self.crs)
    }

    /// Canonical SHA-256 content digest over CRS, schema, geometry, and
    /// attributes. Names, timestamps, and provenance notes are excluded so
    /// the same content always has the same identity.
    pub fn digest(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"genegis-layer-v1\0");
        hasher.update(self.crs.as_bytes());
        hasher.update([0]);
        for field in &self.fields {
            hasher.update(field.name.as_bytes());
            hasher.update([0]);
            hasher.update(format!("{:?}", field.field_type).as_bytes());
            hasher.update([0]);
            hasher.update(field.unit.as_deref().unwrap_or("").as_bytes());
            hasher.update([0]);
        }
        for feature in &self.features {
            hasher.update(feature.id.to_le_bytes());
            match &feature.geometry {
                Some(geometry) => hasher.update(wkb::write(geometry)),
                None => hasher.update(b"\xffnull"),
            }
            hasher.update(canonical_json(&Value::Object(
                feature
                    .properties
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            )));
            hasher.update([0]);
        }
        format!("sha256:{:x}", hasher.finalize())
    }

    /// Short content-derived identifier.
    pub fn id(&self) -> String {
        layer_id_from_digest(&self.digest())
    }

    /// Bounding box in the layer CRS.
    pub fn bbox(&self) -> Option<[f64; 4]> {
        let mut acc: Option<[f64; 4]> = None;
        for geometry in self.features.iter().filter_map(|f| f.geometry.as_ref()) {
            if let Some(rect) = geometry.bounding_rect() {
                let (min, max) = (rect.min(), rect.max());
                acc = Some(match acc {
                    None => [min.x, min.y, max.x, max.y],
                    Some(b) => [
                        b[0].min(min.x),
                        b[1].min(min.y),
                        b[2].max(max.x),
                        b[3].max(max.y),
                    ],
                });
            }
        }
        acc
    }

    /// Bounding box in EPSG:4326, computed from the transformed corners and
    /// edge midpoints.
    pub fn bbox_wgs84(&self) -> Option<[f64; 4]> {
        let bbox = self.bbox()?;
        let crs = self.crs_info().ok()?;
        let mut out: Option<[f64; 4]> = None;
        let xs = [bbox[0], (bbox[0] + bbox[2]) / 2.0, bbox[2]];
        let ys = [bbox[1], (bbox[1] + bbox[3]) / 2.0, bbox[3]];
        for x in xs {
            for y in ys {
                let (lon, lat) = proj::to_geographic(&crs, x, y).ok()?;
                out = Some(match out {
                    None => [lon, lat, lon, lat],
                    Some(b) => [b[0].min(lon), b[1].min(lat), b[2].max(lon), b[3].max(lat)],
                });
            }
        }
        out
    }

    /// Geometry family of the layer.
    pub fn geometry_kind(&self) -> GeometryKind {
        let mut kind: Option<GeometryKind> = None;
        for geometry in self.features.iter().filter_map(|f| f.geometry.as_ref()) {
            let this = geometry_kind(geometry);
            kind = Some(match kind {
                None => this,
                Some(existing) if existing == this => existing,
                Some(_) => GeometryKind::Mixed,
            });
        }
        kind.unwrap_or(GeometryKind::None)
    }

    /// IDs of features with invalid polygon geometry (OGC validity).
    pub fn invalid_feature_ids(&self) -> Vec<u64> {
        use geo::Validation;
        self.features
            .iter()
            .filter(|f| match &f.geometry {
                Some(g @ (Geometry::Polygon(_) | Geometry::MultiPolygon(_))) => !g.is_valid(),
                _ => false,
            })
            .map(|f| f.id)
            .collect()
    }

    /// Look up a field by name.
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|field| field.name == name)
    }

    /// Recompute the schema from the feature properties, preserving units and
    /// column order of existing fields.
    pub fn refresh_schema(&mut self) {
        let mut order: Vec<String> = self.fields.iter().map(|f| f.name.clone()).collect();
        for feature in &self.features {
            for key in feature.properties.keys() {
                if !order.contains(key) {
                    order.push(key.clone());
                }
            }
        }
        let previous: BTreeMap<String, Field> = self
            .fields
            .iter()
            .map(|f| (f.name.clone(), f.clone()))
            .collect();
        self.fields = order
            .into_iter()
            .map(|name| {
                let mut values = self
                    .features
                    .iter()
                    .filter_map(|feature| feature.properties.get(&name))
                    .filter(|v| !v.is_null())
                    .peekable();
                // With no values left (e.g. an empty selection) keep the
                // declared type instead of degrading it to text.
                let field_type = match (values.peek(), previous.get(&name)) {
                    (None, Some(field)) => field.field_type,
                    _ => infer_field_type(values),
                };
                Field {
                    unit: previous.get(&name).and_then(|f| f.unit.clone()),
                    name,
                    field_type,
                }
            })
            .collect();
    }

    /// Set the unit of an existing field.
    pub fn set_field_unit(&mut self, name: &str, unit: impl Into<String>) {
        if let Some(field) = self.fields.iter_mut().find(|field| field.name == name) {
            field.unit = Some(unit.into());
        }
    }

    /// Serializable summary.
    pub fn summary(&self) -> LayerSummary {
        let digest = self.digest();
        let crs = self.crs_info().ok();
        LayerSummary {
            id: layer_id_from_digest(&digest),
            digest,
            name: self.name.clone(),
            crs: self.crs.clone(),
            crs_name: crs.as_ref().map(|c| c.name.clone()).unwrap_or_default(),
            crs_unit: crs
                .as_ref()
                .map(|c| c.unit)
                .unwrap_or(proj::AxisUnit::Degrees),
            crs_status: self.crs_status,
            crs_needs_confirmation: self.crs_status.needs_confirmation(),
            geometry_kind: self.geometry_kind(),
            feature_count: self.features.len(),
            invalid_feature_ids: self.invalid_feature_ids(),
            bbox: self.bbox(),
            bbox_wgs84: self.bbox_wgs84(),
            fields: self.fields.clone(),
            provenance: self.provenance.clone(),
        }
    }

    /// Return a copy reprojected into `target`.
    pub fn reprojected(&self, target: &CrsInfo) -> Result<Layer> {
        let source = self.crs_info()?;
        let mut out = self.clone();
        out.crs = target.id.clone();
        if source.epsg != target.epsg {
            for feature in &mut out.features {
                if let Some(geometry) = &feature.geometry {
                    feature.geometry = Some(proj::transform_geometry(&source, target, geometry)?);
                }
            }
        }
        Ok(out)
    }
}

/// `lyr_<first 12 hex digits>` identifier derived from a content digest.
pub fn layer_id_from_digest(digest: &str) -> String {
    let hex = digest.strip_prefix("sha256:").unwrap_or(digest);
    format!("lyr_{}", &hex[..hex.len().min(12)])
}

/// Geometry family of one geometry.
pub fn geometry_kind(geometry: &Geometry<f64>) -> GeometryKind {
    match geometry {
        Geometry::Point(_) | Geometry::MultiPoint(_) => GeometryKind::Point,
        Geometry::Line(_) | Geometry::LineString(_) | Geometry::MultiLineString(_) => {
            GeometryKind::Line
        }
        Geometry::Polygon(_)
        | Geometry::MultiPolygon(_)
        | Geometry::Rect(_)
        | Geometry::Triangle(_) => GeometryKind::Polygon,
        Geometry::GeometryCollection(collection) => {
            let mut kind = None;
            for part in collection.iter() {
                let this = geometry_kind(part);
                kind = Some(match kind {
                    None => this,
                    Some(existing) if existing == this => existing,
                    Some(_) => GeometryKind::Mixed,
                });
            }
            kind.unwrap_or(GeometryKind::None)
        }
    }
}

/// Infer the narrowest field type that fits every non-null value.
pub fn infer_field_type<'a>(values: impl Iterator<Item = &'a Value>) -> FieldType {
    let mut kind: Option<FieldType> = None;
    for value in values {
        let this = match value {
            Value::Null => continue,
            Value::Bool(_) => FieldType::Boolean,
            Value::Number(n) if n.is_i64() || n.is_u64() => FieldType::Integer,
            Value::Number(_) => FieldType::Float,
            Value::String(_) => FieldType::Text,
            Value::Array(_) | Value::Object(_) => FieldType::Json,
        };
        kind = Some(match (kind, this) {
            (None, t) => t,
            (Some(a), b) if a == b => a,
            (Some(FieldType::Integer), FieldType::Float)
            | (Some(FieldType::Float), FieldType::Integer) => FieldType::Float,
            (Some(FieldType::Json), _) | (_, FieldType::Json) => FieldType::Json,
            _ => FieldType::Text,
        });
    }
    kind.unwrap_or(FieldType::Text)
}

/// Deterministic JSON serialization with sorted object keys.
pub fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by_key(|(key, _)| *key);
            let body = entries
                .into_iter()
                .map(|(k, v)| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap_or_default(),
                        canonical_json(v)
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{body}}}")
        }
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}

/// `sha256:` digest of bytes.
pub fn sha256_bytes(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo_types::point;

    fn sample() -> Layer {
        let mut layer = Layer::new("sample", "EPSG:4326", CrsStatus::Declared);
        layer.features.push(Feature {
            id: 0,
            geometry: Some(Geometry::Point(point!(x: 136.9, y: 35.1))),
            properties: BTreeMap::from([("n".to_string(), Value::from(1))]),
        });
        layer.refresh_schema();
        layer
    }

    #[test]
    fn digest_ignores_name_but_not_content() {
        let a = sample();
        let mut b = sample();
        b.name = "renamed".into();
        assert_eq!(a.digest(), b.digest());
        b.features[0].properties.insert("n".into(), Value::from(2));
        assert_ne!(a.digest(), b.digest());
    }

    #[test]
    fn empty_layers_keep_their_schema() {
        let mut layer = sample();
        layer.set_field_unit("n", "persons");
        layer.features.clear();
        layer.refresh_schema();
        let field = layer.field("n").expect("field survives");
        assert_eq!(field.field_type, FieldType::Integer);
        assert_eq!(field.unit.as_deref(), Some("persons"));
    }

    #[test]
    fn infers_numeric_widening_and_text_fallback() {
        let values = [Value::from(1), Value::from(1.5)];
        assert_eq!(infer_field_type(values.iter()), FieldType::Float);
        let mixed = [Value::from(1), Value::from("a")];
        assert_eq!(infer_field_type(mixed.iter()), FieldType::Text);
    }
}
