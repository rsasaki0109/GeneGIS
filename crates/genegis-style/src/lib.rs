//! Map style model — choropleth ramps and legends.

pub mod choropleth;
pub mod evidence;

pub use choropleth::{ChoroplethStyle, ColorRgba, LegendItem};
pub use evidence::{
    render_evidence_svg, ArtifactSourceIdentity, ClassBreak, ClassificationMethod,
    EvidenceMapStyle, LabelStyle, LegendSpec, MapArtifact, MapArtifactMetadata, MapLayout,
    StyleError, SvgPolygon,
};
