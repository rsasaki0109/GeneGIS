use genegis_crs::{CoordinateUnit, Crs, CrsError, SourceMetadata};
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

    pub fn cog() -> Self {
        Self {
            kind: "cog".into(),
            media_type: "image/tiff".into(),
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
    /// Expected content checksum, when the catalog/provider declares one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
    /// Stable provider revision, release, or fixture version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_version: Option<String>,
    pub tags: Vec<String>,
}

impl DatasetRecord {
    /// Parse and normalize the catalog CRS through the shared CRS contract.
    pub fn parsed_crs(&self) -> Result<Crs, CrsError> {
        Crs::parse(&self.crs)
    }

    /// Return coordinate-axis units derived from the catalog CRS.
    pub fn coordinate_unit(&self) -> CoordinateUnit {
        self.parsed_crs()
            .map(|crs| crs.coordinate_unit())
            .unwrap_or(CoordinateUnit::Unknown)
    }

    /// Convert this catalog record into source metadata for a result receipt.
    pub fn source_metadata(&self) -> SourceMetadata {
        let mut source = SourceMetadata::from_uri(
            self.uri.clone(),
            self.checksum.as_deref(),
            self.source_version.as_deref(),
        );
        source.dataset_id = Some(self.id.clone());
        source.license = Some(self.license.clone());
        source
    }

    pub fn summary_json(&self) -> serde_json::Value {
        let source = self.source_metadata();
        serde_json::json!({
            "id": self.id,
            "title": self.title,
            "format": self.format.kind,
            "media_type": self.format.media_type,
            "crs": self.crs,
            "coordinate_unit": self.coordinate_unit().as_str(),
            "uri": self.uri,
            "license": self.license,
            "source": source,
            "source_version": self.source_version,
            "checksum": source.checksum,
            "expected_checksum": source.expected_checksum,
            "observed_checksum": source.observed_checksum,
            "checksum_status": source.checksum_status,
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
