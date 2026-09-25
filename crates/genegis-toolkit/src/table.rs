//! Attribute inspection: paged table queries, field statistics,
//! classification for choropleths, and feature picking.

use std::collections::BTreeMap;

use geo::{BoundingRect, Distance, Euclidean, Intersects};
use geo_types::{Geometry, Point};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Result, ToolkitError};
use crate::expr;
use crate::geojson_io::geometry_to_json;
use crate::layer::{FieldType, Layer};
use crate::proj;

/// Paged table query.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TableQuery {
    /// Filter expression (see [`crate::expr`]).
    #[serde(rename = "where")]
    pub filter: Option<String>,
    /// Field to sort by.
    pub sort_by: Option<String>,
    /// Sort descending.
    pub descending: bool,
    /// Rows to skip.
    pub offset: usize,
    /// Maximum rows to return (default 100, max 5000).
    pub limit: Option<usize>,
}

/// One table row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableRow {
    /// Feature ID.
    pub id: u64,
    /// Attribute values.
    pub properties: BTreeMap<String, Value>,
}

/// Result page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TablePage {
    /// Rows matching the filter before paging.
    pub total_matched: usize,
    /// Rows in the layer.
    pub total_rows: usize,
    /// Returned rows.
    pub rows: Vec<TableRow>,
}

/// Run a paged, filtered, sorted query over a layer's attributes.
pub fn query(layer: &Layer, query: &TableQuery) -> Result<TablePage> {
    let filter = query
        .filter
        .as_deref()
        .filter(|f| !f.trim().is_empty())
        .map(expr::parse)
        .transpose()?;
    if let Some(filter) = &filter {
        for field in filter.fields() {
            if layer.field(&field).is_none() {
                return Err(ToolkitError::Expression(format!("unknown field {field}")));
            }
        }
    }
    let mut matched = Vec::new();
    for feature in &layer.features {
        let keep = match &filter {
            Some(f) => matches!(
                f.eval(&with_nulls(layer, &feature.properties))?,
                Value::Bool(true)
            ),
            None => true,
        };
        if keep {
            matched.push(feature);
        }
    }
    if let Some(sort) = &query.sort_by {
        if layer.field(sort).is_none() {
            return Err(ToolkitError::Expression(format!(
                "unknown sort field {sort}"
            )));
        }
        matched.sort_by(|a, b| {
            let ordering = order_values(a.properties.get(sort), b.properties.get(sort));
            if query.descending {
                ordering.reverse()
            } else {
                ordering
            }
        });
    }
    let limit = query.limit.unwrap_or(100).min(5000);
    Ok(TablePage {
        total_matched: matched.len(),
        total_rows: layer.features.len(),
        rows: matched
            .into_iter()
            .skip(query.offset)
            .take(limit)
            .map(|f| TableRow {
                id: f.id,
                properties: f.properties.clone(),
            })
            .collect(),
    })
}

/// Properties with explicit nulls for fields missing on this feature, so
/// expressions can reference any schema field.
pub(crate) fn with_nulls(
    layer: &Layer,
    properties: &BTreeMap<String, Value>,
) -> BTreeMap<String, Value> {
    let mut out = properties.clone();
    for field in &layer.fields {
        out.entry(field.name.clone()).or_insert(Value::Null);
    }
    out
}

fn order_values(a: Option<&Value>, b: Option<&Value>) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a.filter(|v| !v.is_null()), b.filter(|v| !v.is_null())) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
            _ => x.to_string().cmp(&y.to_string()),
        },
    }
}

/// Summary statistics for one field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldStats {
    /// Field name.
    pub field: String,
    /// Field type.
    #[serde(rename = "type")]
    pub field_type: FieldType,
    /// Unit, when known.
    pub unit: Option<String>,
    /// Non-null values.
    pub count: usize,
    /// Null values.
    pub nulls: usize,
    /// Distinct non-null values (capped at 10 000).
    pub distinct: usize,
    /// Numeric minimum.
    pub min: Option<f64>,
    /// Numeric maximum.
    pub max: Option<f64>,
    /// Numeric mean.
    pub mean: Option<f64>,
    /// Numeric sum.
    pub sum: Option<f64>,
    /// Most frequent values (text/categorical fields).
    pub top_values: Vec<(String, usize)>,
}

/// Compute statistics for a field.
pub fn field_stats(layer: &Layer, field: &str) -> Result<FieldStats> {
    let schema = layer
        .field(field)
        .ok_or_else(|| ToolkitError::Expression(format!("unknown field {field}")))?;
    let numeric = matches!(schema.field_type, FieldType::Integer | FieldType::Float);
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let (mut count, mut nulls, mut sum) = (0usize, 0usize, 0.0f64);
    let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
    for feature in &layer.features {
        match feature.properties.get(field) {
            None | Some(Value::Null) => nulls += 1,
            Some(value) => {
                count += 1;
                if numeric {
                    if let Some(v) = value.as_f64() {
                        sum += v;
                        min = min.min(v);
                        max = max.max(v);
                    }
                }
                if counts.len() < 10_000 || counts.contains_key(&display(value)) {
                    *counts.entry(display(value)).or_default() += 1;
                }
            }
        }
    }
    let mut top: Vec<(String, usize)> = counts.iter().map(|(k, v)| (k.clone(), *v)).collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    top.truncate(10);
    Ok(FieldStats {
        field: field.to_string(),
        field_type: schema.field_type,
        unit: schema.unit.clone(),
        count,
        nulls,
        distinct: counts.len(),
        min: (numeric && count > 0).then_some(min),
        max: (numeric && count > 0).then_some(max),
        mean: (numeric && count > 0).then(|| sum / count as f64),
        sum: (numeric && count > 0).then_some(sum),
        top_values: if numeric { Vec::new() } else { top },
    })
}

fn display(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

/// Classification method for thematic maps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassMethod {
    /// Equal-width intervals.
    EqualInterval,
    /// Equal-count classes.
    Quantile,
    /// Jenks natural breaks (Fisher's exact optimisation).
    NaturalBreaks,
    /// One class per distinct value.
    Categorical,
}

/// Classification request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassifyRequest {
    /// Field to classify.
    pub field: String,
    /// Method.
    pub method: ClassMethod,
    /// Number of classes (2–9; ignored for categorical).
    #[serde(default = "default_classes")]
    pub classes: usize,
}

fn default_classes() -> usize {
    5
}

/// One legend class.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegendClass {
    /// Legend label.
    pub label: String,
    /// Lower bound (inclusive) for numeric classes.
    pub min: Option<f64>,
    /// Upper bound (inclusive for the last class) for numeric classes.
    pub max: Option<f64>,
    /// Category value for categorical classes.
    pub value: Option<String>,
    /// Fill colour `#rrggbb`.
    pub color: String,
    /// Features in the class.
    pub count: usize,
}

/// Classification result used by the map and exports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Classification {
    /// Field classified.
    pub field: String,
    /// Field unit.
    pub unit: Option<String>,
    /// Method used.
    pub method: ClassMethod,
    /// Legend classes in order.
    pub classes: Vec<LegendClass>,
    /// Class index per feature ID (`None` for null values).
    pub assignments: BTreeMap<u64, Option<usize>>,
}

/// Sequential palette (colour-blind-safe yellow→blue, ColorBrewer YlGnBu).
const SEQUENTIAL: [&str; 9] = [
    "#ffffd9", "#edf8b1", "#c7e9b4", "#7fcdbb", "#41b6c4", "#1d91c0", "#225ea8", "#253494",
    "#081d58",
];
/// Qualitative palette (ColorBrewer Set2 + Dark2).
const QUALITATIVE: [&str; 12] = [
    "#66c2a5", "#fc8d62", "#8da0cb", "#e78ac3", "#a6d854", "#ffd92f", "#e5c494", "#b3b3b3",
    "#1b9e77", "#d95f02", "#7570b3", "#e7298a",
];

fn sequential(n: usize) -> Vec<String> {
    (0..n)
        .map(|i| {
            let idx = if n == 1 { 4 } else { 1 + i * 7 / (n - 1) };
            SEQUENTIAL[idx.min(8)].to_string()
        })
        .collect()
}

/// Classify a field for a thematic map.
pub fn classify(layer: &Layer, request: &ClassifyRequest) -> Result<Classification> {
    let schema = layer
        .field(&request.field)
        .ok_or_else(|| ToolkitError::Expression(format!("unknown field {}", request.field)))?;
    let numeric = matches!(schema.field_type, FieldType::Integer | FieldType::Float);
    let mut assignments = BTreeMap::new();
    let method = if !numeric {
        ClassMethod::Categorical
    } else {
        request.method
    };
    if method == ClassMethod::Categorical {
        let mut values: Vec<String> = layer
            .features
            .iter()
            .filter_map(|f| {
                f.properties
                    .get(&request.field)
                    .filter(|v| !v.is_null())
                    .map(display)
            })
            .collect();
        values.sort();
        values.dedup();
        let shown = values.len().min(QUALITATIVE.len() - 1);
        let mut classes: Vec<LegendClass> = values
            .iter()
            .take(shown)
            .enumerate()
            .map(|(i, v)| LegendClass {
                label: v.clone(),
                min: None,
                max: None,
                value: Some(v.clone()),
                color: QUALITATIVE[i].to_string(),
                count: 0,
            })
            .collect();
        if values.len() > shown {
            classes.push(LegendClass {
                label: format!("その他 ({} 種)", values.len() - shown),
                min: None,
                max: None,
                value: None,
                color: "#cccccc".into(),
                count: 0,
            });
        }
        for feature in &layer.features {
            let index = feature
                .properties
                .get(&request.field)
                .filter(|v| !v.is_null())
                .map(|v| {
                    let key = display(v);
                    classes
                        .iter()
                        .position(|c| c.value.as_deref() == Some(key.as_str()))
                        .unwrap_or(classes.len() - 1)
                });
            if let Some(i) = index {
                classes[i].count += 1;
            }
            assignments.insert(feature.id, index);
        }
        return Ok(Classification {
            field: request.field.clone(),
            unit: schema.unit.clone(),
            method,
            classes,
            assignments,
        });
    }

    let mut values: Vec<f64> = layer
        .features
        .iter()
        .filter_map(|f| f.properties.get(&request.field).and_then(Value::as_f64))
        .filter(|v| v.is_finite())
        .collect();
    if values.is_empty() {
        return Err(ToolkitError::Expression(format!(
            "{} has no numeric values",
            request.field
        )));
    }
    values.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
    let k = request.classes.clamp(2, 9);
    let (lo, hi) = (values[0], values[values.len() - 1]);
    let breaks: Vec<f64> = match method {
        ClassMethod::EqualInterval => (1..k)
            .map(|i| lo + (hi - lo) * i as f64 / k as f64)
            .collect(),
        ClassMethod::Quantile => (1..k)
            .map(|i| values[((values.len() * i) / k).min(values.len() - 1)])
            .collect(),
        ClassMethod::NaturalBreaks => jenks(&values, k),
        ClassMethod::Categorical => unreachable!(),
    };
    // Class i spans [bounds[i], bounds[i + 1]); the last class is closed.
    // A break equal to the maximum is a legitimate class holding only the
    // maximum (natural breaks isolates outliers this way).
    let mut bounds = vec![lo];
    for b in breaks {
        if b > *bounds.last().expect("non-empty") && b <= hi {
            bounds.push(b);
        }
    }
    bounds.push(hi);
    let n = bounds.len() - 1;
    let colors = sequential(n);
    let unit = schema.unit.clone();
    let mut classes: Vec<LegendClass> = (0..n)
        .map(|i| LegendClass {
            label: format!(
                "{} – {}{}",
                fmt_num(bounds[i]),
                fmt_num(bounds[i + 1]),
                unit.as_ref().map(|u| format!(" {u}")).unwrap_or_default()
            ),
            min: Some(bounds[i]),
            max: Some(bounds[i + 1]),
            value: None,
            color: colors[i].clone(),
            count: 0,
        })
        .collect();
    for feature in &layer.features {
        let index = feature
            .properties
            .get(&request.field)
            .and_then(Value::as_f64)
            .map(|v| {
                (0..n)
                    .find(|&i| v < bounds[i + 1] || i == n - 1)
                    .unwrap_or(n - 1)
            });
        if let Some(i) = index {
            classes[i].count += 1;
        }
        assignments.insert(feature.id, index);
    }
    Ok(Classification {
        field: request.field.clone(),
        unit,
        method,
        classes,
        assignments,
    })
}

fn fmt_num(v: f64) -> String {
    if v.abs() >= 1000.0 || v.fract() == 0.0 {
        let rounded = v.round() as i64;
        let s = rounded.abs().to_string();
        let mut out = String::new();
        for (i, c) in s.chars().enumerate() {
            if i > 0 && (s.len() - i).is_multiple_of(3) {
                out.push(',');
            }
            out.push(c);
        }
        if rounded < 0 {
            format!("-{out}")
        } else {
            out
        }
    } else if v.abs() >= 1.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.3}")
    }
}

/// Jenks natural breaks via Fisher's dynamic programming on sorted values
/// (sampled to at most 2 000 values for interactive latency).
fn jenks(sorted: &[f64], k: usize) -> Vec<f64> {
    let data: Vec<f64> = if sorted.len() > 2000 {
        (0..2000)
            .map(|i| sorted[i * (sorted.len() - 1) / 1999])
            .collect()
    } else {
        sorted.to_vec()
    };
    let n = data.len();
    if n <= k {
        return data.iter().skip(1).copied().collect();
    }
    let mut lower = vec![vec![0usize; k + 1]; n + 1];
    let mut variance = vec![vec![f64::INFINITY; k + 1]; n + 1];
    for j in 1..=k {
        lower[1][j] = 1;
        variance[1][j] = 0.0;
    }
    for l in 2..=n {
        let (mut s1, mut s2, mut w) = (0.0, 0.0, 0.0);
        let mut v = 0.0;
        for m in 1..=l {
            let i3 = l - m + 1;
            let val = data[i3 - 1];
            s2 += val * val;
            s1 += val;
            w += 1.0;
            v = s2 - (s1 * s1) / w;
            let i4 = i3 - 1;
            if i4 != 0 {
                for j in 2..=k {
                    if variance[l][j] >= v + variance[i4][j - 1] {
                        lower[l][j] = i3;
                        variance[l][j] = v + variance[i4][j - 1];
                    }
                }
            }
        }
        lower[l][1] = 1;
        variance[l][1] = v;
    }
    let mut breaks = vec![0.0; k - 1];
    let mut idx = n;
    for j in (2..=k).rev() {
        let start = lower[idx][j].max(2);
        breaks[j - 2] = data[start - 1];
        idx = start - 1;
    }
    breaks
}

/// Picked feature.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PickedFeature {
    /// Feature ID.
    pub id: u64,
    /// Distance from the pick point in metres (0 when inside).
    pub distance_m: f64,
    /// Attributes.
    pub properties: BTreeMap<String, Value>,
    /// Geometry as GeoJSON in EPSG:4326.
    pub geometry: Value,
}

/// Return features at (or within `tolerance_m` of) a WGS84 point, nearest first.
pub fn pick(
    layer: &Layer,
    lon: f64,
    lat: f64,
    tolerance_m: f64,
    limit: usize,
) -> Result<Vec<PickedFeature>> {
    let layer_crs = layer.crs_info()?;
    let wgs84 = proj::lookup_epsg(4326)?;
    let metric = proj::metric_crs_for(lon, lat);
    let (px, py) = proj::from_geographic(&metric, lon, lat)?;
    let probe = Point::new(px, py);
    let (qx, qy) = proj::transform_coord(&wgs84, &layer_crs, lon, lat)?;
    let pad = if layer_crs.is_geographic() {
        tolerance_m / 90_000.0
    } else {
        tolerance_m * 1.5
    };
    let mut hits = Vec::new();
    for feature in &layer.features {
        let Some(geometry) = &feature.geometry else {
            continue;
        };
        let Some(rect) = geometry.bounding_rect() else {
            continue;
        };
        if qx < rect.min().x - pad
            || qx > rect.max().x + pad
            || qy < rect.min().y - pad
            || qy > rect.max().y + pad
        {
            continue;
        }
        let metric_geometry = proj::transform_geometry(&layer_crs, &metric, geometry)?;
        let distance = if metric_geometry.intersects(&probe) {
            0.0
        } else {
            Euclidean.distance(&Geometry::Point(probe), &metric_geometry)
        };
        if distance <= tolerance_m {
            hits.push(PickedFeature {
                id: feature.id,
                distance_m: distance,
                properties: feature.properties.clone(),
                geometry: geometry_to_json(&proj::transform_geometry(
                    &layer_crs, &wgs84, geometry,
                )?),
            });
        }
    }
    hits.sort_by(|a, b| {
        a.distance_m
            .partial_cmp(&b.distance_m)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    hits.truncate(limit.max(1));
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::{CrsStatus, Feature};
    use geo_types::{point, polygon};

    fn layer() -> Layer {
        let mut layer = Layer::new("t", "EPSG:4326", CrsStatus::Declared);
        for (i, (name, pop)) in [
            ("a", 10),
            ("b", 20),
            ("c", 30),
            ("d", 100),
            ("e", 110),
            ("f", 500),
        ]
        .iter()
        .enumerate()
        {
            layer.features.push(Feature {
                id: i as u64,
                geometry: Some(Geometry::Point(point!(x: 136.9 + i as f64 * 0.01, y: 35.1))),
                properties: BTreeMap::from([
                    ("name".to_string(), Value::from(*name)),
                    ("pop".to_string(), Value::from(*pop)),
                ]),
            });
        }
        layer.refresh_schema();
        layer
    }

    #[test]
    fn queries_filter_sort_and_page() {
        let page = query(
            &layer(),
            &TableQuery {
                filter: Some("pop >= 20".into()),
                sort_by: Some("pop".into()),
                descending: true,
                offset: 1,
                limit: Some(2),
            },
        )
        .unwrap();
        assert_eq!(page.total_matched, 5);
        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.rows[0].properties["pop"], 110);
        assert!(query(
            &layer(),
            &TableQuery {
                filter: Some("nope > 1".into()),
                ..Default::default()
            }
        )
        .is_err());
    }

    #[test]
    fn classifies_with_natural_breaks_and_counts_every_feature() {
        let result = classify(
            &layer(),
            &ClassifyRequest {
                field: "pop".into(),
                method: ClassMethod::NaturalBreaks,
                classes: 3,
            },
        )
        .unwrap();
        assert_eq!(result.classes.len(), 3);
        assert_eq!(result.classes.iter().map(|c| c.count).sum::<usize>(), 6);
        // Natural groups: {10,20,30}, {100,110}, {500}.
        assert_eq!(result.classes[0].count, 3);
        assert_eq!(result.classes[2].count, 1);
        let quantile = classify(
            &layer(),
            &ClassifyRequest {
                field: "pop".into(),
                method: ClassMethod::Quantile,
                classes: 2,
            },
        )
        .unwrap();
        assert_eq!(
            quantile.classes.iter().map(|c| c.count).collect::<Vec<_>>(),
            vec![3, 3]
        );
        let categorical = classify(
            &layer(),
            &ClassifyRequest {
                field: "name".into(),
                method: ClassMethod::EqualInterval,
                classes: 5,
            },
        )
        .unwrap();
        assert_eq!(categorical.method, ClassMethod::Categorical);
        assert_eq!(categorical.classes.len(), 6);
    }

    #[test]
    fn picks_points_within_tolerance_and_polygons_containing_the_click() {
        let hits = pick(&layer(), 136.9001, 35.1, 50.0, 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, 0);
        assert!(hits[0].distance_m > 5.0 && hits[0].distance_m < 15.0);

        let mut polys = Layer::new("p", "EPSG:4326", CrsStatus::Declared);
        polys.features.push(Feature {
            id: 7,
            geometry: Some(Geometry::Polygon(polygon![(x: 136.0, y: 35.0), (x: 137.0, y: 35.0), (x: 137.0, y: 36.0), (x: 136.0, y: 36.0)])),
            properties: BTreeMap::new(),
        });
        let hits = pick(&polys, 136.5, 35.5, 1.0, 5).unwrap();
        assert_eq!(hits[0].id, 7);
        assert_eq!(hits[0].distance_m, 0.0);
    }

    #[test]
    fn stats_report_numeric_summary() {
        let stats = field_stats(&layer(), "pop").unwrap();
        assert_eq!(stats.count, 6);
        assert_eq!(stats.sum, Some(770.0));
        assert_eq!(stats.max, Some(500.0));
    }
}
