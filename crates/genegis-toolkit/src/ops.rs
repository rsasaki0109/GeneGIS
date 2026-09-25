//! Generic spatial operations.
//!
//! Every operation declares its input roles and parameters, requires explicit
//! units for distances, runs metric work in a metric CRS (never in degrees or
//! Web Mercator), and returns independent verification checks alongside the
//! output layer. An operation is a pure function of its inputs and
//! parameters, so the same graph always yields the same layer digest.

use std::collections::BTreeMap;

use geo::{
    Area, BooleanOps, BoundingRect, Buffer, Centroid, Distance, Euclidean, Geodesic, GeodesicArea,
    InteriorPoint, Intersects, Length, Relate,
};
use geo_types::{
    Coord, Geometry, GeometryCollection, LineString, MultiLineString, MultiPoint, MultiPolygon,
    Point, Polygon,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{Result, ToolkitError};
use crate::expr;
use crate::index::GridIndex;
use crate::layer::{CrsStatus, Feature, Layer};
use crate::proj::{self, AxisUnit, CrsInfo, Projection};
use crate::table::with_nulls;

/// Independent verification result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Check {
    /// Stable check identifier.
    pub id: String,
    /// Whether the check passed.
    pub passed: bool,
    /// Measured evidence or explanation.
    pub detail: String,
}

impl Check {
    fn new(id: impl Into<String>, passed: bool, detail: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            passed,
            detail: detail.into(),
        }
    }
}

/// Output of one operation.
#[derive(Debug, Clone)]
pub struct OpOutput {
    /// Resulting layer.
    pub layer: Layer,
    /// Independent verification checks.
    pub checks: Vec<Check>,
    /// Method notes (working CRS, measurement method, …).
    pub notes: Vec<String>,
}

/// Parameter description for planners and UIs.
#[derive(Debug, Clone, Serialize)]
pub struct ParamSpec {
    /// Parameter name.
    pub name: &'static str,
    /// Human description (with accepted values / units).
    pub description: &'static str,
    /// Whether it must be supplied.
    pub required: bool,
}

/// Operation description for planners and UIs.
#[derive(Debug, Clone, Serialize)]
pub struct OperationSpec {
    /// Operation name used in plans (`buffer`, `spatial_join`, …).
    pub name: &'static str,
    /// Japanese display title.
    pub title: &'static str,
    /// Description for AI planners.
    pub description: &'static str,
    /// Input roles in order; each takes a layer or step reference.
    pub inputs: &'static [&'static str],
    /// Parameters.
    pub params: Vec<ParamSpec>,
}

fn p(name: &'static str, required: bool, description: &'static str) -> ParamSpec {
    ParamSpec {
        name,
        description,
        required,
    }
}

/// The operation catalog.
pub fn catalog() -> Vec<OperationSpec> {
    vec![
        OperationSpec {
            name: "make_points",
            title: "地点を作成",
            description: "Create a point layer from explicit coordinates (e.g. a clicked or geocoded location). No inputs.",
            inputs: &[],
            params: vec![
                p("points", true, "array of {lon, lat, name?} objects or [lon, lat] pairs"),
                p("crs", false, "CRS of the coordinates, default EPSG:4326"),
                p("name", false, "layer name"),
            ],
        },
        OperationSpec {
            name: "buffer",
            title: "バッファ",
            description: "Area within a distance of each feature. Distance needs a unit (m or km). Computed in a metric CRS.",
            inputs: &["layer"],
            params: vec![
                p("distance", true, "e.g. \"500 m\", \"1.5 km\" or {\"value\":500,\"unit\":\"m\"}"),
                p("dissolve", false, "true to merge all buffers into one polygon"),
            ],
        },
        OperationSpec {
            name: "clip",
            title: "切り抜き",
            description: "Keep the parts of `layer` inside the polygons of `mask`.",
            inputs: &["layer", "mask"],
            params: vec![],
        },
        OperationSpec {
            name: "erase",
            title: "消去（差分）",
            description: "Remove the parts of `layer` inside the polygons of `mask`.",
            inputs: &["layer", "mask"],
            params: vec![],
        },
        OperationSpec {
            name: "intersect",
            title: "インターセクト（重ね合わせ）",
            description: "Split `layer` by the polygons of `overlay`; each piece keeps attributes of both.",
            inputs: &["layer", "overlay"],
            params: vec![],
        },
        OperationSpec {
            name: "dissolve",
            title: "ディゾルブ（結合）",
            description: "Merge features, optionally grouped by a field, with optional aggregates.",
            inputs: &["layer"],
            params: vec![
                p("by", false, "field to group by"),
                p("aggregates", false, "list of {op: count|sum|mean|min|max, field?, as?}"),
            ],
        },
        OperationSpec {
            name: "spatial_join",
            title: "空間結合（集計）",
            description: "For each `target` feature, aggregate the `join` features that satisfy a spatial predicate. Use area_weighted_sum to apportion polygon totals (e.g. population) by overlap area.",
            inputs: &["target", "join"],
            params: vec![
                p("predicate", false, "intersects (default) | contains | within | within_distance"),
                p("distance", false, "required for within_distance, with unit"),
                p("aggregates", false, "list of {op: count|sum|mean|min|max|area_weighted_sum, field?, as?, unit?}; default [{op: count}]"),
            ],
        },
        OperationSpec {
            name: "select_by_location",
            title: "位置で選択",
            description: "Keep `layer` features that satisfy a spatial predicate against any `other` feature.",
            inputs: &["layer", "other"],
            params: vec![
                p("predicate", false, "intersects (default) | contains | within | within_distance | disjoint"),
                p("distance", false, "required for within_distance, with unit"),
                p("invert", false, "true keeps the features that do NOT satisfy the predicate"),
            ],
        },
        OperationSpec {
            name: "distance_to_nearest",
            title: "最近傍距離",
            description: "Add the distance in metres from each `layer` feature to the nearest `target` feature.",
            inputs: &["layer", "target"],
            params: vec![
                p("as", false, "output field name, default nearest_distance_m"),
                p("copy_field", false, "target field to copy from the nearest feature"),
            ],
        },
        OperationSpec {
            name: "reproject",
            title: "座標変換",
            description: "Transform to another CRS (EPSG code).",
            inputs: &["layer"],
            params: vec![p("crs", true, "target CRS, e.g. EPSG:6675")],
        },
        OperationSpec {
            name: "assign_crs",
            title: "CRSを指定",
            description: "Declare the CRS of coordinates whose CRS was missing or wrongly inferred. Does not transform coordinates; use reproject for that.",
            inputs: &["layer"],
            params: vec![p("crs", true, "the CRS the coordinates are actually in, e.g. EPSG:6675")],
        },
        OperationSpec {
            name: "make_valid",
            title: "形状を修復",
            description: "Repair invalid polygons (self-intersections, crossing rings). Polygon operations reject invalid input, so add this step when a layer lists invalid_feature_ids.",
            inputs: &["layer"],
            params: vec![],
        },
        OperationSpec {
            name: "centroid",
            title: "重心",
            description: "Replace each geometry by its centroid (or a guaranteed interior point with inside=true).",
            inputs: &["layer"],
            params: vec![p("inside", false, "true to use an interior point")],
        },
        OperationSpec {
            name: "measure",
            title: "計測（面積・長さ）",
            description: "Add geodesic area / length / perimeter fields with explicit units.",
            inputs: &["layer"],
            params: vec![
                p("metrics", false, "subset of [area, length, perimeter]; default by geometry type"),
                p("area_unit", false, "m2 | ha | km2 (default km2)"),
                p("length_unit", false, "m | km (default km)"),
            ],
        },
        OperationSpec {
            name: "filter",
            title: "属性で絞り込み",
            description: "Keep features whose attributes satisfy an expression, e.g. 人口 >= 1000 AND 区名 LIKE '%中%'.",
            inputs: &["layer"],
            params: vec![p("where", true, "filter expression")],
        },
        OperationSpec {
            name: "calculate",
            title: "フィールド計算",
            description: "Add or overwrite a field from an arithmetic expression, e.g. population / area_km2.",
            inputs: &["layer"],
            params: vec![
                p("field", true, "output field name"),
                p("expression", true, "arithmetic expression over fields"),
                p("unit", false, "unit of the result, e.g. persons/km²"),
            ],
        },
        OperationSpec {
            name: "summarize",
            title: "集計",
            description: "Aggregate attributes into a table (optionally grouped).",
            inputs: &["layer"],
            params: vec![
                p("by", false, "field to group by"),
                p("aggregates", true, "list of {op: count|sum|mean|min|max, field?, as?}"),
            ],
        },
    ]
}

/// Run one operation.
pub fn run(op: &str, inputs: &BTreeMap<String, Layer>, params: &Value) -> Result<OpOutput> {
    let spec = catalog()
        .into_iter()
        .find(|s| s.name == op)
        .ok_or_else(|| ToolkitError::Plan(format!("unknown operation {op}")))?;
    for role in spec.inputs {
        if !inputs.contains_key(*role) {
            return Err(ToolkitError::parameter(op, format!("missing input {role}")));
        }
    }
    for role in inputs.keys() {
        if !spec.inputs.contains(&role.as_str()) {
            return Err(ToolkitError::parameter(
                op,
                format!("unexpected input {role}"),
            ));
        }
    }
    let params = match params {
        Value::Null => json!({}),
        Value::Object(_) => params.clone(),
        _ => return Err(ToolkitError::parameter(op, "params must be an object")),
    };
    if let Value::Object(map) = &params {
        for key in map.keys() {
            if !spec.params.iter().any(|p| p.name == key) {
                return Err(ToolkitError::parameter(
                    op,
                    format!("unknown parameter {key}"),
                ));
            }
        }
    }
    for param in spec.params.iter().filter(|p| p.required) {
        if params.get(param.name).is_none_or(Value::is_null) {
            return Err(ToolkitError::parameter(
                op,
                format!("missing parameter {}", param.name),
            ));
        }
    }
    let input = |role: &str| &inputs[role];
    let mut output = match op {
        "make_points" => make_points(&params),
        "buffer" => buffer(input("layer"), &params),
        "clip" => clip(input("layer"), input("mask"), false),
        "erase" => clip(input("layer"), input("mask"), true),
        "intersect" => intersect(input("layer"), input("overlay")),
        "dissolve" => dissolve(input("layer"), &params),
        "spatial_join" => spatial_join(input("target"), input("join"), &params),
        "select_by_location" => select_by_location(input("layer"), input("other"), &params),
        "distance_to_nearest" => distance_to_nearest(input("layer"), input("target"), &params),
        "reproject" => reproject(input("layer"), &params),
        "assign_crs" => assign_crs(input("layer"), &params),
        "make_valid" => make_valid(input("layer")),
        "centroid" => centroid(input("layer"), &params),
        "measure" => measure(input("layer"), &params),
        "filter" => filter(input("layer"), &params),
        "calculate" => calculate(input("layer"), &params),
        "summarize" => summarize(input("layer"), &params),
        _ => unreachable!("catalog and dispatch agree"),
    }?;
    output.layer.provenance.format = "derived".into();
    output.layer.provenance.operation = Some(op.to_string());
    output.layer.provenance.parents = inputs.values().map(Layer::digest).collect();
    output.layer.provenance.source_uri = format!("workflow://toolkit/{op}");
    let mut attribution: Vec<String> = inputs
        .values()
        .filter_map(|l| l.provenance.attribution.clone())
        .collect();
    attribution.sort();
    attribution.dedup();
    if !attribution.is_empty() {
        output.layer.provenance.attribution = Some(attribution.join(" / "));
    }
    let mut licenses: Vec<String> = inputs
        .values()
        .filter_map(|l| l.provenance.license.clone())
        .collect();
    licenses.sort();
    licenses.dedup();
    if !licenses.is_empty() {
        output.layer.provenance.license = Some(licenses.join(" + "));
    }
    output.layer.provenance.notes = output.notes.clone();
    output.layer.refresh_schema();
    Ok(output)
}

// ---------------------------------------------------------------------------
// Parameters and units
// ---------------------------------------------------------------------------

/// Parse a length with an explicit unit into metres.
pub fn parse_length(value: &Value) -> std::result::Result<f64, String> {
    let (number, unit) = match value {
        Value::Object(map) => (
            map.get("value")
                .and_then(Value::as_f64)
                .ok_or("distance.value must be a number")?,
            map.get("unit")
                .and_then(Value::as_str)
                .ok_or("distance.unit is required (m or km)")?
                .to_string(),
        ),
        Value::String(text) => {
            let text = text.trim();
            let split = text
                .char_indices()
                .find(|(_, c)| !(c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '+'))
                .map(|(i, _)| i)
                .unwrap_or(text.len());
            let number: f64 = text[..split]
                .trim()
                .parse()
                .map_err(|_| format!("cannot read a number from {text:?}"))?;
            (number, text[split..].trim().to_string())
        }
        Value::Number(_) => return Err("distance needs a unit, e.g. \"500 m\" or \"1 km\"".into()),
        _ => return Err("distance must be a string like \"500 m\" or {value, unit}".into()),
    };
    let factor = match unit.to_lowercase().as_str() {
        "m" | "meter" | "meters" | "metre" | "metres" | "メートル" | "ｍ" => 1.0,
        "km" | "kilometer" | "kilometers" | "kilometre" | "kilometres" | "キロ"
        | "キロメートル" | "ｋｍ" => 1000.0,
        "deg" | "degree" | "degrees" | "度" => {
            return Err("distances in degrees are not metric and are rejected; use m or km".into())
        }
        "" => return Err("distance needs a unit, e.g. \"500 m\" or \"1 km\"".into()),
        other => return Err(format!("unsupported distance unit {other}; use m or km")),
    };
    if !number.is_finite() {
        return Err("distance must be finite".into());
    }
    Ok(number * factor)
}

fn length_param(op: &str, params: &Value, key: &str) -> Result<f64> {
    let value = params
        .get(key)
        .ok_or_else(|| ToolkitError::parameter(op, format!("missing parameter {key}")))?;
    parse_length(value).map_err(|reason| ToolkitError::Unit(format!("{op}.{key}: {reason}")))
}

fn str_param<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
}

fn bool_param(op: &str, params: &Value, key: &str) -> Result<bool> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(ToolkitError::parameter(
            op,
            format!("{key} must be true or false"),
        )),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AggOp {
    Count,
    Sum,
    Mean,
    Min,
    Max,
    AreaWeightedSum,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Aggregate {
    op: AggOp,
    #[serde(default)]
    field: Option<String>,
    #[serde(default, rename = "as")]
    alias: Option<String>,
    /// Unit of the result when the source field declares none.
    #[serde(default)]
    unit: Option<String>,
}

impl Aggregate {
    fn output_name(&self) -> String {
        if let Some(alias) = &self.alias {
            return alias.clone();
        }
        let op = match self.op {
            AggOp::Count => return "count".into(),
            AggOp::Sum => "sum",
            AggOp::Mean => "mean",
            AggOp::Min => "min",
            AggOp::Max => "max",
            AggOp::AreaWeightedSum => "aw_sum",
        };
        format!("{op}_{}", self.field.as_deref().unwrap_or(""))
    }
}

fn aggregates_param(
    op: &str,
    params: &Value,
    source: &Layer,
    allow_area_weighted: bool,
) -> Result<Vec<Aggregate>> {
    let aggregates: Vec<Aggregate> = match params.get("aggregates") {
        None | Some(Value::Null) => vec![Aggregate {
            op: AggOp::Count,
            field: None,
            alias: None,
            unit: None,
        }],
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|e| ToolkitError::parameter(op, format!("aggregates: {e}")))?,
    };
    for aggregate in &aggregates {
        if aggregate.op == AggOp::AreaWeightedSum && !allow_area_weighted {
            return Err(ToolkitError::parameter(
                op,
                "area_weighted_sum is only valid in spatial_join",
            ));
        }
        if aggregate.op != AggOp::Count {
            let field = aggregate.field.as_deref().ok_or_else(|| {
                ToolkitError::parameter(op, format!("{:?} needs a field", aggregate.op))
            })?;
            let schema = source
                .field(field)
                .ok_or_else(|| ToolkitError::parameter(op, format!("unknown field {field}")))?;
            if !matches!(
                schema.field_type,
                crate::FieldType::Integer | crate::FieldType::Float
            ) {
                return Err(ToolkitError::parameter(
                    op,
                    format!("field {field} is not numeric"),
                ));
            }
        }
    }
    Ok(aggregates)
}

fn aggregate_unit(aggregate: &Aggregate, source: &Layer) -> Option<String> {
    if let Some(unit) = &aggregate.unit {
        return Some(unit.clone());
    }
    match aggregate.op {
        AggOp::Count => Some("features".into()),
        _ => aggregate
            .field
            .as_deref()
            .and_then(|f| source.field(f))
            .and_then(|f| f.unit.clone()),
    }
}

fn number_value(v: f64) -> Value {
    serde_json::Number::from_f64(v)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

/// Aggregate a set of (feature, weight) pairs.
fn aggregate_values(aggregate: &Aggregate, members: &[(&Feature, f64)]) -> Value {
    if aggregate.op == AggOp::Count {
        return Value::from(members.len() as i64);
    }
    let field = aggregate.field.as_deref().unwrap_or("");
    let values: Vec<(f64, f64)> = members
        .iter()
        .filter_map(|(f, w)| {
            f.properties
                .get(field)
                .and_then(Value::as_f64)
                .map(|v| (v, *w))
        })
        .collect();
    if values.is_empty() {
        return if matches!(aggregate.op, AggOp::Sum | AggOp::AreaWeightedSum) {
            Value::from(0)
        } else {
            Value::Null
        };
    }
    match aggregate.op {
        AggOp::Sum => number_value(values.iter().map(|(v, _)| v).sum()),
        AggOp::AreaWeightedSum => number_value(values.iter().map(|(v, w)| v * w).sum()),
        AggOp::Mean => {
            number_value(values.iter().map(|(v, _)| v).sum::<f64>() / values.len() as f64)
        }
        AggOp::Min => number_value(values.iter().map(|(v, _)| *v).fold(f64::INFINITY, f64::min)),
        AggOp::Max => number_value(
            values
                .iter()
                .map(|(v, _)| *v)
                .fold(f64::NEG_INFINITY, f64::max),
        ),
        AggOp::Count => unreachable!(),
    }
}

// ---------------------------------------------------------------------------
// Geometry helpers
// ---------------------------------------------------------------------------

/// Polygonal part of a geometry in OGC orientation (exterior CCW, holes CW).
/// The overlay engine behind buffer, union, and clipping reads ring
/// direction, so clockwise source data (common in N03/A31 exports) must be
/// normalised before any polygon operation.
fn polygonal(geometry: &Geometry<f64>) -> Option<MultiPolygon<f64>> {
    use geo::orient::{Direction, Orient};
    polygonal_raw(geometry).map(|m| m.orient(Direction::Default))
}

fn polygonal_raw(geometry: &Geometry<f64>) -> Option<MultiPolygon<f64>> {
    match geometry {
        Geometry::Polygon(p) => Some(MultiPolygon(vec![p.clone()])),
        Geometry::MultiPolygon(m) => Some(m.clone()),
        Geometry::Rect(r) => Some(MultiPolygon(vec![r.to_polygon()])),
        Geometry::Triangle(t) => Some(MultiPolygon(vec![t.to_polygon()])),
        Geometry::GeometryCollection(c) => {
            let polys: Vec<Polygon<f64>> = c
                .iter()
                .filter_map(polygonal_raw)
                .flat_map(|m| m.0)
                .collect();
            (!polys.is_empty()).then_some(MultiPolygon(polys))
        }
        _ => None,
    }
}

fn lineal(geometry: &Geometry<f64>) -> Option<MultiLineString<f64>> {
    match geometry {
        Geometry::Line(l) => Some(MultiLineString(vec![LineString(vec![l.start, l.end])])),
        Geometry::LineString(l) => Some(MultiLineString(vec![l.clone()])),
        Geometry::MultiLineString(m) => Some(m.clone()),
        _ => None,
    }
}

fn puntal(geometry: &Geometry<f64>) -> Option<MultiPoint<f64>> {
    match geometry {
        Geometry::Point(p) => Some(MultiPoint(vec![*p])),
        Geometry::MultiPoint(m) => Some(m.clone()),
        _ => None,
    }
}

fn from_polygons(mp: MultiPolygon<f64>) -> Option<Geometry<f64>> {
    let mut polys: Vec<Polygon<f64>> =
        mp.0.into_iter()
            .filter(|p| p.unsigned_area() > 0.0)
            .collect();
    match polys.len() {
        0 => None,
        1 => Some(Geometry::Polygon(polys.remove(0))),
        _ => Some(Geometry::MultiPolygon(MultiPolygon(polys))),
    }
}

fn from_lines(ml: MultiLineString<f64>) -> Option<Geometry<f64>> {
    let mut lines: Vec<LineString<f64>> = ml.0.into_iter().filter(|l| l.0.len() >= 2).collect();
    match lines.len() {
        0 => None,
        1 => Some(Geometry::LineString(lines.remove(0))),
        _ => Some(Geometry::MultiLineString(MultiLineString(lines))),
    }
}

fn from_points(mp: MultiPoint<f64>) -> Option<Geometry<f64>> {
    match mp.0.len() {
        0 => None,
        1 => Some(Geometry::Point(mp.0[0])),
        _ => Some(Geometry::MultiPoint(mp)),
    }
}

/// Intersection (`invert = false`) or difference (`invert = true`) of any
/// geometry with a polygonal mask.
fn clip_geometry(
    geometry: &Geometry<f64>,
    mask: &MultiPolygon<f64>,
    invert: bool,
) -> Option<Geometry<f64>> {
    if let Some(poly) = polygonal(geometry) {
        return from_polygons(if invert {
            poly.difference(mask)
        } else {
            poly.intersection(mask)
        });
    }
    if let Some(lines) = lineal(geometry) {
        return from_lines(mask.clip(&lines, invert));
    }
    if let Some(points) = puntal(geometry) {
        return from_points(MultiPoint(
            points
                .0
                .into_iter()
                .filter(|p| mask.intersects(p) != invert)
                .collect(),
        ));
    }
    if let Geometry::GeometryCollection(c) = geometry {
        let parts: Vec<Geometry<f64>> = c
            .iter()
            .filter_map(|g| clip_geometry(g, mask, invert))
            .collect();
        return (!parts.is_empty())
            .then_some(Geometry::GeometryCollection(GeometryCollection(parts)));
    }
    None
}

fn union_all(polys: &[MultiPolygon<f64>]) -> MultiPolygon<f64> {
    if polys.is_empty() {
        return MultiPolygon(vec![]);
    }
    geo::unary_union(polys.iter())
}

fn bbox_of(geometry: &Geometry<f64>) -> Option<[f64; 4]> {
    geometry
        .bounding_rect()
        .map(|r| [r.min().x, r.min().y, r.max().x, r.max().y])
}

fn bbox_overlap(a: &[f64; 4], b: &[f64; 4], pad: f64) -> bool {
    a[0] <= b[2] + pad && b[0] <= a[2] + pad && a[1] <= b[3] + pad && b[1] <= a[3] + pad
}

fn representative_point(geometry: &Geometry<f64>) -> Option<Point<f64>> {
    match geometry {
        Geometry::Point(p) => Some(*p),
        _ => geometry.interior_point(),
    }
}

/// Metric working CRS for a layer: the layer CRS if it is a projected metric
/// CRS (Web Mercator excluded — its scale error grows with latitude),
/// otherwise the UTM zone at the layer's centre.
fn working_crs(layer: &Layer) -> Result<CrsInfo> {
    let crs = layer.crs_info()?;
    if crs.unit == AxisUnit::Metres && !matches!(crs.projection, Projection::WebMercator) {
        return Ok(crs);
    }
    let bbox = layer.bbox_wgs84().ok_or_else(|| {
        ToolkitError::parameter(
            "working CRS",
            format!("layer {} has no geometry", layer.name),
        )
    })?;
    Ok(proj::metric_crs_for(
        (bbox[0] + bbox[2]) / 2.0,
        (bbox[1] + bbox[3]) / 2.0,
    ))
}

fn derived(name: String, crs: &str) -> Layer {
    Layer::new(name, crs, CrsStatus::Derived)
}

fn working_note(crs: &CrsInfo) -> String {
    format!("metric work performed in {} ({})", crs.id, crs.name)
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

fn make_points(params: &Value) -> Result<OpOutput> {
    let crs = proj::lookup(str_param(params, "crs").unwrap_or("EPSG:4326"))?;
    let points = params
        .get("points")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
        .ok_or_else(|| {
            ToolkitError::parameter("make_points", "points must be a non-empty array")
        })?;
    let mut layer = derived(
        str_param(params, "name").unwrap_or("地点").to_string(),
        &crs.id,
    );
    layer.crs_status = CrsStatus::UserSupplied;
    for (i, point) in points.iter().enumerate() {
        let (x, y, mut properties) = match point {
            Value::Array(pair) if pair.len() >= 2 => {
                (pair[0].as_f64(), pair[1].as_f64(), BTreeMap::new())
            }
            Value::Object(map) => {
                let x = map
                    .get("lon")
                    .or_else(|| map.get("x"))
                    .and_then(Value::as_f64);
                let y = map
                    .get("lat")
                    .or_else(|| map.get("y"))
                    .and_then(Value::as_f64);
                let mut props: BTreeMap<String, Value> = map
                    .get("properties")
                    .and_then(Value::as_object)
                    .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                    .unwrap_or_default();
                if let Some(name) = map.get("name").or_else(|| map.get("label")) {
                    props.insert("name".into(), name.clone());
                }
                (x, y, props)
            }
            _ => (None, None, BTreeMap::new()),
        };
        let (Some(x), Some(y)) = (x, y) else {
            return Err(ToolkitError::parameter(
                "make_points",
                format!("point {i} needs numeric coordinates"),
            ));
        };
        if crs.is_geographic() {
            proj::from_geographic(&crs, x, y)?;
        } else if !x.is_finite() || !y.is_finite() {
            return Err(ToolkitError::InvalidCoordinate(format!("point {i}")));
        }
        properties
            .entry("name".into())
            .or_insert_with(|| Value::from(format!("地点{}", i + 1)));
        layer.features.push(Feature {
            id: i as u64,
            geometry: Some(Geometry::Point(Point::new(x, y))),
            properties,
        });
    }
    Ok(OpOutput {
        layer,
        checks: vec![Check::new(
            "coordinates_in_domain",
            true,
            format!("{} points validated in {}", points.len(), crs.id),
        )],
        notes: vec!["points supplied explicitly by the user or planner".into()],
    })
}

/// Polygon operations on invalid rings silently produce empty or wrong
/// results, so they fail closed with an actionable message instead.
fn require_valid(op: &str, layer: &Layer) -> Result<()> {
    let invalid = layer.invalid_feature_ids();
    if invalid.is_empty() {
        return Ok(());
    }
    Err(ToolkitError::parameter(
        op,
        format!(
            "layer {} has {} invalid polygons (ids {:?}); add a make_valid step before {op}",
            layer.id(),
            invalid.len(),
            &invalid[..invalid.len().min(10)]
        ),
    ))
}

fn make_valid(layer: &Layer) -> Result<OpOutput> {
    use geo::{MakeValid, Validation};
    let mut out = layer.clone();
    out.name = format!("{}_valid", layer.name);
    out.crs_status = CrsStatus::Derived;
    out.provenance = Default::default();
    let mut repaired = 0usize;
    let mut worst_change = 0.0f64;
    for feature in &mut out.features {
        let Some(geometry) = &feature.geometry else {
            continue;
        };
        if !matches!(geometry, Geometry::Polygon(_) | Geometry::MultiPolygon(_))
            || geometry.is_valid()
        {
            continue;
        }
        let before = polygonal_raw(geometry)
            .map(|p| p.unsigned_area())
            .unwrap_or(0.0);
        let fixed = match geometry {
            Geometry::Polygon(p) => p.make_valid(),
            Geometry::MultiPolygon(m) => m.make_valid(),
            _ => unreachable!(),
        }
        .map_err(|e| {
            ToolkitError::parameter("make_valid", format!("feature {}: {e:?}", feature.id))
        })?;
        if before > 0.0 {
            worst_change = worst_change.max((fixed.unsigned_area() - before).abs() / before);
        }
        feature.geometry = from_polygons(fixed);
        repaired += 1;
    }
    let still_invalid = out.invalid_feature_ids();
    Ok(OpOutput {
        checks: vec![Check::new(
            "all_polygons_valid",
            still_invalid.is_empty(),
            format!("{repaired} repaired, {} still invalid", still_invalid.len()),
        )],
        notes: vec![format!(
            "repaired {repaired} invalid polygons; largest planar area change {:.2}% (self-overlaps are counted once)",
            worst_change * 100.0
        )],
        layer: out,
    })
}

fn buffer(layer: &Layer, params: &Value) -> Result<OpOutput> {
    require_valid("buffer", layer)?;
    let distance = length_param("buffer", params, "distance")?;
    let dissolve_all = bool_param("buffer", params, "dissolve")?;
    if distance <= 0.0 && layer.geometry_kind() != crate::GeometryKind::Polygon {
        return Err(ToolkitError::parameter(
            "buffer",
            "a zero or negative distance is only valid for polygons",
        ));
    }
    let source_crs = layer.crs_info()?;
    let work = working_crs(layer)?;
    let projected = layer.reprojected(&work)?;
    let mut buffers: Vec<(Feature, MultiPolygon<f64>)> = Vec::new();
    for feature in &projected.features {
        let Some(geometry) = &feature.geometry else {
            continue;
        };
        buffers.push((
            feature.clone(),
            orient_ogc(geometry.clone()).buffer(distance),
        ));
    }

    // Independent checks in the working CRS.
    let mut contain_failures = 0;
    let mut area_errors: Vec<f64> = Vec::new();
    for (feature, buffered) in &buffers {
        let geometry = feature.geometry.as_ref().expect("filtered");
        if distance > 0.0 {
            if let Some(point) = representative_point(geometry) {
                if !buffered.intersects(&point) {
                    contain_failures += 1;
                }
            }
        }
        if let Geometry::Point(_) = geometry {
            let expected = std::f64::consts::PI * distance * distance;
            area_errors.push((buffered.unsigned_area() - expected).abs() / expected);
        }
    }
    let mut checks = vec![Check::new(
        "buffer_contains_source",
        contain_failures == 0,
        format!(
            "{} of {} buffers contain their source feature",
            buffers.len() - contain_failures,
            buffers.len()
        ),
    )];
    if !area_errors.is_empty() {
        let worst = area_errors.iter().cloned().fold(0.0, f64::max);
        checks.push(Check::new(
            "point_buffer_area_matches_circle",
            worst <= 0.02,
            format!("max relative area error vs πr² = {:.4}%", worst * 100.0),
        ));
    }

    let mut out = derived(format!("{}_buffer", layer.name), &layer.crs);
    if dissolve_all {
        let merged = union_all(&buffers.iter().map(|(_, b)| b.clone()).collect::<Vec<_>>());
        if let Some(geometry) = from_polygons(merged) {
            out.features.push(Feature {
                id: 0,
                geometry: Some(proj::transform_geometry(&work, &source_crs, &geometry)?),
                properties: BTreeMap::from([
                    (
                        "source_count".to_string(),
                        Value::from(buffers.len() as i64),
                    ),
                    ("buffer_m".to_string(), number_value(distance)),
                ]),
            });
        }
    } else {
        for (feature, buffered) in buffers {
            let Some(geometry) = from_polygons(buffered) else {
                continue;
            };
            let mut properties = feature.properties.clone();
            properties.insert("buffer_m".into(), number_value(distance));
            out.features.push(Feature {
                id: feature.id,
                geometry: Some(proj::transform_geometry(&work, &source_crs, &geometry)?),
                properties,
            });
        }
    }
    out.refresh_schema();
    out.set_field_unit("buffer_m", "m");
    Ok(OpOutput {
        layer: out,
        checks,
        notes: vec![
            working_note(&work),
            format!("buffer distance {distance} m with rounded joins and caps"),
        ],
    })
}

fn clip(layer: &Layer, mask: &Layer, invert: bool) -> Result<OpOutput> {
    require_valid(if invert { "erase" } else { "clip" }, layer)?;
    require_valid(if invert { "erase" } else { "clip" }, mask)?;
    let op = if invert { "erase" } else { "clip" };
    let crs = layer.crs_info()?;
    let mask_layer = mask.reprojected(&crs)?;
    let mask_polys: Vec<MultiPolygon<f64>> = mask_layer
        .features
        .iter()
        .filter_map(|f| f.geometry.as_ref().and_then(polygonal))
        .collect();
    if mask_polys.is_empty() {
        return Err(ToolkitError::parameter(op, "mask has no polygons"));
    }
    let mask_union = union_all(&mask_polys);
    let mask_bbox = bbox_of(&Geometry::MultiPolygon(mask_union.clone()));
    let mut out = derived(format!("{}_{op}", layer.name), &layer.crs);
    out.fields = layer.fields.clone();
    for feature in &layer.features {
        let Some(geometry) = &feature.geometry else {
            continue;
        };
        if !invert {
            if let (Some(a), Some(b)) = (bbox_of(geometry), mask_bbox) {
                if !bbox_overlap(&a, &b, 0.0) {
                    continue;
                }
            }
        }
        if let Some(clipped) = clip_geometry(geometry, &mask_union, invert) {
            out.features.push(Feature {
                id: feature.id,
                geometry: Some(clipped),
                properties: feature.properties.clone(),
            });
        }
    }
    let mut checks = vec![Check::new(
        "feature_count_not_increased",
        out.features.len() <= layer.features.len(),
        format!("{} → {} features", layer.features.len(), out.features.len()),
    )];
    if !invert {
        let tolerance = if crs.is_geographic() { 1e-9 } else { 1e-4 };
        let inside = match (out.bbox(), mask_bbox) {
            (Some(o), Some(m)) => {
                bbox_overlap(&o, &m, 0.0)
                    && o[0] >= m[0] - tolerance
                    && o[1] >= m[1] - tolerance
                    && o[2] <= m[2] + tolerance
                    && o[3] <= m[3] + tolerance
            }
            (None, _) => true,
            _ => false,
        };
        checks.push(Check::new(
            "output_within_mask_extent",
            inside,
            "clipped extent lies within the mask extent",
        ));
    } else {
        // Independent: nothing that remains may overlap the mask interior.
        let overlapping = out
            .features
            .iter()
            .filter_map(|f| f.geometry.as_ref().and_then(polygonal))
            .map(|p| p.intersection(&mask_union).unsigned_area())
            .fold(0.0, f64::max);
        let scale = if crs.is_geographic() { 1e-12 } else { 1e-3 };
        checks.push(Check::new(
            "erased_parts_do_not_overlap_mask",
            overlapping <= scale,
            format!("max remaining overlap area {overlapping:.3e} (CRS units²)"),
        ));
    }
    if mask.crs != layer.crs {
        checks.push(Check::new(
            "mask_reprojected",
            true,
            format!("mask transformed {} → {}", mask.crs, layer.crs),
        ));
    }
    Ok(OpOutput {
        layer: out,
        checks,
        notes: vec![],
    })
}

fn intersect(layer: &Layer, overlay: &Layer) -> Result<OpOutput> {
    require_valid("intersect", layer)?;
    require_valid("intersect", overlay)?;
    let crs = layer.crs_info()?;
    let overlay = overlay.reprojected(&crs)?;
    let overlay_polys: Vec<(&Feature, MultiPolygon<f64>, [f64; 4])> = overlay
        .features
        .iter()
        .filter_map(|f| {
            let g = f.geometry.as_ref()?;
            Some((f, polygonal(g)?, bbox_of(g)?))
        })
        .collect();
    if overlay_polys.is_empty() {
        return Err(ToolkitError::parameter(
            "intersect",
            "overlay has no polygons",
        ));
    }
    let mut out = derived(format!("{}_x_{}", layer.name, overlay.name), &layer.crs);
    out.fields = layer.fields.clone();
    let mut next_id = 0u64;
    let mut input_area = 0.0;
    for feature in &layer.features {
        let Some(geometry) = &feature.geometry else {
            continue;
        };
        let Some(bbox) = bbox_of(geometry) else {
            continue;
        };
        if let Some(p) = polygonal(geometry) {
            input_area += p.unsigned_area();
        }
        for (other, poly, other_bbox) in &overlay_polys {
            if !bbox_overlap(&bbox, other_bbox, 0.0) {
                continue;
            }
            let Some(piece) = clip_geometry(geometry, poly, false) else {
                continue;
            };
            let mut properties = feature.properties.clone();
            for (key, value) in &other.properties {
                let key = if properties.contains_key(key) {
                    format!("{key}_2")
                } else {
                    key.clone()
                };
                properties.insert(key, value.clone());
            }
            out.features.push(Feature {
                id: next_id,
                geometry: Some(piece),
                properties,
            });
            next_id += 1;
        }
    }
    let output_area: f64 = out
        .features
        .iter()
        .filter_map(|f| f.geometry.as_ref().and_then(polygonal))
        .map(|p| p.unsigned_area())
        .sum();
    let overlay_union_area = union_all(
        &overlay_polys
            .iter()
            .map(|(_, p, _)| p.clone())
            .collect::<Vec<_>>(),
    )
    .unsigned_area();
    let overlapping_overlay = overlay_polys
        .iter()
        .map(|(_, p, _)| p.unsigned_area())
        .sum::<f64>()
        > overlay_union_area * (1.0 + 1e-9);
    let bound_ok = overlapping_overlay || output_area <= input_area * (1.0 + 1e-9) + 1e-12;
    Ok(OpOutput {
        layer: out,
        checks: vec![Check::new(
            "piece_area_bounded_by_input",
            bound_ok,
            if overlapping_overlay {
                "overlay polygons overlap; area bound not applicable".to_string()
            } else {
                format!("pieces {output_area:.6e} ≤ input {input_area:.6e} (CRS units²)")
            },
        )],
        notes: vec![],
    })
}

fn dissolve(layer: &Layer, params: &Value) -> Result<OpOutput> {
    require_valid("dissolve", layer)?;
    let by = str_param(params, "by");
    if let Some(by) = by {
        if layer.field(by).is_none() {
            return Err(ToolkitError::parameter(
                "dissolve",
                format!("unknown field {by}"),
            ));
        }
    }
    let aggregates = if params.get("aggregates").is_some() {
        aggregates_param("dissolve", params, layer, false)?
    } else {
        vec![Aggregate {
            op: AggOp::Count,
            field: None,
            alias: None,
            unit: None,
        }]
    };
    let mut groups: BTreeMap<String, (Value, Vec<&Feature>)> = BTreeMap::new();
    for feature in &layer.features {
        let key_value = by
            .and_then(|b| feature.properties.get(b))
            .cloned()
            .unwrap_or(Value::Null);
        groups
            .entry(crate::layer::canonical_json(&key_value))
            .or_insert_with(|| (key_value.clone(), Vec::new()))
            .1
            .push(feature);
    }
    let mut out = derived(format!("{}_dissolved", layer.name), &layer.crs);
    let mut input_area = 0.0;
    let mut largest = 0.0f64;
    for (index, (_, (key, members))) in groups.into_iter().enumerate() {
        let polys: Vec<MultiPolygon<f64>> = members
            .iter()
            .filter_map(|f| f.geometry.as_ref().and_then(polygonal))
            .collect();
        for p in &polys {
            let a = p.unsigned_area();
            input_area += a;
            largest = largest.max(a);
        }
        let geometry = if polys.len() == members.len() && !polys.is_empty() {
            from_polygons(union_all(&polys))
        } else {
            let lines: Vec<LineString<f64>> = members
                .iter()
                .filter_map(|f| f.geometry.as_ref().and_then(lineal))
                .flat_map(|m| m.0)
                .collect();
            let points: Vec<Point<f64>> = members
                .iter()
                .filter_map(|f| f.geometry.as_ref().and_then(puntal))
                .flat_map(|m| m.0)
                .collect();
            if !lines.is_empty() && points.is_empty() && polys.is_empty() {
                from_lines(MultiLineString(lines))
            } else if !points.is_empty() && lines.is_empty() && polys.is_empty() {
                from_points(MultiPoint(points))
            } else {
                let parts: Vec<Geometry<f64>> =
                    members.iter().filter_map(|f| f.geometry.clone()).collect();
                (!parts.is_empty())
                    .then_some(Geometry::GeometryCollection(GeometryCollection(parts)))
            }
        };
        let mut properties = BTreeMap::new();
        if let Some(by) = by {
            properties.insert(by.to_string(), key);
        }
        let weighted: Vec<(&Feature, f64)> = members.iter().map(|f| (*f, 1.0)).collect();
        for aggregate in &aggregates {
            properties.insert(
                aggregate.output_name(),
                aggregate_values(aggregate, &weighted),
            );
        }
        out.features.push(Feature {
            id: index as u64,
            geometry,
            properties,
        });
    }
    out.refresh_schema();
    for aggregate in &aggregates {
        if let Some(unit) = aggregate_unit(aggregate, layer) {
            out.set_field_unit(&aggregate.output_name(), unit);
        }
    }
    let output_area: f64 = out
        .features
        .iter()
        .filter_map(|f| f.geometry.as_ref().and_then(polygonal))
        .map(|p| p.unsigned_area())
        .sum();
    let tolerance = input_area * 1e-9 + 1e-12;
    let mut checks = vec![];
    if input_area > 0.0 {
        checks.push(Check::new(
            "dissolved_area_bounds",
            output_area <= input_area + tolerance && output_area + tolerance >= largest,
            format!("largest input {largest:.6e} ≤ dissolved {output_area:.6e} ≤ sum of inputs {input_area:.6e}"),
        ));
    }
    let counted: i64 = out
        .features
        .iter()
        .filter_map(|f| f.properties.get("count").and_then(Value::as_i64))
        .sum();
    if aggregates
        .iter()
        .any(|a| a.op == AggOp::Count && a.alias.is_none())
    {
        checks.push(Check::new(
            "members_conserved",
            counted as usize == layer.features.len(),
            format!(
                "{counted} members across groups vs {} input features",
                layer.features.len()
            ),
        ));
    }
    Ok(OpOutput {
        layer: out,
        checks,
        notes: vec![],
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Predicate {
    Intersects,
    Contains,
    Within,
    WithinDistance(f64),
    Disjoint,
}

fn predicate_param(op: &str, params: &Value, allow_disjoint: bool) -> Result<Predicate> {
    Ok(
        match str_param(params, "predicate").unwrap_or("intersects") {
            "intersects" => Predicate::Intersects,
            "contains" => Predicate::Contains,
            "within" => Predicate::Within,
            "within_distance" | "dwithin" => {
                Predicate::WithinDistance(length_param(op, params, "distance")?)
            }
            "disjoint" if allow_disjoint => Predicate::Disjoint,
            other => {
                return Err(ToolkitError::parameter(
                    op,
                    format!("unknown predicate {other}"),
                ))
            }
        },
    )
}

fn predicate_holds(predicate: Predicate, a: &Geometry<f64>, b: &Geometry<f64>) -> bool {
    match predicate {
        Predicate::Intersects => a.intersects(b),
        Predicate::Disjoint => !a.intersects(b),
        Predicate::Contains => a.relate(b).is_contains(),
        Predicate::Within => a.relate(b).is_within(),
        Predicate::WithinDistance(d) => a.intersects(b) || Euclidean.distance(a, b) <= d,
    }
}

/// Second implementation used only for verification: ray casting for
/// point/polygon pairs and the DE-9IM matrix for everything else.
fn predicate_independent(
    predicate: Predicate,
    a: &Geometry<f64>,
    b: &Geometry<f64>,
) -> Option<bool> {
    let point_in = |pt: &Point<f64>, poly: &MultiPolygon<f64>| {
        poly.0.iter().any(|p| {
            ray_cast(pt.0, &p.exterior().0) && p.interiors().iter().all(|h| !ray_cast(pt.0, &h.0))
        })
    };
    match (predicate, a, b) {
        (Predicate::Intersects | Predicate::Contains, _, Geometry::Point(pt)) => {
            polygonal(a).map(|poly| point_in(pt, &poly))
        }
        (Predicate::Intersects | Predicate::Within, Geometry::Point(pt), _) => {
            polygonal(b).map(|poly| point_in(pt, &poly))
        }
        (Predicate::Intersects, _, _) => Some(a.relate(b).is_intersects()),
        (Predicate::Disjoint, _, _) => Some(a.relate(b).is_disjoint()),
        _ => None,
    }
}

fn ray_cast(point: Coord<f64>, ring: &[Coord<f64>]) -> bool {
    let mut inside = false;
    let mut j = ring.len().saturating_sub(1);
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[j]);
        if (a.y > point.y) != (b.y > point.y)
            && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

struct Indexed<'a> {
    feature: &'a Feature,
    geometry: Geometry<f64>,
    bbox: [f64; 4],
}

fn index_layer(layer: &Layer) -> Vec<Indexed<'_>> {
    layer
        .features
        .iter()
        .filter_map(|feature| {
            let geometry = feature.geometry.clone()?;
            let bbox = bbox_of(&geometry)?;
            Some(Indexed {
                feature,
                geometry,
                bbox,
            })
        })
        .collect()
}

fn spatial_join(target: &Layer, join: &Layer, params: &Value) -> Result<OpOutput> {
    require_valid("spatial_join", target)?;
    require_valid("spatial_join", join)?;
    let predicate = predicate_param("spatial_join", params, false)?;
    let aggregates = aggregates_param("spatial_join", params, join, true)?;
    let area_weighted = aggregates.iter().any(|a| a.op == AggOp::AreaWeightedSum);
    let work = working_crs(target)?;
    let target_m = target.reprojected(&work)?;
    let join_m = join.reprojected(&work)?;
    let joins = index_layer(&join_m);
    let join_grid = GridIndex::build(joins.iter().map(|j| j.bbox).collect(), 4);
    if area_weighted && joins.iter().any(|j| polygonal(&j.geometry).is_none()) {
        return Err(ToolkitError::parameter(
            "spatial_join",
            "area_weighted_sum needs polygon join features",
        ));
    }
    let join_areas: Vec<f64> = joins
        .iter()
        .map(|j| {
            polygonal(&j.geometry)
                .map(|p| p.unsigned_area())
                .unwrap_or(0.0)
        })
        .collect();
    let pad = match predicate {
        Predicate::WithinDistance(d) => d,
        _ => 0.0,
    };

    let mut out = target.clone();
    out.name = format!("{}_join_{}", target.name, join.name);
    out.crs_status = CrsStatus::Derived;
    out.provenance = Default::default();
    let mut total_matches = 0usize;
    let mut independent_matches = 0usize;
    let mut independent_available = true;
    let mut allocated_totals = vec![0.0; aggregates.len()];
    let mut target_polys: Vec<MultiPolygon<f64>> = Vec::new();
    for (index, feature) in target_m.features.iter().enumerate() {
        let mut members: Vec<(&Feature, f64)> = Vec::new();
        if let Some(geometry) = &feature.geometry {
            let bbox = bbox_of(geometry).unwrap_or([0.0; 4]);
            let target_poly = polygonal(geometry);
            if let Some(p) = &target_poly {
                target_polys.push(p.clone());
            }
            for j in join_grid.query(bbox, pad) {
                let candidate = &joins[j];
                let hit = predicate_holds(predicate, geometry, &candidate.geometry);
                match predicate_independent(predicate, geometry, &candidate.geometry) {
                    Some(true) => independent_matches += 1,
                    Some(false) => {}
                    None => independent_available = false,
                }
                if !hit {
                    continue;
                }
                let weight = if area_weighted {
                    match (&target_poly, polygonal(&candidate.geometry)) {
                        (Some(t), Some(jp)) if join_areas[j] > 0.0 => {
                            t.intersection(&jp).unsigned_area() / join_areas[j]
                        }
                        _ => 0.0,
                    }
                } else {
                    1.0
                };
                members.push((candidate.feature, weight));
            }
        }
        total_matches += members.len();
        for (a, aggregate) in aggregates.iter().enumerate() {
            let value = aggregate_values(aggregate, &members);
            if let Some(v) = value.as_f64() {
                allocated_totals[a] += v;
            }
            out.features[index]
                .properties
                .insert(aggregate.output_name(), value);
        }
    }
    out.refresh_schema();
    for aggregate in &aggregates {
        if let Some(unit) = aggregate_unit(aggregate, join) {
            out.set_field_unit(&aggregate.output_name(), unit);
        }
    }

    let mut checks = Vec::new();
    if independent_available {
        checks.push(Check::new(
            "match_count_independent",
            independent_matches == total_matches,
            format!("{total_matches} matches; independent predicate implementation found {independent_matches}"),
        ));
    } else {
        checks.push(Check::new(
            "match_count_independent",
            true,
            "no second predicate implementation for this geometry combination; primary DE-9IM result used",
        ));
    }
    for (a, aggregate) in aggregates.iter().enumerate() {
        if aggregate.op != AggOp::AreaWeightedSum {
            continue;
        }
        let field = aggregate.field.as_deref().unwrap_or_default();
        let source_total: f64 = join
            .features
            .iter()
            .filter_map(|f| f.properties.get(field).and_then(Value::as_f64))
            .sum();
        let overlapping_targets = targets_overlap(&target_polys);
        let allocated = allocated_totals[a];
        let passed = overlapping_targets || allocated <= source_total * (1.0 + 1e-6) + 1e-9;
        checks.push(Check::new(
            format!("area_weighted_{field}_conserved"),
            passed,
            if overlapping_targets {
                format!("targets overlap, so totals may legitimately double count; allocated {allocated:.3} of source {source_total:.3}")
            } else {
                format!(
                    "allocated {allocated:.3} ≤ source total {source_total:.3} ({:.2}% of source falls inside targets)",
                    if source_total > 0.0 { allocated / source_total * 100.0 } else { 0.0 }
                )
            },
        ));
    }
    let mut notes = vec![working_note(&work)];
    if area_weighted {
        notes.push("area_weighted_sum assumes values are spread uniformly within each join polygon (areal interpolation)".into());
    }
    Ok(OpOutput {
        layer: out,
        checks,
        notes,
    })
}

fn targets_overlap(polys: &[MultiPolygon<f64>]) -> bool {
    if polys.len() < 2 {
        return false;
    }
    if polys.len() > 200 {
        return true; // too many to test pairwise; treat conservatively
    }
    for i in 0..polys.len() {
        for j in (i + 1)..polys.len() {
            let (a, b) = (&polys[i], &polys[j]);
            let (Some(ba), Some(bb)) = (a.bounding_rect(), b.bounding_rect()) else {
                continue;
            };
            if ba.max().x < bb.min().x
                || bb.max().x < ba.min().x
                || ba.max().y < bb.min().y
                || bb.max().y < ba.min().y
            {
                continue;
            }
            let overlap = a.intersection(b).unsigned_area();
            if overlap > 1e-6 * a.unsigned_area().min(b.unsigned_area()) {
                return true;
            }
        }
    }
    false
}

fn select_by_location(layer: &Layer, other: &Layer, params: &Value) -> Result<OpOutput> {
    require_valid("select_by_location", layer)?;
    require_valid("select_by_location", other)?;
    let predicate = predicate_param("select_by_location", params, true)?;
    let invert = bool_param("select_by_location", params, "invert")?;
    let work = working_crs(layer)?;
    let layer_m = layer.reprojected(&work)?;
    let other_m = other.reprojected(&work)?;
    let others = index_layer(&other_m);
    let other_grid = GridIndex::build(others.iter().map(|o| o.bbox).collect(), 4);
    let others_all_points = others
        .iter()
        .all(|o| matches!(o.geometry, Geometry::Point(_)));
    let pad = match predicate {
        Predicate::WithinDistance(d) => d,
        _ => 0.0,
    };
    let mut keep = Vec::new();
    let mut disagreements = 0usize;
    let mut independent_available = true;
    for (index, feature) in layer_m.features.iter().enumerate() {
        let Some(geometry) = &feature.geometry else {
            continue;
        };
        let Some(bbox) = bbox_of(geometry) else {
            continue;
        };
        // Features whose boxes do not overlap cannot intersect, so disjoint
        // only needs to rule out the overlapping candidates too.
        let candidates: Vec<&Indexed> = other_grid
            .query(bbox, pad)
            .into_iter()
            .map(|i| &others[i])
            .collect();
        let hit = if predicate == Predicate::Disjoint {
            candidates.iter().all(|o| !o.geometry.intersects(geometry))
        } else {
            candidates
                .iter()
                .any(|o| predicate_holds(predicate, geometry, &o.geometry))
        };
        // Independent verification.
        let independent: Option<bool> = match predicate {
            Predicate::WithinDistance(d) => match geometry {
                Geometry::Point(pt) if others_all_points => {
                    let lonlat = proj::to_geographic(&work, pt.x(), pt.y())?;
                    let mut best = f64::INFINITY;
                    let available = true;
                    // Geodesic re-measurement of every point within a
                    // slightly wider box; anything outside it is farther
                    // than the threshold in both metrics.
                    for i in other_grid.query(bbox, d * 1.01 + 1.0) {
                        if let Geometry::Point(q) = &others[i].geometry {
                            let q = proj::to_geographic(&work, q.x(), q.y())?;
                            best = best.min(
                                Geodesic
                                    .distance(Point::new(lonlat.0, lonlat.1), Point::new(q.0, q.1)),
                            );
                        }
                    }
                    // Geodesic vs projected distances differ by the UTM scale
                    // factor (< 0.1 %), so only compare away from the threshold.
                    (available && (best - d).abs() > d * 0.002 + 0.5).then_some(best <= d)
                }
                _ => None,
            },
            Predicate::Disjoint => Some(candidates.iter().all(|o| {
                predicate_independent(Predicate::Disjoint, geometry, &o.geometry).unwrap_or(true)
            })),
            _ => {
                let mut result = Some(false);
                for o in &candidates {
                    match predicate_independent(predicate, geometry, &o.geometry) {
                        Some(true) => {
                            result = Some(true);
                            break;
                        }
                        Some(false) => {}
                        None => {
                            result = None;
                            break;
                        }
                    }
                }
                result
            }
        };
        match independent {
            Some(value) if value != hit => disagreements += 1,
            Some(_) => {}
            None => independent_available = false,
        }
        if hit != invert {
            keep.push(index);
        }
    }
    let mut out = derived(format!("{}_selected", layer.name), &layer.crs);
    out.fields = layer.fields.clone();
    for index in &keep {
        out.features.push(layer.features[*index].clone());
    }
    Ok(OpOutput {
        layer: out,
        checks: vec![Check::new(
            "selection_independent",
            disagreements == 0,
            if independent_available {
                format!(
                    "{} of {} selected; independent method disagreed on {disagreements}",
                    keep.len(),
                    layer.features.len()
                )
            } else {
                format!(
                    "{} of {} selected; independent method covered part of the pairs and disagreed on {disagreements}",
                    keep.len(),
                    layer.features.len()
                )
            },
        )],
        notes: vec![working_note(&work)],
    })
}

fn distance_to_nearest(layer: &Layer, target: &Layer, params: &Value) -> Result<OpOutput> {
    require_valid("distance_to_nearest", layer)?;
    require_valid("distance_to_nearest", target)?;
    let field = str_param(params, "as")
        .unwrap_or("nearest_distance_m")
        .to_string();
    let copy_field = str_param(params, "copy_field");
    if let Some(copy) = copy_field {
        if target.field(copy).is_none() {
            return Err(ToolkitError::parameter(
                "distance_to_nearest",
                format!("unknown target field {copy}"),
            ));
        }
    }
    let work = working_crs(layer)?;
    let layer_m = layer.reprojected(&work)?;
    let target_m = target.reprojected(&work)?;
    let targets = index_layer(&target_m);
    let target_grid = GridIndex::build(targets.iter().map(|t| t.bbox).collect(), 2);
    if targets.is_empty() {
        return Err(ToolkitError::parameter(
            "distance_to_nearest",
            "target has no geometry",
        ));
    }
    let mut out = layer.clone();
    out.name = format!("{}_nearest_{}", layer.name, target.name);
    out.crs_status = CrsStatus::Derived;
    out.provenance = Default::default();
    let mut worst_relative = 0.0f64;
    let mut compared = 0usize;
    for (index, feature) in layer_m.features.iter().enumerate() {
        let Some(geometry) = &feature.geometry else {
            continue;
        };
        let (best_index, distance) = match geometry {
            // Points: ring search on the grid with exact distances.
            Geometry::Point(p) => target_grid
                .nearest((p.x(), p.y()), |i| {
                    Euclidean.distance(geometry, &targets[i].geometry)
                })
                .expect("targets is non-empty"),
            // Extended geometries: exact scan (the ring bound assumes a point).
            _ => targets
                .iter()
                .enumerate()
                .map(|(i, t)| (i, Euclidean.distance(geometry, &t.geometry)))
                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                .expect("targets is non-empty"),
        };
        let nearest = &targets[best_index];
        if let (Geometry::Point(a), Geometry::Point(b)) = (geometry, &nearest.geometry) {
            let a = proj::to_geographic(&work, a.x(), a.y())?;
            let b = proj::to_geographic(&work, b.x(), b.y())?;
            let geodesic = Geodesic.distance(Point::new(a.0, a.1), Point::new(b.0, b.1));
            if geodesic > 1.0 {
                worst_relative = worst_relative.max((geodesic - distance).abs() / geodesic);
                compared += 1;
            }
        }
        let properties = &mut out.features[index].properties;
        properties.insert(
            field.clone(),
            number_value((distance * 100.0).round() / 100.0),
        );
        if let Some(copy) = copy_field {
            properties.insert(
                format!("nearest_{copy}"),
                nearest
                    .feature
                    .properties
                    .get(copy)
                    .cloned()
                    .unwrap_or(Value::Null),
            );
        }
    }
    out.refresh_schema();
    out.set_field_unit(&field, "m");
    let checks = vec![Check::new(
        "distance_matches_geodesic",
        worst_relative <= 0.005,
        if compared > 0 {
            format!(
                "{compared} point pairs re-measured geodesically; max relative difference {:.4}%",
                worst_relative * 100.0
            )
        } else {
            "no point pairs to re-measure geodesically".to_string()
        },
    )];
    Ok(OpOutput {
        layer: out,
        checks,
        notes: vec![working_note(&work)],
    })
}

fn reproject(layer: &Layer, params: &Value) -> Result<OpOutput> {
    let target = proj::lookup(str_param(params, "crs").unwrap_or(""))?;
    let source = layer.crs_info()?;
    let mut out = layer.reprojected(&target)?;
    out.name = format!("{}_{}", layer.name, target.id.replace(':', ""));
    out.crs_status = CrsStatus::Derived;
    out.provenance = Default::default();
    // Independent round trip of up to 200 coordinates.
    use geo::CoordsIter;
    let mut worst = 0.0f64;
    for (a, b) in layer
        .features
        .iter()
        .zip(&out.features)
        .filter_map(|(a, b)| Some((a.geometry.as_ref()?, b.geometry.as_ref()?)))
        .flat_map(|(a, b)| a.coords_iter().zip(b.coords_iter()).collect::<Vec<_>>())
        .take(200)
    {
        let (x, y) = proj::transform_coord(&target, &source, b.x, b.y)?;
        worst = worst.max((x - a.x).abs().max((y - a.y).abs()));
    }
    let tolerance = if source.is_geographic() { 1e-7 } else { 1e-3 };
    let mut notes = vec![format!("{} → {}", source.id, target.id)];
    let datums = |c: &CrsInfo| match c.epsg {
        6668..=6692 => "JGD2011",
        2443..=2461 | 3097..=3101 | 4612 => "JGD2000",
        _ => "WGS84",
    };
    if datums(&source) != datums(&target) {
        notes.push(proj::DATUM_ASSUMPTION.into());
    }
    Ok(OpOutput {
        layer: out,
        checks: vec![Check::new(
            "round_trip",
            worst <= tolerance,
            format!("max round-trip error {worst:.2e} source units (tolerance {tolerance:.0e})"),
        )],
        notes,
    })
}

fn assign_crs(layer: &Layer, params: &Value) -> Result<OpOutput> {
    let target = proj::lookup(str_param(params, "crs").unwrap_or(""))?;
    let mut out = layer.clone();
    out.crs = target.id.clone();
    out.crs_status = CrsStatus::UserSupplied;
    out.provenance = Default::default();
    // Every coordinate must be valid in the assigned CRS.
    use geo::CoordsIter;
    let mut invalid = 0usize;
    for c in out
        .features
        .iter()
        .filter_map(|f| f.geometry.as_ref())
        .flat_map(|g| g.coords_iter())
    {
        if proj::to_geographic(&target, c.x, c.y)
            .and_then(|(lon, lat)| proj::from_geographic(&target, lon, lat))
            .is_err()
        {
            invalid += 1;
        }
    }
    Ok(OpOutput {
        checks: vec![Check::new(
            "coordinates_valid_in_assigned_crs",
            invalid == 0,
            format!("{invalid} coordinates outside the domain of {}", target.id),
        )],
        notes: vec![format!(
            "CRS assigned by the user: {} → {} (coordinates unchanged; previously {:?})",
            layer.crs, target.id, layer.crs_status
        )],
        layer: out,
    })
}

fn centroid(layer: &Layer, params: &Value) -> Result<OpOutput> {
    let inside = bool_param("centroid", params, "inside")?;
    let mut out = derived(format!("{}_centroid", layer.name), &layer.crs);
    out.fields = layer.fields.clone();
    let mut outside = 0;
    for feature in &layer.features {
        let Some(geometry) = &feature.geometry else {
            continue;
        };
        let point = if inside {
            geometry.interior_point()
        } else {
            geometry.centroid()
        };
        let Some(point) = point else { continue };
        if inside && polygonal(geometry).is_some_and(|p| !p.intersects(&point)) {
            outside += 1;
        }
        out.features.push(Feature {
            id: feature.id,
            geometry: Some(Geometry::Point(point)),
            properties: feature.properties.clone(),
        });
    }
    let mut checks = vec![Check::new(
        "one_point_per_feature",
        out.features.len()
            == layer
                .features
                .iter()
                .filter(|f| f.geometry.is_some())
                .count(),
        format!("{} points", out.features.len()),
    )];
    if inside {
        checks.push(Check::new(
            "interior_points_inside",
            outside == 0,
            format!("{outside} points fall outside their polygon"),
        ));
    }
    Ok(OpOutput {
        layer: out,
        checks,
        notes: vec![if inside {
            "interior point (guaranteed inside)"
        } else {
            "planar centroid in the layer CRS"
        }
        .into()],
    })
}

fn measure(layer: &Layer, params: &Value) -> Result<OpOutput> {
    let (area_factor, area_label, area_field) =
        match str_param(params, "area_unit").unwrap_or("km2") {
            "m2" | "m²" | "㎡" => (1.0, "m²", "area_m2"),
            "ha" => (1e4, "ha", "area_ha"),
            "km2" | "km²" | "㎢" => (1e6, "km²", "area_km2"),
            other => {
                return Err(ToolkitError::Unit(format!(
                    "unsupported area unit {other}; use m2, ha, or km2"
                )))
            }
        };
    let (length_factor, length_label, length_suffix) =
        match str_param(params, "length_unit").unwrap_or("km") {
            "m" => (1.0, "m", "m"),
            "km" => (1e3, "km", "km"),
            other => {
                return Err(ToolkitError::Unit(format!(
                    "unsupported length unit {other}; use m or km"
                )))
            }
        };
    let kind = layer.geometry_kind();
    let metrics: Vec<String> = match params.get("metrics") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        Some(_) => {
            return Err(ToolkitError::parameter(
                "measure",
                "metrics must be an array",
            ))
        }
        None => match kind {
            crate::GeometryKind::Polygon => vec!["area".into()],
            crate::GeometryKind::Line => vec!["length".into()],
            _ => vec!["area".into(), "length".into()],
        },
    };
    for m in &metrics {
        if !["area", "length", "perimeter"].contains(&m.as_str()) {
            return Err(ToolkitError::parameter(
                "measure",
                format!("unknown metric {m}"),
            ));
        }
    }
    let source = layer.crs_info()?;
    let wgs84 = proj::lookup_epsg(4326)?;
    let work = working_crs(layer)?;
    let mut out = layer.clone();
    out.name = format!("{}_measured", layer.name);
    out.crs_status = CrsStatus::Derived;
    out.provenance = Default::default();
    let length_field = format!("length_{length_suffix}");
    let perimeter_field = format!("perimeter_{length_suffix}");
    let mut worst = 0.0f64;
    for feature in &mut out.features {
        let Some(geometry) = &feature.geometry else {
            continue;
        };
        // Karney's geodesic area interprets ring direction: a clockwise
        // exterior would be measured as "the whole Earth minus the polygon".
        // Normalise to OGC orientation (exterior CCW, holes CW) first.
        let geographic = orient_ogc(proj::transform_geometry(&source, &wgs84, geometry)?);
        if metrics.iter().any(|m| m == "area") {
            let area = geographic.geodesic_area_unsigned();
            // Independent planar check in the metric working CRS.
            let planar = proj::transform_geometry(&source, &work, geometry)?.unsigned_area();
            if area > 1.0 {
                worst = worst.max((planar - area).abs() / area);
            }
            feature
                .properties
                .insert(area_field.into(), number_value(area / area_factor));
        }
        if metrics.iter().any(|m| m == "length") {
            let length = match &geographic {
                Geometry::LineString(l) => Geodesic.length(l),
                Geometry::MultiLineString(m) => Geodesic.length(m),
                Geometry::Line(l) => Geodesic.length(l),
                _ => 0.0,
            };
            feature
                .properties
                .insert(length_field.clone(), number_value(length / length_factor));
        }
        if metrics.iter().any(|m| m == "perimeter") {
            let perimeter = geographic.geodesic_perimeter();
            feature.properties.insert(
                perimeter_field.clone(),
                number_value(perimeter / length_factor),
            );
        }
    }
    out.refresh_schema();
    out.set_field_unit(area_field, area_label);
    out.set_field_unit(&length_field, length_label);
    out.set_field_unit(&perimeter_field, length_label);
    let wide = layer.bbox_wgs84().is_some_and(|b| b[2] - b[0] > 6.0);
    Ok(OpOutput {
        layer: out,
        checks: vec![Check::new(
            "geodesic_vs_planar_area",
            wide || worst <= 0.01,
            if wide {
                "extent wider than one UTM zone; planar comparison skipped".to_string()
            } else {
                format!(
                    "max relative difference geodesic vs {} planar area {:.4}%",
                    work.id,
                    worst * 100.0
                )
            },
        )],
        notes: vec!["geodesic measurement on the WGS 84 ellipsoid (Karney)".into()],
    })
}

fn orient_ogc(geometry: Geometry<f64>) -> Geometry<f64> {
    use geo::orient::{Direction, Orient};
    match geometry {
        Geometry::Polygon(p) => Geometry::Polygon(p.orient(Direction::Default)),
        Geometry::MultiPolygon(m) => Geometry::MultiPolygon(m.orient(Direction::Default)),
        Geometry::GeometryCollection(c) => Geometry::GeometryCollection(GeometryCollection(
            c.0.into_iter().map(orient_ogc).collect(),
        )),
        other => other,
    }
}

fn filter(layer: &Layer, params: &Value) -> Result<OpOutput> {
    let text = str_param(params, "where").unwrap_or_default();
    let expression = expr::parse(text)?;
    for field in expression.fields() {
        if layer.field(&field).is_none() {
            return Err(ToolkitError::Expression(format!("unknown field {field}")));
        }
    }
    let mut out = derived(format!("{}_filtered", layer.name), &layer.crs);
    out.fields = layer.fields.clone();
    let mut rejected = 0;
    for feature in &layer.features {
        if expression.eval(&with_nulls(layer, &feature.properties))? == Value::Bool(true) {
            out.features.push(feature.clone());
        } else {
            rejected += 1;
        }
    }
    Ok(OpOutput {
        checks: vec![Check::new(
            "partition_complete",
            out.features.len() + rejected == layer.features.len(),
            format!(
                "{} kept + {rejected} rejected = {}",
                out.features.len(),
                layer.features.len()
            ),
        )],
        layer: out,
        notes: vec![format!("filter: {text}")],
    })
}

fn calculate(layer: &Layer, params: &Value) -> Result<OpOutput> {
    let field = str_param(params, "field").unwrap_or_default().to_string();
    let text = str_param(params, "expression").unwrap_or_default();
    let expression = expr::parse(text)?;
    for name in expression.fields() {
        if layer.field(&name).is_none() {
            return Err(ToolkitError::Expression(format!("unknown field {name}")));
        }
    }
    let mut out = layer.clone();
    out.name = format!("{}_calc", layer.name);
    out.crs_status = CrsStatus::Derived;
    out.provenance = Default::default();
    let mut nulls = 0;
    for feature in &mut out.features {
        let value = expression.eval(&with_nulls(layer, &feature.properties))?;
        if value.is_null() {
            nulls += 1;
        }
        feature.properties.insert(field.clone(), value);
    }
    out.refresh_schema();
    if let Some(unit) = str_param(params, "unit") {
        out.set_field_unit(&field, unit);
    }
    Ok(OpOutput {
        layer: out,
        checks: vec![Check::new(
            "values_computed",
            nulls < layer.features.len() || layer.features.is_empty(),
            format!("{field} = {text}; {nulls} null results (missing inputs or division by zero)"),
        )],
        notes: vec![],
    })
}

/// Group key value and weighted members.
type Group<'a> = (Value, Vec<(&'a Feature, f64)>);

fn summarize(layer: &Layer, params: &Value) -> Result<OpOutput> {
    let by = str_param(params, "by");
    if let Some(by) = by {
        if layer.field(by).is_none() {
            return Err(ToolkitError::parameter(
                "summarize",
                format!("unknown field {by}"),
            ));
        }
    }
    let aggregates = aggregates_param("summarize", params, layer, false)?;
    let mut groups: BTreeMap<String, Group> = BTreeMap::new();
    for feature in &layer.features {
        let key = by
            .and_then(|b| feature.properties.get(b))
            .cloned()
            .unwrap_or(Value::Null);
        groups
            .entry(crate::layer::canonical_json(&key))
            .or_insert_with(|| (key.clone(), Vec::new()))
            .1
            .push((feature, 1.0));
    }
    if by.is_none() && groups.is_empty() {
        // A total over nothing is still one answer: count 0, sum 0.
        groups.insert(String::new(), (Value::Null, Vec::new()));
    }
    let mut out = derived(format!("{}_summary", layer.name), &layer.crs);
    let mut member_total = 0;
    for (index, (_, (key, members))) in groups.into_iter().enumerate() {
        member_total += members.len();
        let mut properties = BTreeMap::new();
        if let Some(by) = by {
            properties.insert(by.to_string(), key);
        }
        for aggregate in &aggregates {
            properties.insert(
                aggregate.output_name(),
                aggregate_values(aggregate, &members),
            );
        }
        out.features.push(Feature {
            id: index as u64,
            geometry: None,
            properties,
        });
    }
    out.refresh_schema();
    for aggregate in &aggregates {
        if let Some(unit) = aggregate_unit(aggregate, layer) {
            out.set_field_unit(&aggregate.output_name(), unit);
        }
    }
    Ok(OpOutput {
        layer: out,
        checks: vec![Check::new(
            "members_conserved",
            member_total == layer.features.len(),
            format!(
                "{member_total} grouped of {} features",
                layer.features.len()
            ),
        )],
        notes: vec![],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo_types::polygon;

    fn points(coords: &[(f64, f64, i64)]) -> Layer {
        let mut layer = Layer::new("pts", "EPSG:4326", CrsStatus::Declared);
        for (i, (x, y, v)) in coords.iter().enumerate() {
            layer.features.push(Feature {
                id: i as u64,
                geometry: Some(Geometry::Point(Point::new(*x, *y))),
                properties: BTreeMap::from([("v".to_string(), Value::from(*v))]),
            });
        }
        layer.refresh_schema();
        layer
    }

    fn squares() -> Layer {
        // Two adjacent ~1.1 km squares near Nagoya with population.
        let mut layer = Layer::new("zones", "EPSG:4326", CrsStatus::Declared);
        for (i, (x0, pop)) in [(136.90, 1000), (136.91, 3000)].iter().enumerate() {
            let x0 = *x0;
            layer.features.push(Feature {
                id: i as u64,
                geometry: Some(Geometry::Polygon(polygon![
                    (x: x0, y: 35.16), (x: x0 + 0.01, y: 35.16), (x: x0 + 0.01, y: 35.17), (x: x0, y: 35.17), (x: x0, y: 35.16)
                ])),
                properties: BTreeMap::from([
                    ("name".to_string(), Value::from(format!("z{i}"))),
                    ("pop".to_string(), Value::from(*pop)),
                ]),
            });
        }
        layer.refresh_schema();
        layer.set_field_unit("pop", "persons");
        layer
    }

    fn inputs(pairs: &[(&str, &Layer)]) -> BTreeMap<String, Layer> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), (*v).clone()))
            .collect()
    }

    fn all_pass(output: &OpOutput) {
        for check in &output.checks {
            assert!(check.passed, "check {} failed: {}", check.id, check.detail);
        }
    }

    #[test]
    fn distance_units_are_mandatory() {
        assert_eq!(parse_length(&json!("500 m")).unwrap(), 500.0);
        assert_eq!(parse_length(&json!("1.5km")).unwrap(), 1500.0);
        assert_eq!(
            parse_length(&json!({"value": 2, "unit": "キロ"})).unwrap(),
            2000.0
        );
        assert!(parse_length(&json!(500)).is_err());
        assert!(parse_length(&json!("0.01 deg")).is_err());
        let error = run(
            "buffer",
            &inputs(&[("layer", &points(&[(136.9, 35.1, 1)]))]),
            &json!({"distance": 500}),
        )
        .unwrap_err();
        assert!(matches!(error, ToolkitError::Unit(_)), "{error}");
    }

    #[test]
    fn buffers_points_in_metres() {
        let output = run(
            "buffer",
            &inputs(&[("layer", &points(&[(136.9, 35.1, 1)]))]),
            &json!({"distance": "1 km"}),
        )
        .unwrap();
        all_pass(&output);
        assert_eq!(output.layer.crs, "EPSG:4326");
        assert_eq!(
            output.layer.field("buffer_m").unwrap().unit.as_deref(),
            Some("m")
        );
        let area = output.layer.features[0]
            .geometry
            .as_ref()
            .unwrap()
            .geodesic_area_unsigned();
        let expected = std::f64::consts::PI * 1e6;
        assert!((area - expected).abs() / expected < 0.02, "area {area}");
        assert!(
            output.notes.iter().any(|n| n.contains("EPSG:32653")),
            "{:?}",
            output.notes
        );
    }

    #[test]
    fn counts_points_in_polygons_with_independent_check() {
        let pts = points(&[
            (136.905, 35.165, 1),
            (136.906, 35.166, 2),
            (136.915, 35.165, 3),
            (137.5, 35.5, 4),
        ]);
        let output = run(
            "spatial_join",
            &inputs(&[("target", &squares()), ("join", &pts)]),
            &json!({"aggregates": [{"op": "count"}, {"op": "sum", "field": "v"}]}),
        )
        .unwrap();
        all_pass(&output);
        assert_eq!(output.layer.features[0].properties["count"], 2);
        assert_eq!(output.layer.features[1].properties["count"], 1);
        assert_eq!(output.layer.features[0].properties["sum_v"], 3.0);
        assert_eq!(
            output.layer.field("count").unwrap().unit.as_deref(),
            Some("features")
        );
    }

    #[test]
    fn apportions_population_to_a_buffer_by_area() {
        // A 300 m buffer around a point in the middle of zone z0.
        let centre = points(&[(136.905, 35.165, 0)]);
        let buffered = run(
            "buffer",
            &inputs(&[("layer", &centre)]),
            &json!({"distance": "300 m"}),
        )
        .unwrap()
        .layer;
        let output = run(
            "spatial_join",
            &inputs(&[("target", &buffered), ("join", &squares())]),
            &json!({"aggregates": [{"op": "area_weighted_sum", "field": "pop", "as": "population"}]}),
        )
        .unwrap();
        all_pass(&output);
        let population = output.layer.features[0].properties["population"]
            .as_f64()
            .unwrap();
        // Circle area / square area ≈ π·0.3² / (0.9109 × 1.1119) ≈ 0.279 → ≈ 279 persons.
        assert!(
            (250.0..310.0).contains(&population),
            "population {population}"
        );
        assert_eq!(
            output.layer.field("population").unwrap().unit.as_deref(),
            Some("persons")
        );
    }

    #[test]
    fn selects_within_distance_and_verifies_geodesically() {
        let stations = points(&[(136.8815, 35.1709, 0)]);
        let shelters = points(&[(136.8840, 35.1709, 1), (136.8950, 35.1709, 2)]);
        let output = run(
            "select_by_location",
            &inputs(&[("layer", &shelters), ("other", &stations)]),
            &json!({"predicate": "within_distance", "distance": "500 m"}),
        )
        .unwrap();
        all_pass(&output);
        assert_eq!(output.layer.features.len(), 1);
        assert_eq!(output.layer.features[0].properties["v"], 1);
    }

    #[test]
    fn clip_erase_dissolve_and_measure() {
        let zones = squares();
        let mask = {
            let mut m = Layer::new("mask", "EPSG:4326", CrsStatus::Declared);
            m.features.push(Feature {
                id: 0,
                geometry: Some(Geometry::Polygon(polygon![(x: 136.905, y: 35.15), (x: 136.915, y: 35.15), (x: 136.915, y: 35.18), (x: 136.905, y: 35.18), (x: 136.905, y: 35.15)])),
                properties: BTreeMap::new(),
            });
            m
        };
        let clipped = run(
            "clip",
            &inputs(&[("layer", &zones), ("mask", &mask)]),
            &json!({}),
        )
        .unwrap();
        all_pass(&clipped);
        assert_eq!(clipped.layer.features.len(), 2);
        let erased = run(
            "erase",
            &inputs(&[("layer", &zones), ("mask", &mask)]),
            &json!({}),
        )
        .unwrap();
        all_pass(&erased);
        let dissolved = run(
            "dissolve",
            &inputs(&[("layer", &zones)]),
            &json!({"aggregates": [{"op": "count"}, {"op": "sum", "field": "pop"}]}),
        )
        .unwrap();
        all_pass(&dissolved);
        assert_eq!(dissolved.layer.features.len(), 1);
        assert_eq!(dissolved.layer.features[0].properties["sum_pop"], 4000.0);
        let measured = run(
            "measure",
            &inputs(&[("layer", &zones)]),
            &json!({"area_unit": "km2"}),
        )
        .unwrap();
        all_pass(&measured);
        let area = measured.layer.features[0].properties["area_km2"]
            .as_f64()
            .unwrap();
        assert!((1.0..1.02).contains(&area), "area {area}");
        assert_eq!(
            measured.layer.field("area_km2").unwrap().unit.as_deref(),
            Some("km²")
        );
        let density = run(
            "calculate",
            &inputs(&[("layer", &measured.layer)]),
            &json!({"field": "density", "expression": "pop / area_km2", "unit": "persons/km²"}),
        )
        .unwrap();
        assert!(
            density.layer.features[1].properties["density"]
                .as_f64()
                .unwrap()
                > 2900.0
        );
    }

    #[test]
    fn invalid_polygons_fail_closed_until_repaired() {
        let mut bowtie = Layer::new("bowtie", "EPSG:4326", CrsStatus::Declared);
        bowtie.features.push(Feature {
            id: 0,
            geometry: Some(Geometry::Polygon(polygon![
                (x: 136.90, y: 35.16), (x: 136.92, y: 35.18), (x: 136.92, y: 35.16), (x: 136.90, y: 35.18), (x: 136.90, y: 35.16)
            ])),
            properties: BTreeMap::new(),
        });
        assert_eq!(bowtie.invalid_feature_ids(), vec![0]);
        let error = run(
            "buffer",
            &inputs(&[("layer", &bowtie)]),
            &json!({"distance": "100 m"}),
        )
        .unwrap_err();
        assert!(error.to_string().contains("make_valid"), "{error}");
        let repaired = run("make_valid", &inputs(&[("layer", &bowtie)]), &json!({})).unwrap();
        all_pass(&repaired);
        let buffered = run(
            "buffer",
            &inputs(&[("layer", &repaired.layer)]),
            &json!({"distance": "100 m"}),
        )
        .unwrap();
        all_pass(&buffered);
    }

    #[test]
    fn buffer_and_union_ignore_ring_direction() {
        let mut clockwise = squares();
        for feature in &mut clockwise.features {
            if let Some(Geometry::Polygon(p)) = &mut feature.geometry {
                let mut exterior = p.exterior().clone();
                exterior.0.reverse();
                *p = Polygon::new(exterior, vec![]);
            }
        }
        let buffered = run(
            "buffer",
            &inputs(&[("layer", &clockwise)]),
            &json!({"distance": "200 m", "dissolve": true}),
        )
        .unwrap();
        all_pass(&buffered);
        let area = buffered.layer.features[0]
            .geometry
            .as_ref()
            .unwrap()
            .geodesic_area_unsigned();
        // Two adjacent ~1.01 km² squares grown by 200 m ≈ 3.1 km².
        assert!((2.5e6..3.8e6).contains(&area), "buffered area {area} m²");
        let dissolved = run("dissolve", &inputs(&[("layer", &clockwise)]), &json!({})).unwrap();
        all_pass(&dissolved);
    }

    #[test]
    fn geodesic_area_ignores_ring_direction() {
        let mut clockwise = squares();
        for feature in &mut clockwise.features {
            if let Some(Geometry::Polygon(p)) = &mut feature.geometry {
                let mut exterior = p.exterior().clone();
                exterior.0.reverse();
                *p = Polygon::new(exterior, vec![]);
            }
        }
        let measured = run("measure", &inputs(&[("layer", &clockwise)]), &json!({})).unwrap();
        all_pass(&measured);
        let area = measured.layer.features[0].properties["area_km2"]
            .as_f64()
            .unwrap();
        assert!(
            (1.0..1.02).contains(&area),
            "clockwise ring measured as {area} km²"
        );
    }

    #[test]
    fn rejects_unknown_parameters_and_inputs() {
        let layer = points(&[(136.9, 35.1, 1)]);
        assert!(run(
            "buffer",
            &inputs(&[("layer", &layer)]),
            &json!({"distance": "1 km", "radius": 3})
        )
        .is_err());
        assert!(run(
            "buffer",
            &inputs(&[("mask", &layer)]),
            &json!({"distance": "1 km"})
        )
        .is_err());
        assert!(run("teleport", &inputs(&[]), &json!({})).is_err());
    }

    #[test]
    fn assign_crs_declares_without_moving_coordinates() {
        let output = run(
            "assign_crs",
            &inputs(&[("layer", &squares())]),
            &json!({"crs": "EPSG:6668"}),
        )
        .unwrap();
        all_pass(&output);
        assert_eq!(output.layer.crs, "EPSG:6668");
        assert_eq!(output.layer.crs_status, CrsStatus::UserSupplied);
        assert_eq!(
            output.layer.features[0].geometry,
            squares().features[0].geometry
        );
        let bad = run(
            "assign_crs",
            &inputs(&[("layer", &points(&[(-26000.0, -92000.0, 0)]))]),
            &json!({"crs": "EPSG:4326"}),
        )
        .unwrap();
        assert!(!bad.checks[0].passed);
    }

    #[test]
    fn reprojects_with_round_trip_check() {
        let output = run(
            "reproject",
            &inputs(&[("layer", &squares())]),
            &json!({"crs": "EPSG:6675"}),
        )
        .unwrap();
        all_pass(&output);
        assert_eq!(output.layer.crs, "EPSG:6675");
        assert!(output.notes.iter().any(|n| n.contains("coincident")));
    }
}
