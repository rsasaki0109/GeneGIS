use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Kind of data source backing a layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    File,
    Directory,
    ObjectStore,
    HttpRange,
    Database,
    Stac,
    OgcApi,
    Rest,
    SensorStream,
    Generated,
}

/// Reference to where spatial data lives.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataSource {
    pub id: Uuid,
    pub name: String,
    pub kind: SourceKind,
    pub uri: String,
    pub format_hint: Option<String>,
    pub crs: Option<String>,
    pub license: Option<String>,
    pub source_url: Option<String>,
}

impl DataSource {
    pub fn new(name: impl Into<String>, kind: SourceKind, uri: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            kind,
            uri: uri.into(),
            format_hint: None,
            crs: None,
            license: None,
            source_url: None,
        }
    }
}
