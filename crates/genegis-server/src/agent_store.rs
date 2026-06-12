use std::path::{Path, PathBuf};
use std::sync::Mutex;

use genegis_agent::{AgentError, AgentRun, AgentRunSummary};
use uuid::Uuid;

/// Directory for persisted agent run traces (one JSON file per run id).
pub const DEFAULT_AGENT_RUNS_DIR: &str = ".genegis/agent-runs";

/// Latest agent run projection for workbench / CLI convenience.
pub const DEFAULT_AGENT_LATEST_PATH: &str = ".genegis/agent-run.json";

/// Thread-safe store for agent orchestration runs.
pub struct AgentRunStore {
    runs_dir: PathBuf,
    latest_path: PathBuf,
    latest: Mutex<Option<AgentRun>>,
}

impl AgentRunStore {
    pub fn load(runs_dir: impl AsRef<Path>, latest_path: impl AsRef<Path>) -> Self {
        let runs_dir = runs_dir.as_ref().to_path_buf();
        let latest_path = latest_path.as_ref().to_path_buf();
        let latest = load_latest(&latest_path, &runs_dir);
        Self {
            runs_dir,
            latest_path,
            latest: Mutex::new(latest),
        }
    }

    pub fn runs_dir(&self) -> &Path {
        &self.runs_dir
    }

    pub fn latest_path(&self) -> &Path {
        &self.latest_path
    }

    pub fn latest(&self) -> Result<Option<AgentRun>, AgentError> {
        let latest = self
            .latest
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(latest.clone())
    }

    pub fn list(&self) -> Result<Vec<AgentRunSummary>, AgentError> {
        AgentRun::list_from_dir(&self.runs_dir)
    }

    pub fn get(&self, id: Uuid) -> Result<Option<AgentRun>, AgentError> {
        let path = self.runs_dir.join(format!("{id}.json"));
        if !path.is_file() {
            return Ok(None);
        }
        AgentRun::load_from_path(path).map(Some)
    }

    pub fn save(&self, run: &AgentRun) -> Result<AgentRun, AgentError> {
        std::fs::create_dir_all(&self.runs_dir)
            .map_err(|err| AgentError::Json(err.to_string()))?;

        let run_path = self.runs_dir.join(format!("{}.json", run.id));
        run.save_to_path(&run_path)?;
        run.save_to_path(&self.latest_path)?;

        let mut latest = self
            .latest
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *latest = Some(run.clone());
        Ok(run.clone())
    }
}

fn load_latest(latest_path: &Path, runs_dir: &Path) -> Option<AgentRun> {
    if latest_path.is_file() {
        if let Ok(run) = AgentRun::load_from_path(latest_path) {
            return Some(run);
        }
    }

    AgentRun::list_from_dir(runs_dir)
        .ok()
        .and_then(|summaries| summaries.first().map(|summary| summary.id))
        .and_then(|id| AgentRun::load_from_runs_dir(runs_dir, id).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use genegis_agent::{AgentOrchestrator, AgentRunConfig};

    #[test]
    fn saves_lists_and_gets_runs() {
        let temp = tempfile::tempdir().expect("tempdir");
        let runs_dir = temp.path().join("agent-runs");
        let latest_path = temp.path().join("agent-run.json");
        let store = AgentRunStore::load(&runs_dir, &latest_path);

        let run = AgentOrchestrator::new()
            .with_config(AgentRunConfig::rule_based_offline().plan_only())
            .run("名古屋市の人口密度を表示")
            .expect("run");

        store.save(&run).expect("save");
        assert_eq!(store.list().expect("list").len(), 1);
        assert_eq!(
            store.get(run.id).expect("get").expect("run").id,
            run.id
        );
    }
}
