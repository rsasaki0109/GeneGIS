use genegis_geometry::BoundingBox;
use serde::{Deserialize, Serialize};

/// Cloud-native or legacy vector format descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetFormat {
    pub kind: String,
    pub media_type: String,
}

impl DatasetFormat {
    pub fn geojson() -> Self {
        Self {
            kind: "geojson".into(),
            media_type: "application/geo+json".into(),
        }
    }

    pub fn geoparquet() -> Self {
        Self {
            kind: "geoparquet".into(),
            media_type: "application/vnd.apache.parquet".into(),
        }
    }
}

/// Catalog entry describing a discoverable dataset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatasetRecord {
    pub id: String,
    pub title: String,
    pub description: String,
    pub format: DatasetFormat,
    pub crs: String,
    pub bbox: BoundingBox,
    /// Local path or cloud URI for the primary asset.
    pub uri: String,
    pub license: String,
    pub tags: Vec<String>,
}

impl DatasetRecord {
    pub fn summary_json(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "title": self.title,
            "format": self.format.kind,
            "media_type": self.format.media_type,
            "crs": self.crs,
            "uri": self.uri,
            "license": self.license,
            "tags": self.tags,
            "bbox": {
                "min_x": self.bbox.min.x,
                "min_y": self.bbox.min.y,
                "max_x": self.bbox.max.x,
                "max_y": self.bbox.max.y,
            },
        })
    }
}
