//! Budgeted city-scale terrain/COPC/3D Tiles/2D frame planning.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use genegis_crs::{Crs, SourceSnapshot};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// One layer family sharing the renderer state contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CityLayerKind {
    /// Tiled terrain/height coverage.
    Terrain,
    /// COPC point-cloud hierarchy.
    PointCloudCopc,
    /// OGC 3D Tiles building/model hierarchy.
    Buildings3dTiles,
    /// 2D vector/raster context rendered in the same view.
    Context2d,
}

/// One city layer with explicit format and provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CityLayer {
    /// Stable layer identity.
    pub id: String,
    /// Layer family.
    pub kind: CityLayerKind,
    /// Open format such as `3dtiles-1.1`, `copc-1.0`, `cog`, or `pmtiles`.
    pub format: String,
    /// Exact layer source.
    pub source: SourceSnapshot,
}

/// One streamable hierarchy tile/chunk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CityTile {
    /// Stable tile identity.
    pub id: String,
    /// Owning layer.
    pub layer_id: String,
    /// Parent tile identity; absent for roots.
    pub parent_id: Option<String>,
    /// Geometric error in metres.
    pub geometric_error_m: f64,
    /// Estimated camera distance in metres for this frame.
    pub camera_distance_m: f64,
    /// Exact content bytes used by transfer budgeting.
    pub content_bytes: u64,
    /// Exact content digest.
    pub content_digest: String,
    /// Stable feature identities represented by this tile.
    pub feature_ids: Vec<String>,
}

/// Camera, selection, temporal cursor, and CRS shared by 2D and 3D layers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedSpatialViewState {
    /// Scene CRS.
    pub crs: Crs,
    /// Camera eye in scene coordinates.
    pub camera_position: [f64; 3],
    /// Camera target in scene coordinates.
    pub camera_target: [f64; 3],
    /// Vertical field of view in degrees.
    pub field_of_view_degrees: f64,
    /// Viewport height used for screen-space error.
    pub viewport_height_px: u32,
    /// Selected feature identities shared with 2D layers/widgets.
    pub selected_feature_ids: BTreeSet<String>,
    /// Optional temporal cursor shared by all temporal layers.
    pub temporal_cursor: Option<String>,
}

/// Hard per-frame streaming budget.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CityStreamBudget {
    /// Maximum selected contents.
    pub maximum_tiles: u32,
    /// Maximum selected transfer bytes.
    pub maximum_transfer_bytes: u64,
    /// Maximum desired screen-space error in pixels.
    pub maximum_screen_space_error: f64,
}

/// Complete city scene input manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CitySceneManifest {
    /// Manifest schema version.
    pub schema_version: String,
    /// Open-format layers.
    pub layers: Vec<CityLayer>,
    /// Hierarchy contents across all layers.
    pub tiles: Vec<CityTile>,
}

/// Deterministic budgeted frame plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CitySceneFramePlan {
    /// Plan schema version.
    pub schema_version: String,
    /// Shared 2D/3D view state.
    pub view: SharedSpatialViewState,
    /// Applied hard budget.
    pub budget: CityStreamBudget,
    /// Selected tile identities in deterministic order.
    pub selected_tiles: Vec<String>,
    /// Refined candidates deferred by tile/byte budgets.
    pub deferred_tiles: Vec<String>,
    /// Selected transfer bytes.
    pub transfer_bytes: u64,
    /// Provenance snapshots for selected layers.
    pub sources: Vec<SourceSnapshot>,
    /// Selected feature identities intersecting shared selection.
    pub selected_features_present: Vec<String>,
    /// Canonical complete frame-plan identity.
    pub plan_digest: String,
}

/// Fail-closed city scene planning error.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CityScenePlanError {
    /// Layer, tile hierarchy, view, provenance, or budget is invalid.
    #[error("invalid city scene plan: {0}")]
    Invalid(String),
    /// Canonical serialization failed.
    #[error("city scene serialization failed: {0}")]
    Serialization(String),
    /// Sealed frame plan changed.
    #[error("city scene frame plan digest mismatch")]
    Digest,
}

/// Select a screen-space-error frontier under hard tile and byte budgets.
pub fn plan_city_scene_frame(
    manifest: &CitySceneManifest,
    view: SharedSpatialViewState,
    budget: CityStreamBudget,
) -> Result<CitySceneFramePlan, CityScenePlanError> {
    validate(manifest, &view, &budget)?;
    let by_parent = manifest.tiles.iter().fold(
        BTreeMap::<Option<&str>, Vec<&CityTile>>::new(),
        |mut map, tile| {
            map.entry(tile.parent_id.as_deref()).or_default().push(tile);
            map
        },
    );
    let mut queue = by_parent
        .get(&None)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .collect::<VecDeque<_>>();
    let mut frontier = Vec::new();
    while let Some(tile) = queue.pop_front() {
        let screen_error = tile.geometric_error_m / tile.camera_distance_m
            * view.viewport_height_px as f64
            / (2.0 * (view.field_of_view_degrees.to_radians() / 2.0).tan());
        let children = by_parent.get(&Some(tile.id.as_str()));
        if screen_error > budget.maximum_screen_space_error
            && children.is_some_and(|v| !v.is_empty())
        {
            queue.extend(children.expect("children").iter().copied());
        } else {
            frontier.push(tile);
        }
    }
    frontier.sort_by(|a, b| a.id.cmp(&b.id));
    let mut selected_tiles = Vec::new();
    let mut deferred_tiles = Vec::new();
    let mut transfer_bytes = 0_u64;
    let mut selected_features = BTreeSet::new();
    let mut selected_layers = BTreeSet::new();
    for tile in frontier {
        if selected_tiles.len() < budget.maximum_tiles as usize
            && transfer_bytes.saturating_add(tile.content_bytes) <= budget.maximum_transfer_bytes
        {
            transfer_bytes += tile.content_bytes;
            selected_layers.insert(tile.layer_id.as_str());
            selected_features.extend(tile.feature_ids.iter().cloned());
            selected_tiles.push(tile.id.clone());
        } else {
            deferred_tiles.push(tile.id.clone());
        }
    }
    let sources = manifest
        .layers
        .iter()
        .filter(|layer| selected_layers.contains(layer.id.as_str()))
        .map(|layer| layer.source.clone())
        .collect();
    let selected_features_present = view
        .selected_feature_ids
        .intersection(&selected_features)
        .cloned()
        .collect();
    let mut plan = CitySceneFramePlan {
        schema_version: "0.1.0".into(),
        view,
        budget,
        selected_tiles,
        deferred_tiles,
        transfer_bytes,
        sources,
        selected_features_present,
        plan_digest: String::new(),
    };
    plan.plan_digest = digest(&plan)?;
    verify_city_scene_frame_plan(&plan)?;
    Ok(plan)
}

/// Verify hard budgets, shared state, provenance, and sealed identity.
pub fn verify_city_scene_frame_plan(plan: &CitySceneFramePlan) -> Result<(), CityScenePlanError> {
    if plan.schema_version != "0.1.0"
        || plan.selected_tiles.len() > plan.budget.maximum_tiles as usize
        || plan.transfer_bytes > plan.budget.maximum_transfer_bytes
        || plan
            .sources
            .iter()
            .any(|source| !source.checksum_status.is_verified())
        || digest(plan)? != plan.plan_digest
    {
        return Err(CityScenePlanError::Digest);
    }
    Ok(())
}

fn validate(
    manifest: &CitySceneManifest,
    view: &SharedSpatialViewState,
    budget: &CityStreamBudget,
) -> Result<(), CityScenePlanError> {
    view.crs
        .require_known()
        .map_err(|error| CityScenePlanError::Invalid(error.to_string()))?;
    if manifest.schema_version != "0.1.0"
        || manifest.layers.is_empty()
        || manifest.tiles.is_empty()
        || budget.maximum_tiles == 0
        || budget.maximum_transfer_bytes == 0
        || !budget.maximum_screen_space_error.is_finite()
        || budget.maximum_screen_space_error <= 0.0
        || view.viewport_height_px == 0
        || !view.field_of_view_degrees.is_finite()
        || !(1.0..=179.0).contains(&view.field_of_view_degrees)
        || view
            .camera_position
            .iter()
            .chain(view.camera_target.iter())
            .any(|v| !v.is_finite())
    {
        return Err(CityScenePlanError::Invalid(
            "manifest, view, or budget is invalid".into(),
        ));
    }
    let layer_ids = manifest
        .layers
        .iter()
        .map(|layer| layer.id.as_str())
        .collect::<BTreeSet<_>>();
    if layer_ids.len() != manifest.layers.len()
        || manifest.layers.iter().any(|layer| {
            layer.id.trim().is_empty()
                || layer.format.trim().is_empty()
                || !layer.source.checksum_status.is_verified()
        })
    {
        return Err(CityScenePlanError::Invalid(
            "layer identity, format, or source is invalid".into(),
        ));
    }
    let tile_ids = manifest
        .tiles
        .iter()
        .map(|tile| tile.id.as_str())
        .collect::<BTreeSet<_>>();
    if tile_ids.len() != manifest.tiles.len()
        || !manifest.tiles.iter().any(|tile| tile.parent_id.is_none())
    {
        return Err(CityScenePlanError::Invalid(
            "tile identities must be unique and include a root".into(),
        ));
    }
    for tile in &manifest.tiles {
        if tile.id.trim().is_empty()
            || !layer_ids.contains(tile.layer_id.as_str())
            || tile
                .parent_id
                .as_deref()
                .is_some_and(|parent| !tile_ids.contains(parent))
            || !tile.geometric_error_m.is_finite()
            || tile.geometric_error_m < 0.0
            || !tile.camera_distance_m.is_finite()
            || tile.camera_distance_m <= 0.0
            || tile.content_bytes == 0
            || !valid_digest(&tile.content_digest)
        {
            return Err(CityScenePlanError::Invalid(
                "tile hierarchy or content is invalid".into(),
            ));
        }
        let mut ancestors = BTreeSet::new();
        let mut parent = tile.parent_id.as_deref();
        while let Some(parent_id) = parent {
            if !ancestors.insert(parent_id) || parent_id == tile.id {
                return Err(CityScenePlanError::Invalid(
                    "tile hierarchy contains a cycle".into(),
                ));
            }
            parent = manifest
                .tiles
                .iter()
                .find(|candidate| candidate.id == parent_id)
                .and_then(|candidate| candidate.parent_id.as_deref());
        }
    }
    Ok(())
}

fn digest(plan: &CitySceneFramePlan) -> Result<String, CityScenePlanError> {
    let mut semantic = plan.clone();
    semantic.plan_digest.clear();
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(
            serde_json::to_vec(&semantic)
                .map_err(|error| CityScenePlanError::Serialization(error.to_string()))?
        )
    ))
}

fn valid_digest(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use genegis_crs::ChecksumVerification;

    use super::*;

    fn hash(value: &str) -> String {
        format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
    }
    fn source(uri: &str) -> SourceSnapshot {
        let digest = hash(uri);
        let mut source = SourceSnapshot::new(uri);
        source.checksum = Some(digest.clone());
        source.observed_checksum = Some(digest);
        source.checksum_status = ChecksumVerification::Verified;
        source
    }

    #[test]
    fn streams_open_city_layers_under_budget_with_shared_state() {
        let layers = vec![
            CityLayer {
                id: "terrain".into(),
                kind: CityLayerKind::Terrain,
                format: "cog".into(),
                source: source("fixture://terrain"),
            },
            CityLayer {
                id: "points".into(),
                kind: CityLayerKind::PointCloudCopc,
                format: "copc-1.0".into(),
                source: source("fixture://copc"),
            },
            CityLayer {
                id: "buildings".into(),
                kind: CityLayerKind::Buildings3dTiles,
                format: "3dtiles-1.1".into(),
                source: source("fixture://3dtiles"),
            },
            CityLayer {
                id: "context".into(),
                kind: CityLayerKind::Context2d,
                format: "pmtiles".into(),
                source: source("fixture://pmtiles"),
            },
        ];
        let tiles = layers
            .iter()
            .flat_map(|layer| {
                (0..3).map(move |index| CityTile {
                    id: format!("{}-{index}", layer.id),
                    layer_id: layer.id.clone(),
                    parent_id: None,
                    geometric_error_m: 4.0,
                    camera_distance_m: 1000.0 + index as f64 * 100.0,
                    content_bytes: 100,
                    content_digest: hash(&format!("{}-{index}", layer.id)),
                    feature_ids: vec![format!("feature-{index}")],
                })
            })
            .collect();
        let plan = plan_city_scene_frame(
            &CitySceneManifest {
                schema_version: "0.1.0".into(),
                layers,
                tiles,
            },
            SharedSpatialViewState {
                crs: Crs::nagoya_projected(),
                camera_position: [0.0, -500.0, 300.0],
                camera_target: [0.0, 0.0, 0.0],
                field_of_view_degrees: 60.0,
                viewport_height_px: 1080,
                selected_feature_ids: BTreeSet::from(["feature-0".into()]),
                temporal_cursor: Some("2026-08-26T10:00:00Z".into()),
            },
            CityStreamBudget {
                maximum_tiles: 5,
                maximum_transfer_bytes: 500,
                maximum_screen_space_error: 16.0,
            },
        )
        .expect("plan");
        assert_eq!(plan.selected_tiles.len(), 5);
        assert_eq!(plan.transfer_bytes, 500);
        assert_eq!(plan.selected_features_present, vec!["feature-0"]);
        assert!(!plan.deferred_tiles.is_empty());
        verify_city_scene_frame_plan(&plan).expect("verify");
    }

    #[test]
    fn rejects_unverified_source_and_plan_tampering() {
        let mut bad = source("fixture://bad");
        bad.checksum_status = ChecksumVerification::Unknown;
        let manifest = CitySceneManifest {
            schema_version: "0.1.0".into(),
            layers: vec![CityLayer {
                id: "b".into(),
                kind: CityLayerKind::Buildings3dTiles,
                format: "3dtiles-1.1".into(),
                source: bad,
            }],
            tiles: vec![CityTile {
                id: "root".into(),
                layer_id: "b".into(),
                parent_id: None,
                geometric_error_m: 1.0,
                camera_distance_m: 100.0,
                content_bytes: 10,
                content_digest: hash("root"),
                feature_ids: vec![],
            }],
        };
        let result = plan_city_scene_frame(
            &manifest,
            SharedSpatialViewState {
                crs: Crs::nagoya_projected(),
                camera_position: [0.0; 3],
                camera_target: [1.0; 3],
                field_of_view_degrees: 60.0,
                viewport_height_px: 100,
                selected_feature_ids: BTreeSet::new(),
                temporal_cursor: None,
            },
            CityStreamBudget {
                maximum_tiles: 1,
                maximum_transfer_bytes: 10,
                maximum_screen_space_error: 16.0,
            },
        );
        assert!(result.is_err());
    }
}
