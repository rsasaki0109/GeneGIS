use serde::{Deserialize, Serialize};

/// One polygon part in WGS84 lon/lat.
///
/// `coords` is the exterior ring.  Interior rings are retained in `holes`
/// instead of being discarded by a format adapter; area operations subtract
/// them and renderers can choose an appropriate even-odd/triangulation path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolygonRing {
    pub coords: Vec<(f64, f64)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holes: Vec<Vec<(f64, f64)>>,
}

impl PolygonRing {
    pub fn new(coords: Vec<(f64, f64)>) -> Self {
        Self {
            coords,
            holes: Vec::new(),
        }
    }

    /// Construct a polygon part with its interior rings intact.
    pub fn with_holes(coords: Vec<(f64, f64)>, holes: Vec<Vec<(f64, f64)>>) -> Self {
        Self { coords, holes }
    }

    pub fn exterior(&self) -> &[(f64, f64)] {
        &self.coords
    }

    pub fn holes(&self) -> &[Vec<(f64, f64)>] {
        &self.holes
    }
}
