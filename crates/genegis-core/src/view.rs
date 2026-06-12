use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for a view (map, scene, dashboard, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ViewId(pub Uuid);

impl ViewId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ViewId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewKind {
    Map,
    Scene,
    Dashboard,
    Notebook,
    Report,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct View {
    pub id: ViewId,
    pub name: String,
    pub kind: ViewKind,
    pub layer_ids: Vec<super::LayerId>,
    pub crs: Option<String>,
    pub center: Option<[f64; 2]>,
    pub zoom: Option<f64>,
}

impl View {
    pub fn new(name: impl Into<String>, kind: ViewKind) -> Self {
        Self {
            id: ViewId::new(),
            name: name.into(),
            kind,
            layer_ids: Vec::new(),
            crs: None,
            center: None,
            zoom: None,
        }
    }
}
