use genegis_geometry::PolygonRing;
use genegis_style::{ChoroplethStyle, ColorRgba};
use genegis_workflow::GeoWorkflow;
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReport {
    pub crs: String,
    pub area_method: String,
    pub density_unit: String,
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
