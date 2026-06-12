use serde::{Deserialize, Serialize};

/// Closed polygon ring in WGS84 lon/lat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolygonRing {
    pub coords: Vec<(f64, f64)>,
}

impl PolygonRing {
    pub fn new(coords: Vec<(f64, f64)>) -> Self {
        Self { coords }
    }

    pub fn exterior(&self) -> &[(f64, f64)] {
        &self.coords
    }
}
