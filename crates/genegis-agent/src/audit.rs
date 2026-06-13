//! Audit bundle export for collab provenance + agent run history.

use std::path::Path;

use genegis_catalog::{alpha_catalog, bind_stac_item, browse_alpha_stac_collection};
use serde_json::Value;

use crate::error::AgentError;
use crate::model::AgentRun;

pub const AUDIT_BUNDLE_SCHEMA: &str = "genegis-audit-bundle-v2";

/// Collab-side fields included in an audit bundle.
#[derive(Debug, Clone)]
pub struct AuditCollabSnapshot {
    pub summary: Value,
    pub comments: Value,
    pub provenance: Value,
}

impl AuditCollabSnapshot {
    pub fn empty() -> Self {
        Self {
            summary: Value::Object(Default::default()),
            comments: Value::Array(Vec::new()),
            provenance: Value::Array(Vec::new()),
        }
    }
}

/// Build STAC collection + item snapshot from the alpha catalog.
pub fn build_audit_stac_snapshot() -> Result<Value, AgentError> {
    let catalog = alpha_catalog();
    let collection = browse_alpha_stac_collection(&catalog);
    let mut items = Vec::new();

    for record in catalog.list() {
        let item = bind_stac_item(&catalog, &record.id)
            .map_err(|err| AgentError::Message(err.to_string()))?;
        items.push(
            serde_json::to_value(item).map_err(|err| AgentError::Json(err.to_string()))?,
        );
    }

    Ok(serde_json::json!({
        "collection": collection.summary_json(),
        "items": items,
    }))
}

/// Build the audit bundle JSON consumed by `genegis agent export-audit`.
pub fn build_audit_bundle(
    collab: &AuditCollabSnapshot,
    runs_dir: impl AsRef<Path>,
    latest_path: impl AsRef<Path>,
) -> Result<Value, AgentError> {
    let runs = AgentRun::list_from_dir(runs_dir)?;
    let latest = AgentRun::load_from_path(latest_path).ok();
    let stac = build_audit_stac_snapshot()?;

    Ok(serde_json::json!({
        "schema": AUDIT_BUNDLE_SCHEMA,
        "collab_summary": collab.summary,
        "comments": collab.comments,
        "provenance": collab.provenance,
        "stac": stac,
        "agent_runs": runs,
        "latest_agent_run_id": latest.as_ref().map(|run| run.id),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentOrchestrator, AgentRunConfig};
    use genegis_catalog::LOCAL_COG_DEMO_ID;

    #[test]
    fn audit_bundle_has_stable_schema_and_run_index() {
        let temp = tempfile::tempdir().expect("tempdir");
        let runs_dir = temp.path().join("agent-runs");
        let latest_path = temp.path().join("agent-run.json");

        let run = AgentOrchestrator::new()
            .with_config(AgentRunConfig::rule_based_offline())
            .run("名古屋市の人口密度を表示")
            .expect("run");
        run.save_to_path(&runs_dir.join(format!("{}.json", run.id)))
            .expect("save run");
        run.save_to_path(&latest_path).expect("save latest");

        let bundle = build_audit_bundle(
            &AuditCollabSnapshot::empty(),
            &runs_dir,
            &latest_path,
        )
        .expect("bundle");

        assert_eq!(
            bundle.get("schema").and_then(Value::as_str),
            Some(AUDIT_BUNDLE_SCHEMA)
        );
        assert!(bundle.get("collab_summary").is_some());
        assert!(bundle.get("stac").is_some());

        let stac = bundle.get("stac").expect("stac");
        assert_eq!(
            stac.pointer("/collection/item_count").and_then(Value::as_u64),
            Some(3)
        );
        let items = stac
            .get("items")
            .and_then(Value::as_array)
            .expect("stac items");
        assert_eq!(items.len(), 3);

        let runs = bundle
            .get("agent_runs")
            .and_then(Value::as_array)
            .expect("agent_runs array");
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0]
                .get("verification_passed")
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn audit_stac_snapshot_includes_local_cog_item() {
        let stac = build_audit_stac_snapshot().expect("stac");
        let items = stac
            .get("items")
            .and_then(Value::as_array)
            .expect("items");
        assert!(
            items
                .iter()
                .any(|item| item.get("id").and_then(Value::as_str) == Some(LOCAL_COG_DEMO_ID))
        );
    }

    #[test]
    fn audit_bundle_defaults_latest_path() {
        let bundle = build_audit_bundle(
            &AuditCollabSnapshot::empty(),
            ".genegis/missing-runs",
            crate::model::DEFAULT_AGENT_RUN_PATH,
        )
        .expect("bundle");

        assert_eq!(
            bundle.get("schema").and_then(Value::as_str),
            Some(AUDIT_BUNDLE_SCHEMA)
        );
        assert!(bundle.get("stac").is_some());
        assert_eq!(
            bundle.get("agent_runs").and_then(Value::as_array).map(Vec::len),
            Some(0)
        );
    }
}
