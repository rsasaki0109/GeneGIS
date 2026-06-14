//! Audit bundle export for collab provenance + agent run history.

use std::path::Path;

use genegis_catalog::{
    alpha_catalog, bind_stac_item, browse_alpha_stac_collection, extended_catalog,
    load_catalog_overlay, Catalog, DatasetRecord,
};
use serde_json::Value;

use crate::error::AgentError;
use crate::model::AgentRun;

pub const AUDIT_BUNDLE_SCHEMA: &str = "genegis-audit-bundle-v3";

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

fn stac_items_for_catalog(catalog: &Catalog) -> Result<Vec<Value>, AgentError> {
    catalog
        .list()
        .into_iter()
        .map(|record| {
            bind_stac_item(catalog, &record.id)
                .map_err(|err| AgentError::Message(err.to_string()))
                .and_then(|item| {
                    serde_json::to_value(item).map_err(|err| AgentError::Json(err.to_string()))
                })
        })
        .collect()
}

/// Build STAC alpha, overlay, and merged catalog snapshots for audit export.
pub fn build_audit_stac_snapshot() -> Result<Value, AgentError> {
    let alpha = alpha_catalog();
    let merged = extended_catalog();
    let overlay = load_catalog_overlay();

    let alpha_collection = browse_alpha_stac_collection(&alpha);
    let merged_collection = browse_alpha_stac_collection(&merged);

    Ok(serde_json::json!({
        "alpha": {
            "collection": alpha_collection.summary_json(),
            "items": stac_items_for_catalog(&alpha)?,
        },
        "overlay": {
            "record_count": overlay.len(),
            "records": overlay.iter().map(DatasetRecord::summary_json).collect::<Vec<_>>(),
        },
        "merged": {
            "collection": merged_collection.summary_json(),
            "items": stac_items_for_catalog(&merged)?,
        },
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
            stac.pointer("/alpha/collection/item_count")
                .and_then(Value::as_u64),
            Some(5)
        );
        let alpha_items = stac
            .pointer("/alpha/items")
            .and_then(Value::as_array)
            .expect("alpha items");
        assert_eq!(alpha_items.len(), 5);
        assert!(stac.get("overlay").is_some());
        assert_eq!(
            stac.pointer("/merged/collection/item_count")
                .and_then(Value::as_u64),
            Some(5)
        );

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
            .pointer("/alpha/items")
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
