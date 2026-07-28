use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Who initiated a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandOrigin {
    Ui,
    Ai,
    Cli,
    Plugin,
    System,
}

/// Envelope for every mutating operation in GeneGIS.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandEnvelope {
    pub id: Uuid,
    pub origin: CommandOrigin,
    pub timestamp: DateTime<Utc>,
    pub command: Command,
}

/// Command variants — all UX and AI paths converge here.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    RegisterStacEndpoint {
        endpoint_id: String,
        title: String,
        url: String,
        auth_kind: String,
        auth_env: Option<String>,
        auth_header: Option<String>,
    },
    RemoveStacEndpoint {
        endpoint_id: String,
    },
    SearchFederatedStac {
        endpoint_ids: Vec<String>,
        bbox: Option<[f64; 4]>,
        datetime: Option<String>,
        collections: Vec<String>,
        limit: Option<usize>,
    },
    BindStacAsset {
        stac_item_key: String,
        asset_key: String,
        source_endpoints: Vec<String>,
        href: String,
        media_type: String,
        crs: String,
        units: String,
        license: String,
    },
    ReadRemoteGeoParquet {
        uri: String,
        row_groups: Option<Vec<usize>>,
    },
    AddLayer {
        name: String,
        source_id: Uuid,
    },
    RemoveLayer {
        layer_id: Uuid,
    },
    SetLayerVisibility {
        layer_id: Uuid,
        visible: bool,
    },
    SetViewCamera {
        view_id: Uuid,
        center: [f64; 2],
        zoom: f64,
    },
    RunWorkflow {
        workflow_id: Uuid,
    },
    Undo,
    Redo,
}

impl CommandEnvelope {
    pub fn new(origin: CommandOrigin, command: Command) -> Self {
        Self {
            id: Uuid::new_v4(),
            origin,
            timestamp: Utc::now(),
            command,
        }
    }
}

/// In-memory command bus for undo/redo and audit.
#[derive(Debug, Default)]
pub struct CommandBus {
    history: Vec<CommandEnvelope>,
    cursor: usize,
}

impl CommandBus {
    pub fn push(&mut self, envelope: CommandEnvelope) {
        self.history.truncate(self.cursor);
        self.history.push(envelope);
        self.cursor = self.history.len();
    }

    pub fn history(&self) -> &[CommandEnvelope] {
        &self.history
    }

    pub fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    pub fn can_redo(&self) -> bool {
        self.cursor < self.history.len()
    }
}
