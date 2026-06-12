use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::CollabError;

/// Map- or layer-anchored review comment (Figma-style thread seed).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapComment {
    pub id: Uuid,
    pub thread_id: Uuid,
    pub author: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer_id: Option<uuid::Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map_anchor: Option<[f64; 2]>,
}

impl MapComment {
    /// Create a comment on the shared map canvas.
    pub fn new(author: impl Into<String>, body: impl Into<String>) -> Self {
        let id = Uuid::new_v4();
        Self {
            id,
            thread_id: id,
            author: author.into(),
            body: body.into(),
            created_at: Utc::now(),
            layer_id: None,
            map_anchor: None,
        }
    }

    /// Attach WGS84 map coordinates to the comment.
    pub fn with_map_anchor(mut self, lon: f64, lat: f64) -> Self {
        self.map_anchor = Some([lon, lat]);
        self
    }

    /// Attach a layer id from the project workspace.
    pub fn with_layer_id(mut self, layer_id: Uuid) -> Self {
        self.layer_id = Some(layer_id);
        self
    }

    pub(crate) fn validate(&self) -> Result<(), CollabError> {
        if self.author.trim().is_empty() {
            return Err(CollabError::InvalidComment(
                "author must not be empty".into(),
            ));
        }
        if self.body.trim().is_empty() {
            return Err(CollabError::InvalidComment("body must not be empty".into()));
        }
        Ok(())
    }
}
