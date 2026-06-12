use std::path::{Path, PathBuf};
use std::sync::Mutex;

use genegis_collab::{CollabError, CollabSession, CollabUpload};

/// Default on-disk path for the shared collab session JSON projection.
pub const DEFAULT_COLLAB_PATH: &str = ".genegis/collab.json";

/// Default Automerge snapshot path paired with [`DEFAULT_COLLAB_PATH`].
pub const DEFAULT_COLLAB_SNAPSHOT_SUFFIX: &str = ".automerge";

/// Thread-safe collab session store with JSON projection + Automerge snapshot.
pub struct CollabStore {
    session: Mutex<CollabSession>,
    json_path: PathBuf,
    snapshot_path: PathBuf,
}

impl CollabStore {
    /// Load from Automerge snapshot or JSON when present, otherwise seed the Nagoya demo session.
    pub fn load(path: impl AsRef<Path>) -> Self {
        let json_path = path.as_ref().to_path_buf();
        let snapshot_path = snapshot_path_for(&json_path);
        let session = load_session(&json_path, &snapshot_path);
        let store = Self {
            session: Mutex::new(session),
            json_path,
            snapshot_path,
        };
        if let Ok(mut session) = store.session.lock() {
            let _ = store.persist_locked(&mut session);
        }
        store
    }

    pub fn json_path(&self) -> &Path {
        &self.json_path
    }

    pub fn snapshot_path(&self) -> &Path {
        &self.snapshot_path
    }

    /// Backward-compatible alias for the JSON path.
    pub fn path(&self) -> &Path {
        &self.json_path
    }

    pub fn snapshot(&self) -> Result<CollabSession, CollabError> {
        let session = self
            .session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(session.clone())
    }

    pub fn merge_upload(&self, upload: &CollabUpload) -> Result<CollabSession, CollabError> {
        let mut session = self
            .session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(snapshot) = upload.automerge_snapshot.as_deref() {
            session.merge_snapshot_base64(snapshot)?;
        }
        session.merge_json(&upload.session)?;
        self.persist_locked(&mut session)?;
        Ok(session.clone())
    }

    pub fn replace_json(&self, json: &str) -> Result<CollabSession, CollabError> {
        self.merge_upload(&CollabUpload {
            session: json.to_string(),
            automerge_snapshot: None,
        })
    }

    fn persist_locked(&self, session: &mut CollabSession) -> Result<(), CollabError> {
        if let Some(parent) = self.json_path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| CollabError::Json(err.to_string()))?;
        }

        let json = session.export_json()?;
        std::fs::write(&self.json_path, json).map_err(|err| CollabError::Json(err.to_string()))?;

        let snapshot = session.snapshot_bytes();
        std::fs::write(&self.snapshot_path, snapshot)
            .map_err(|err| CollabError::Automerge(err.to_string()))?;
        Ok(())
    }
}

fn snapshot_path_for(json_path: &Path) -> PathBuf {
    let ext = json_path
        .extension()
        .map(|ext| ext.to_string_lossy().into_owned())
        .unwrap_or_else(|| "json".into());
    json_path.with_extension(format!("{ext}{DEFAULT_COLLAB_SNAPSHOT_SUFFIX}"))
}

fn load_session(json_path: &Path, snapshot_path: &Path) -> CollabSession {
    if snapshot_path.is_file() {
        if let Ok(bytes) = std::fs::read(snapshot_path) {
            if let Ok(session) = CollabSession::from_snapshot(&bytes) {
                return session;
            }
        }
    }

    if json_path.is_file() {
        if let Ok(json) = std::fs::read_to_string(json_path) {
            if let Ok(session) = CollabSession::import_json(&json) {
                return session;
            }
        }
    }

    CollabSession::demo_nagoya()
}

#[cfg(test)]
mod tests {
    use super::*;
    use genegis_collab::MapComment;

    #[test]
    fn merges_uploaded_comments() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("collab.json");
        let store = CollabStore::load(&path);

        let mut client = CollabSession::demo_nagoya();
        client
            .add_comment(MapComment::new("cli", "Server merge smoke"))
            .expect("comment");

        let merged = store
            .merge_upload(&CollabUpload {
                session: client.export_json().expect("json"),
                automerge_snapshot: None,
            })
            .expect("merge");

        assert!(merged.comments().expect("comments").len() >= 2);
        assert!(path.with_extension("json.automerge").is_file());
    }
}
