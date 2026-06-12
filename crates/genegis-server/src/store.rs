use std::path::{Path, PathBuf};
use std::sync::Mutex;

use genegis_collab::{CollabError, CollabSession};

/// Default on-disk path for the shared collab session.
pub const DEFAULT_COLLAB_PATH: &str = ".genegis/collab.json";

/// Thread-safe collab session store with optional JSON persistence.
pub struct CollabStore {
    session: Mutex<CollabSession>,
    path: PathBuf,
}

impl CollabStore {
    /// Load from disk when present, otherwise seed the Nagoya demo session.
    pub fn load(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        let session = if path.is_file() {
            std::fs::read_to_string(&path)
                .ok()
                .and_then(|json| CollabSession::import_json(&json).ok())
                .unwrap_or_else(CollabSession::demo_nagoya)
        } else {
            CollabSession::demo_nagoya()
        };

        Self {
            session: Mutex::new(session),
            path,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn snapshot(&self) -> Result<CollabSession, CollabError> {
        let session = self
            .session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        CollabSession::import_json(&session.export_json()?)
    }

    pub fn replace_json(&self, json: &str) -> Result<CollabSession, CollabError> {
        let imported = CollabSession::import_json(json)?;
        let mut session = self
            .session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *session = imported;
        self.persist(&session)?;
        Ok(CollabSession::import_json(&session.export_json()?)?)
    }

    fn persist(&self, session: &CollabSession) -> Result<(), CollabError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| CollabError::Json(err.to_string()))?;
        }
        let json = session.export_json()?;
        std::fs::write(&self.path, json).map_err(|err| CollabError::Json(err.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_session_json() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("collab.json");
        let store = CollabStore::load(&path);
        let updated = store.replace_json(&CollabSession::demo_nagoya().export_json().expect("json"));
        assert!(updated.is_ok());
        assert!(path.is_file());
    }
}
