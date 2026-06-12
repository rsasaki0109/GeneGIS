use genegis_geometry::{BoundingBox, PolygonRing};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureRecord {
    pub id: usize,
    pub properties: Value,
    pub rings: Vec<PolygonRing>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorDataset {
    pub name: String,
    pub crs: String,
    pub features: Vec<FeatureRecord>,
    pub bbox: BoundingBox,
}

impl VectorDataset {
    pub fn feature_count(&self) -> usize {
        self.features.len()
    }
}

impl FeatureRecord {
    pub fn exterior_rings(&self) -> impl Iterator<Item = &[(f64, f64)]> {
        self.rings.iter().map(|ring| ring.exterior())
    }
}
