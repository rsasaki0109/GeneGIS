use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{DataSource, Layer, LayerId, View, ViewId};

/// Top-level workspace: projects, data connections, cache, history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub sources: Vec<DataSource>,
    pub layers: Vec<Layer>,
    pub views: Vec<View>,
    pub active_view: Option<ViewId>,
}

impl Workspace {
    pub fn new(name: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            created_at: now,
            updated_at: now,
            sources: Vec::new(),
            layers: Vec::new(),
            views: Vec::new(),
            active_view: None,
        }
    }

    pub fn add_source(&mut self, source: DataSource) {
        self.sources.push(source);
        self.touch();
    }

    pub fn add_layer(&mut self, layer: Layer) -> LayerId {
        let id = layer.id;
        self.layers.push(layer);
        self.touch();
        id
    }

    pub fn add_view(&mut self, view: View) -> ViewId {
        let id = view.id;
        if self.active_view.is_none() {
            self.active_view = Some(id);
        }
        self.views.push(view);
        self.touch();
        id
    }

    fn touch(&mut self) {
        self.updated_at = Utc::now();
    }
}
