use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for a layer in the project graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LayerId(pub Uuid);

impl LayerId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for LayerId {
    fn default() -> Self {
        Self::new()
    }
}

/// Layer type in the semantic model (not format-specific).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerKind {
    Vector,
    Raster,
    Tile,
    PointCloud,
    Scene3d,
    Table,
}

/// Optional cached statistics for a layer.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LayerStatistics {
    pub feature_count: Option<u64>,
    pub bbox: Option<[f64; 4]>,
}

/// A layer in the project graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    pub kind: LayerKind,
    pub source_id: uuid::Uuid,
    pub crs: Option<String>,
    pub extent: Option<[f64; 4]>,
    pub time_extent: Option<(DateTime<Utc>, DateTime<Utc>)>,
    pub statistics: LayerStatistics,
    pub style_id: Option<uuid::Uuid>,
    pub visible: bool,
    pub opacity: f32,
}

impl Layer {
    pub fn new(name: impl Into<String>, kind: LayerKind, source_id: uuid::Uuid) -> Self {
        Self {
            id: LayerId::new(),
            name: name.into(),
            kind,
            source_id,
            crs: None,
            extent: None,
            time_extent: None,
            statistics: LayerStatistics::default(),
            style_id: None,
            visible: true,
            opacity: 1.0,
        }
    }
}
