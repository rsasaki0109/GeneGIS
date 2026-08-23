//! Deterministic, evidence-carrying cartography primitives.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

/// Classification algorithm recorded in the style identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassificationMethod {
    /// Equal-width numeric intervals.
    EqualInterval,
    /// Quantiles computed from an explicitly identified input snapshot.
    Quantile,
    /// User-supplied reviewed breaks.
    Manual,
}

/// One non-overlapping numeric class.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassBreak {
    /// Inclusive lower bound.
    pub minimum: f64,
    /// Upper bound, inclusive only for the last class.
    pub maximum: f64,
    /// CSS hexadecimal fill color (`#rrggbb`).
    pub fill: String,
    /// Human-readable legend label.
    pub label: String,
}

/// Map legend content and placement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegendSpec {
    /// Legend heading.
    pub title: String,
    /// Unit displayed beside numeric values.
    pub unit: String,
    /// Fixed X position in layout pixels.
    pub x: u32,
    /// Fixed Y position in layout pixels.
    pub y: u32,
}

/// Deterministic feature-label settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabelStyle {
    /// Attribute supplying label text.
    pub field: String,
    /// Exact font family requested by the layout.
    pub font_family: String,
    /// SHA-256 identity of the reviewed font file/build.
    pub font_digest: String,
    /// Font size in layout pixels.
    pub size_px: f64,
    /// CSS hexadecimal text color.
    pub color: String,
}

/// Complete layer style stored as project semantic state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceMapStyle {
    /// Stable style identity.
    pub id: Uuid,
    /// Project layer being styled.
    pub layer_id: Uuid,
    /// Numeric classification field.
    pub field: String,
    /// Unit of that field.
    pub value_unit: String,
    /// Recorded classification method.
    pub classification: ClassificationMethod,
    /// Ordered, gap-free class breaks.
    pub breaks: Vec<ClassBreak>,
    /// Legend settings.
    pub legend: LegendSpec,
    /// Optional feature labels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub labels: Option<LabelStyle>,
}

impl EvidenceMapStyle {
    /// Validate classification, legend, label, color, and font identity.
    pub fn validate(&self) -> Result<(), StyleError> {
        if self.field.trim().is_empty()
            || self.value_unit.trim().is_empty()
            || self.legend.title.trim().is_empty()
            || self.legend.unit != self.value_unit
            || self.breaks.is_empty()
        {
            return Err(StyleError::InvalidStyle(
                "field, unit, matching legend unit, title, and breaks are required".into(),
            ));
        }
        for (index, class) in self.breaks.iter().enumerate() {
            if !class.minimum.is_finite()
                || !class.maximum.is_finite()
                || class.maximum <= class.minimum
                || !valid_hex_color(&class.fill)
                || class.label.trim().is_empty()
            {
                return Err(StyleError::InvalidStyle(format!(
                    "class {index} has invalid bounds, fill, or label"
                )));
            }
            if index > 0 && self.breaks[index - 1].maximum != class.minimum {
                return Err(StyleError::InvalidStyle(format!(
                    "classes {} and {index} have a gap or overlap",
                    index - 1
                )));
            }
        }
        if let Some(labels) = &self.labels {
            if labels.field.trim().is_empty()
                || labels.font_family.trim().is_empty()
                || !valid_digest(&labels.font_digest)
                || !labels.size_px.is_finite()
                || !(4.0..=96.0).contains(&labels.size_px)
                || !valid_hex_color(&labels.color)
            {
                return Err(StyleError::InvalidStyle(
                    "labels require field, font identity, 4-96 px size, and color".into(),
                ));
            }
        }
        Ok(())
    }

    fn fill_for(&self, value: f64) -> Result<&str, StyleError> {
        if !value.is_finite() {
            return Err(StyleError::Feature(
                "classification value is not finite".into(),
            ));
        }
        self.breaks
            .iter()
            .enumerate()
            .find(|(index, class)| {
                value >= class.minimum
                    && (value < class.maximum
                        || (*index == self.breaks.len() - 1 && value <= class.maximum))
            })
            .map(|(_, class)| class.fill.as_str())
            .ok_or_else(|| StyleError::Feature(format!("value {value} is outside style breaks")))
    }
}

/// Fixed map page and semantic coordinate identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapLayout {
    /// Pixel width.
    pub width_px: u32,
    /// Pixel height.
    pub height_px: u32,
    /// Map title.
    pub title: String,
    /// Exact CRS identity, for example `EPSG:6675`.
    pub crs: String,
    /// Coordinate unit, for example `metre`.
    pub coordinate_unit: String,
    /// Renderer build identity.
    pub renderer: String,
    /// Padding around the mapped extent.
    pub padding_px: u32,
}

/// Source snapshot identity embedded into map metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactSourceIdentity {
    /// Stable source URI or dataset identity.
    pub source: String,
    /// Immutable content digest.
    pub digest: String,
    /// License identifier.
    pub license: String,
}

/// One polygon supplied to the deterministic SVG renderer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SvgPolygon {
    /// Stable feature identity.
    pub id: String,
    /// Exterior ring, already expressed in the layout CRS.
    pub exterior: Vec<[f64; 2]>,
    /// Numeric value classified by the style.
    pub value: f64,
    /// Optional pre-resolved label text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Machine-readable metadata bound to an exported map.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapArtifactMetadata {
    /// Exact source identities.
    pub sources: Vec<ArtifactSourceIdentity>,
    /// CRS identity.
    pub crs: String,
    /// Coordinate unit.
    pub coordinate_unit: String,
    /// Value unit.
    pub value_unit: String,
    /// Stable digest of style JSON.
    pub style_digest: String,
    /// Font digest, when labels are enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_digest: Option<String>,
    /// Renderer build identity.
    pub renderer: String,
    /// Command/workflow provenance digest authorizing export.
    pub provenance_digest: String,
}

/// Deterministic SVG plus its independently digestible metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapArtifact {
    /// `image/svg+xml`.
    pub media_type: String,
    /// SHA-256 of exact SVG bytes.
    pub artifact_digest: String,
    /// SHA-256 of canonical metadata JSON.
    pub metadata_digest: String,
    /// Exact SVG document.
    pub svg: String,
    /// Evidence bound into the SVG `<metadata>` element.
    pub metadata: MapArtifactMetadata,
}

/// Cartography validation or rendering error.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StyleError {
    /// Style definition is incomplete or ambiguous.
    #[error("invalid map style: {0}")]
    InvalidStyle(String),
    /// Layout definition is unsafe or incomplete.
    #[error("invalid map layout: {0}")]
    Layout(String),
    /// A feature cannot be rendered under the style contract.
    #[error("invalid render feature: {0}")]
    Feature(String),
    /// Source or provenance evidence is missing or malformed.
    #[error("invalid map evidence: {0}")]
    Evidence(String),
    /// Stable JSON serialization failed.
    #[error("map metadata serialization failed: {0}")]
    Serialization(String),
}

/// Render a byte-stable SVG with source, CRS, units, style, font, renderer, and provenance identity.
pub fn render_evidence_svg(
    style: &EvidenceMapStyle,
    layout: &MapLayout,
    features: &[SvgPolygon],
    mut sources: Vec<ArtifactSourceIdentity>,
    provenance_digest: &str,
) -> Result<MapArtifact, StyleError> {
    style.validate()?;
    if layout.width_px < 64
        || layout.height_px < 64
        || layout.padding_px.saturating_mul(2) >= layout.width_px.min(layout.height_px)
        || layout.title.trim().is_empty()
        || layout.crs.trim().is_empty()
        || layout.coordinate_unit.trim().is_empty()
        || layout.renderer.trim().is_empty()
    {
        return Err(StyleError::Layout(
            "page, padding, title, CRS, coordinate unit, and renderer are required".into(),
        ));
    }
    if features.is_empty() || !valid_digest(provenance_digest) || sources.is_empty() {
        return Err(StyleError::Evidence(
            "features, sources, and a provenance SHA-256 are required".into(),
        ));
    }
    if sources.iter().any(|source| {
        source.source.trim().is_empty()
            || source.license.trim().is_empty()
            || !valid_digest(&source.digest)
    }) {
        return Err(StyleError::Evidence(
            "every source needs identity, license, and SHA-256".into(),
        ));
    }
    sources.sort_by(|left, right| left.source.cmp(&right.source));
    let mut ordered = features.to_vec();
    ordered.sort_by(|left, right| left.id.cmp(&right.id));
    let coordinates = ordered
        .iter()
        .flat_map(|feature| feature.exterior.iter())
        .collect::<Vec<_>>();
    if coordinates
        .iter()
        .any(|point| !point[0].is_finite() || !point[1].is_finite())
    {
        return Err(StyleError::Feature("coordinates must be finite".into()));
    }
    let min_x = coordinates
        .iter()
        .map(|point| point[0])
        .fold(f64::INFINITY, f64::min);
    let min_y = coordinates
        .iter()
        .map(|point| point[1])
        .fold(f64::INFINITY, f64::min);
    let max_x = coordinates
        .iter()
        .map(|point| point[0])
        .fold(f64::NEG_INFINITY, f64::max);
    let max_y = coordinates
        .iter()
        .map(|point| point[1])
        .fold(f64::NEG_INFINITY, f64::max);
    if max_x <= min_x || max_y <= min_y {
        return Err(StyleError::Feature("map extent is empty".into()));
    }
    let style_digest = digest_json(style)?;
    let metadata = MapArtifactMetadata {
        sources,
        crs: layout.crs.clone(),
        coordinate_unit: layout.coordinate_unit.clone(),
        value_unit: style.value_unit.clone(),
        style_digest,
        font_digest: style.labels.as_ref().map(|label| label.font_digest.clone()),
        renderer: layout.renderer.clone(),
        provenance_digest: provenance_digest.into(),
    };
    let metadata_json = canonical_json(
        &serde_json::to_value(&metadata)
            .map_err(|error| StyleError::Serialization(error.to_string()))?,
    );
    let metadata_digest = digest_bytes(metadata_json.as_bytes());
    let padding = layout.padding_px as f64;
    let drawable_width = layout.width_px as f64 - padding * 2.0;
    let drawable_height = layout.height_px as f64 - padding * 2.0;
    let project = |point: [f64; 2]| {
        let x = padding + (point[0] - min_x) / (max_x - min_x) * drawable_width;
        let y = padding + (max_y - point[1]) / (max_y - min_y) * drawable_height;
        (x, y)
    };
    let mut body = String::new();
    for feature in &ordered {
        if feature.exterior.len() < 4 || feature.exterior.first() != feature.exterior.last() {
            return Err(StyleError::Feature(format!(
                "feature {} ring is not closed",
                feature.id
            )));
        }
        let fill = style.fill_for(feature.value)?;
        let points = feature
            .exterior
            .iter()
            .map(|point| {
                let (x, y) = project(*point);
                format!("{x:.6},{y:.6}")
            })
            .collect::<Vec<_>>()
            .join(" ");
        body.push_str(&format!(
            "<polygon id=\"{}\" points=\"{}\" fill=\"{}\" stroke=\"#222222\" stroke-width=\"0.5\"/>",
            escape_xml(&feature.id), points, fill
        ));
        if let (Some(label_style), Some(label)) = (&style.labels, &feature.label) {
            let count = feature.exterior.len().saturating_sub(1) as f64;
            let centroid = feature
                .exterior
                .iter()
                .take(count as usize)
                .fold([0.0, 0.0], |sum, point| {
                    [sum[0] + point[0], sum[1] + point[1]]
                });
            let (x, y) = project([centroid[0] / count, centroid[1] / count]);
            body.push_str(&format!(
                "<text x=\"{x:.6}\" y=\"{y:.6}\" text-anchor=\"middle\" font-family=\"{}\" font-size=\"{:.3}\" fill=\"{}\">{}</text>",
                escape_xml(&label_style.font_family), label_style.size_px, label_style.color, escape_xml(label)
            ));
        }
    }
    let mut legend = String::new();
    for (index, class) in style.breaks.iter().enumerate() {
        let y = style.legend.y + index as u32 * 20;
        legend.push_str(&format!(
            "<rect x=\"{}\" y=\"{y}\" width=\"14\" height=\"14\" fill=\"{}\"/><text x=\"{}\" y=\"{}\" font-size=\"12\">{}</text>",
            style.legend.x, class.fill, style.legend.x + 20, y + 12, escape_xml(&class.label)
        ));
    }
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\"><metadata>{}</metadata><title>{}</title>{}<g id=\"legend\"><text x=\"{}\" y=\"{}\" font-size=\"14\">{}</text>{}</g></svg>",
        layout.width_px,
        layout.height_px,
        layout.width_px,
        layout.height_px,
        escape_xml(&metadata_json),
        escape_xml(&layout.title),
        body,
        style.legend.x,
        style.legend.y.saturating_sub(8),
        escape_xml(&style.legend.title),
        legend
    );
    Ok(MapArtifact {
        media_type: "image/svg+xml".into(),
        artifact_digest: digest_bytes(svg.as_bytes()),
        metadata_digest,
        svg,
        metadata,
    })
}

fn valid_hex_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
}

fn valid_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value.as_bytes()[7..].iter().all(u8::is_ascii_hexdigit)
}

fn digest_json(value: &impl Serialize) -> Result<String, StyleError> {
    let value = serde_json::to_value(value)
        .map_err(|error| StyleError::Serialization(error.to_string()))?;
    Ok(digest_bytes(canonical_json(&value).as_bytes()))
}

fn digest_bytes(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut entries = map.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(key, _)| *key);
            let body = entries
                .into_iter()
                .map(|(key, value)| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(key).unwrap(),
                        canonical_json(value)
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{body}}}")
        }
        serde_json::Value::Array(values) => {
            format!(
                "[{}]",
                values
                    .iter()
                    .map(canonical_json)
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        _ => serde_json::to_string(value).unwrap(),
    }
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> EvidenceMapStyle {
        EvidenceMapStyle {
            id: Uuid::nil(),
            layer_id: Uuid::nil(),
            field: "density".into(),
            value_unit: "persons/km²".into(),
            classification: ClassificationMethod::Manual,
            breaks: vec![
                ClassBreak {
                    minimum: 0.0,
                    maximum: 100.0,
                    fill: "#eeeeee".into(),
                    label: "0–100".into(),
                },
                ClassBreak {
                    minimum: 100.0,
                    maximum: 200.0,
                    fill: "#333333".into(),
                    label: "100–200".into(),
                },
            ],
            legend: LegendSpec {
                title: "Density".into(),
                unit: "persons/km²".into(),
                x: 10,
                y: 40,
            },
            labels: Some(LabelStyle {
                field: "name".into(),
                font_family: "Noto Sans".into(),
                font_digest: format!("sha256:{}", "a".repeat(64)),
                size_px: 12.0,
                color: "#111111".into(),
            }),
        }
    }

    #[test]
    fn svg_is_stable_and_binds_all_required_evidence() {
        let layout = MapLayout {
            width_px: 400,
            height_px: 300,
            title: "Demo".into(),
            crs: "EPSG:6675".into(),
            coordinate_unit: "metre".into(),
            renderer: "genegis-svg/0.1.0".into(),
            padding_px: 20,
        };
        let features = [SvgPolygon {
            id: "a".into(),
            exterior: vec![
                [0.0, 0.0],
                [10.0, 0.0],
                [10.0, 10.0],
                [0.0, 10.0],
                [0.0, 0.0],
            ],
            value: 150.0,
            label: Some("A".into()),
        }];
        let sources = vec![ArtifactSourceIdentity {
            source: "fixture://a".into(),
            digest: format!("sha256:{}", "b".repeat(64)),
            license: "CC-BY-4.0".into(),
        }];
        let provenance = format!("sha256:{}", "c".repeat(64));
        let first = render_evidence_svg(&style(), &layout, &features, sources.clone(), &provenance)
            .unwrap();
        let second =
            render_evidence_svg(&style(), &layout, &features, sources, &provenance).unwrap();
        assert_eq!(first, second);
        assert!(first.svg.contains("EPSG:6675"));
        assert!(first.svg.contains("font_digest"));
    }

    #[test]
    fn gaps_bad_font_and_out_of_range_values_fail_closed() {
        let mut invalid = style();
        invalid.breaks[1].minimum = 101.0;
        assert!(invalid.validate().is_err());
        invalid = style();
        invalid.labels.as_mut().unwrap().font_digest = "unknown".into();
        assert!(invalid.validate().is_err());
    }
}
