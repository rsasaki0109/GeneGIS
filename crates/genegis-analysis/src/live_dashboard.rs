//! Digest-bound live dashboard widgets for verified spatial results.

use std::collections::BTreeMap;

use genegis_core::WorkflowDigest;
use genegis_crs::SourceSnapshot;
use genegis_render::Scene3d;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistogramBin {
    pub label: String,
    pub minimum_inclusive: f64,
    pub maximum_exclusive: Option<f64>,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CategoryCount {
    pub category: String,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DashboardWidget {
    Kpi {
        id: String,
        label: String,
        value: f64,
        unit: String,
    },
    Histogram {
        id: String,
        label: String,
        unit: String,
        bins: Vec<HistogramBin>,
    },
    CategoryBreakdown {
        id: String,
        label: String,
        categories: Vec<CategoryCount>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveDashboard {
    pub schema_version: String,
    pub workflow_digest: WorkflowDigest,
    pub result_digest: String,
    pub source_snapshots: Vec<SourceSnapshot>,
    pub widgets: Vec<DashboardWidget>,
    pub dashboard_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LiveDashboardError {
    #[error("dashboard requires a non-empty workflow digest")]
    WorkflowDigest,
    #[error("dashboard result digest does not match the verified scene")]
    ResultDigest,
    #[error("dashboard source snapshots do not match the verified scene")]
    SourceSnapshots,
    #[error("dashboard widgets do not match the verified scene facts")]
    Widgets,
    #[error("dashboard digest mismatch")]
    DashboardDigest,
    #[error("dashboard serialization failed: {0}")]
    Serialization(String),
}

pub fn canonical_scene_result_digest(scene: &Scene3d) -> Result<String, LiveDashboardError> {
    scene
        .validate()
        .map_err(|error| LiveDashboardError::Serialization(error.to_string()))?;
    let bytes = serde_json::to_vec(scene)
        .map_err(|error| LiveDashboardError::Serialization(error.to_string()))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

pub fn build_scene3d_dashboard(
    scene: &Scene3d,
    workflow_digest: WorkflowDigest,
    result_digest: impl Into<String>,
) -> Result<LiveDashboard, LiveDashboardError> {
    if workflow_digest.is_empty() {
        return Err(LiveDashboardError::WorkflowDigest);
    }
    let result_digest = result_digest.into();
    if canonical_scene_result_digest(scene)? != result_digest {
        return Err(LiveDashboardError::ResultDigest);
    }
    let source_snapshots = scene
        .sources
        .iter()
        .map(|source| source.snapshot.clone())
        .collect();
    let widgets = scene_widgets(scene);
    let mut dashboard = LiveDashboard {
        schema_version: "0.1.0".into(),
        workflow_digest,
        result_digest,
        source_snapshots,
        widgets,
        dashboard_digest: String::new(),
    };
    dashboard.dashboard_digest = dashboard_digest(&dashboard)?;
    Ok(dashboard)
}

impl LiveDashboard {
    pub fn verify(&self, scene: &Scene3d) -> Result<(), LiveDashboardError> {
        if self.workflow_digest.is_empty() {
            return Err(LiveDashboardError::WorkflowDigest);
        }
        if canonical_scene_result_digest(scene)? != self.result_digest {
            return Err(LiveDashboardError::ResultDigest);
        }
        let expected_sources: Vec<_> = scene
            .sources
            .iter()
            .map(|source| source.snapshot.clone())
            .collect();
        if self.source_snapshots != expected_sources {
            return Err(LiveDashboardError::SourceSnapshots);
        }
        if self.widgets != scene_widgets(scene) {
            return Err(LiveDashboardError::Widgets);
        }
        if self.dashboard_digest != dashboard_digest(self)? {
            return Err(LiveDashboardError::DashboardDigest);
        }
        Ok(())
    }
}

fn scene_widgets(scene: &Scene3d) -> Vec<DashboardWidget> {
    let heights: Vec<_> = scene
        .buildings
        .iter()
        .map(|building| building.height)
        .collect();
    let mut bins = vec![
        HistogramBin {
            label: "0–4 m".into(),
            minimum_inclusive: 0.0,
            maximum_exclusive: Some(4.0),
            count: 0,
        },
        HistogramBin {
            label: "4–6 m".into(),
            minimum_inclusive: 4.0,
            maximum_exclusive: Some(6.0),
            count: 0,
        },
        HistogramBin {
            label: "6–9 m".into(),
            minimum_inclusive: 6.0,
            maximum_exclusive: Some(9.0),
            count: 0,
        },
        HistogramBin {
            label: "9–15 m".into(),
            minimum_inclusive: 9.0,
            maximum_exclusive: Some(15.0),
            count: 0,
        },
        HistogramBin {
            label: "15 m+".into(),
            minimum_inclusive: 15.0,
            maximum_exclusive: None,
            count: 0,
        },
    ];
    for height in &heights {
        if let Some(bin) = bins.iter_mut().find(|bin| {
            *height >= bin.minimum_inclusive
                && bin
                    .maximum_exclusive
                    .is_none_or(|maximum| *height < maximum)
        }) {
            bin.count += 1;
        }
    }
    let mut categories = BTreeMap::new();
    for poi in &scene.pois {
        *categories.entry(poi.category.clone()).or_insert(0_u64) += 1;
    }
    let categories = categories
        .into_iter()
        .map(|(category, count)| CategoryCount { category, count })
        .collect();
    vec![
        DashboardWidget::Kpi {
            id: "building-count".into(),
            label: "Buildings".into(),
            value: scene.buildings.len() as f64,
            unit: "count".into(),
        },
        DashboardWidget::Kpi {
            id: "maximum-building-height".into(),
            label: "Maximum height".into(),
            value: heights.into_iter().fold(0.0_f64, f64::max),
            unit: "metres".into(),
        },
        DashboardWidget::Kpi {
            id: "poi-count".into(),
            label: "POIs".into(),
            value: scene.pois.len() as f64,
            unit: "count".into(),
        },
        DashboardWidget::Histogram {
            id: "building-height-histogram".into(),
            label: "Building height".into(),
            unit: "metres".into(),
            bins,
        },
        DashboardWidget::CategoryBreakdown {
            id: "poi-category-breakdown".into(),
            label: "POI categories".into(),
            categories,
        },
    ]
}

fn dashboard_digest(dashboard: &LiveDashboard) -> Result<String, LiveDashboardError> {
    let value = serde_json::json!({
        "schema_version": dashboard.schema_version,
        "workflow_digest": dashboard.workflow_digest,
        "result_digest": dashboard.result_digest,
        "source_snapshots": dashboard.source_snapshots,
        "widgets": dashboard.widgets,
    });
    let canonical = canonical_json(&value);
    Ok(format!("sha256:{:x}", Sha256::digest(canonical.as_bytes())))
}

fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by_key(|(key, _)| *key);
            format!(
                "{{{}}}",
                entries
                    .into_iter()
                    .map(|(key, value)| format!(
                        "{}:{}",
                        serde_json::to_string(key).expect("JSON key"),
                        canonical_json(value)
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        serde_json::Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        _ => serde_json::to_string(value).expect("JSON scalar"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use genegis_crs::{CoordinateUnit, Crs};
    use genegis_render::{BuildingLod1, OrbitCamera, ScenePoi, SceneSource};

    fn scene() -> Scene3d {
        let source = SourceSnapshot::new("file:///verified-scene.json");
        Scene3d {
            schema_version: "0.1.0".into(),
            crs: Crs::nagoya_projected(),
            coordinate_unit: CoordinateUnit::Metres,
            vertical_unit: "metres".into(),
            sources: vec![SceneSource {
                id: "verified".into(),
                role: "scene_facts".into(),
                snapshot: source,
            }],
            point_source_id: "verified".into(),
            points: vec![[0.0, 0.0, 0.0], [20.0, 20.0, 2.0]],
            buildings: [3.5, 7.0, 12.0, 18.0]
                .into_iter()
                .enumerate()
                .map(|(index, height)| BuildingLod1 {
                    id: format!("b-{index}"),
                    footprint: vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0]],
                    base_z: 0.0,
                    height,
                    height_source_id: "verified".into(),
                })
                .collect(),
            pois: vec![
                ScenePoi {
                    id: "p-1".into(),
                    position: [1.0, 1.0, 0.0],
                    category: "school".into(),
                    source_id: "verified".into(),
                },
                ScenePoi {
                    id: "p-2".into(),
                    position: [2.0, 2.0, 0.0],
                    category: "school".into(),
                    source_id: "verified".into(),
                },
                ScenePoi {
                    id: "p-3".into(),
                    position: [3.0, 3.0, 0.0],
                    category: "park".into(),
                    source_id: "verified".into(),
                },
            ],
            camera: OrbitCamera {
                target: [10.0, 10.0, 0.0],
                yaw_degrees: 30.0,
                pitch_degrees: 35.0,
                radius: 30.0,
            },
        }
    }

    #[test]
    fn creates_and_verifies_all_required_widget_types() {
        let scene = scene();
        let result_digest = canonical_scene_result_digest(&scene).expect("result digest");
        let dashboard = build_scene3d_dashboard(
            &scene,
            WorkflowDigest::new("sha256:workflow"),
            result_digest.clone(),
        )
        .expect("dashboard");
        dashboard.verify(&scene).expect("verified dashboard");
        assert_eq!(dashboard.widgets.len(), 5);
        assert!(dashboard
            .widgets
            .iter()
            .any(|widget| matches!(widget, DashboardWidget::Histogram { .. })));
        assert!(dashboard
            .widgets
            .iter()
            .any(|widget| matches!(widget, DashboardWidget::CategoryBreakdown { .. })));
    }

    #[test]
    fn rejects_tampered_result_widget_and_source() {
        let scene = scene();
        let result_digest = canonical_scene_result_digest(&scene).expect("result digest");
        assert_eq!(
            build_scene3d_dashboard(
                &scene,
                WorkflowDigest::new("sha256:workflow"),
                "sha256:tampered",
            ),
            Err(LiveDashboardError::ResultDigest)
        );
        let mut dashboard = build_scene3d_dashboard(
            &scene,
            WorkflowDigest::new("sha256:workflow"),
            result_digest.clone(),
        )
        .expect("dashboard");
        dashboard.widgets.pop();
        assert_eq!(dashboard.verify(&scene), Err(LiveDashboardError::Widgets));

        let mut dashboard = build_scene3d_dashboard(
            &scene,
            WorkflowDigest::new("sha256:workflow"),
            canonical_scene_result_digest(&scene).expect("digest"),
        )
        .expect("dashboard");
        dashboard.source_snapshots[0].uri = "file:///tampered".into();
        assert_eq!(
            dashboard.verify(&scene),
            Err(LiveDashboardError::SourceSnapshots)
        );

        let mut dashboard = build_scene3d_dashboard(
            &scene,
            WorkflowDigest::new("sha256:workflow"),
            result_digest,
        )
        .expect("dashboard");
        dashboard.dashboard_digest = "sha256:tampered".into();
        assert_eq!(
            dashboard.verify(&scene),
            Err(LiveDashboardError::DashboardDigest)
        );
    }
}
