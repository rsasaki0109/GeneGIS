//! Digest-bound temporal layer playback and per-epoch MVT encoding evidence.

use std::{collections::BTreeMap, time::Instant};

use genegis_core::WorkflowDigest;
use genegis_crs::SourceMetadata;
use genegis_tile::{
    encode_polygon_tile, lat_to_tile_y, lon_to_tile_x, TilePolygonFeature, TilePolygonPart,
    TileValue, DEFAULT_EXTENT,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{AnalysisError, NdviTimeseriesAnalysis};

/// Contract version for serialized temporal playback state.
pub const TEMPORAL_PLAYBACK_SCHEMA_VERSION: &str = "0.1.0";
/// Fixed stream zoom for the first temporal playback slice.
pub const TEMPORAL_TILE_ZOOM: u8 = 10;

/// Explicit per-epoch vector-tile encoding limits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TileEncodingBudget {
    /// Maximum encoded bytes accepted for one tile.
    pub maximum_tile_bytes: u64,
    /// Maximum sum of encoded bytes accepted for one epoch layer.
    pub maximum_layer_bytes: u64,
    /// Maximum wall-clock encoding time accepted for one epoch layer.
    pub maximum_encode_ns: u64,
}

impl Default for TileEncodingBudget {
    fn default() -> Self {
        Self {
            maximum_tile_bytes: 512 * 1024,
            maximum_layer_bytes: 8 * 1024 * 1024,
            maximum_encode_ns: 2_000_000_000,
        }
    }
}

/// Measured evidence for one encoded temporal layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TileEncodingReceipt {
    /// MVT zoom encoded for playback.
    pub zoom: u8,
    /// Number of non-empty tiles.
    pub tile_count: u64,
    /// Sum of encoded MVT payload bytes.
    pub encoded_bytes: u64,
    /// Largest encoded tile payload.
    pub largest_tile_bytes: u64,
    /// Measured encoding wall time.
    pub encode_ns: u64,
    /// SHA-256 over ordered z/x/y and payload bytes.
    pub tile_set_digest: String,
    /// Budget evaluated against these measurements.
    pub budget: TileEncodingBudget,
    /// True only when every budget dimension passed.
    pub budget_passed: bool,
}

/// One ward value exposed to the playback widget for an epoch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemporalFeatureValue {
    /// Stable ward identifier.
    pub id: String,
    /// Human-readable ward label.
    pub label: String,
    /// Verified NDVI zonal mean.
    pub value: f64,
}

/// One ordered temporal layer and its stream evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemporalEpochLayer {
    /// STAC item identifier.
    pub id: String,
    /// ISO-8601 acquisition datetime.
    pub datetime: String,
    /// Semantic measure unit.
    pub value_unit: String,
    /// Values rendered by the workbench widget.
    pub values: Vec<TemporalFeatureValue>,
    /// Actual MVT encoding evidence for this epoch.
    pub encoding: TileEncodingReceipt,
}

/// Verified playback state shared by UI, workflow, and offline evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemporalPlayback {
    /// Playback schema version.
    pub schema_version: String,
    /// CRS shared by all epoch geometries.
    pub crs: String,
    /// Coordinate unit derived from the CRS contract.
    pub coordinate_unit: String,
    /// Workflow graph identity that produced the values.
    pub workflow_digest: WorkflowDigest,
    /// Verified analysis result identity.
    pub result_digest: String,
    /// Immutable sources authorized by the command.
    pub source_snapshots: Vec<SourceMetadata>,
    /// Strictly ordered epoch layers.
    pub epochs: Vec<TemporalEpochLayer>,
    /// Canonical digest of every field above.
    pub playback_digest: String,
}

impl TemporalPlayback {
    /// Recompute structural and budget checks and verify the binding digest.
    pub fn verify(&self) -> Result<(), AnalysisError> {
        if self.schema_version != TEMPORAL_PLAYBACK_SCHEMA_VERSION {
            return Err(AnalysisError::Message(
                "unsupported temporal playback schema".into(),
            ));
        }
        if self.epochs.len() < 2 || self.workflow_digest.is_empty() || self.result_digest.is_empty()
        {
            return Err(AnalysisError::Message(
                "temporal playback requires two epochs and workflow/result digests".into(),
            ));
        }
        if self
            .epochs
            .windows(2)
            .any(|pair| pair[0].datetime >= pair[1].datetime)
        {
            return Err(AnalysisError::Message(
                "temporal playback epochs must be strictly ordered".into(),
            ));
        }
        for epoch in &self.epochs {
            let receipt = &epoch.encoding;
            let passed = receipt.tile_count > 0
                && receipt.encoded_bytes > 0
                && receipt.largest_tile_bytes <= receipt.budget.maximum_tile_bytes
                && receipt.encoded_bytes <= receipt.budget.maximum_layer_bytes
                && receipt.encode_ns <= receipt.budget.maximum_encode_ns;
            if !passed || !receipt.budget_passed || epoch.values.is_empty() {
                return Err(AnalysisError::Message(format!(
                    "temporal epoch {} failed encoding evidence",
                    epoch.id
                )));
            }
        }
        if playback_digest(self)? != self.playback_digest {
            return Err(AnalysisError::Message(
                "temporal playback digest mismatch".into(),
            ));
        }
        Ok(())
    }
}

/// Encode every verified NDVI epoch into an MVT layer and bind execution evidence.
pub fn build_ndvi_temporal_playback(
    analysis: &NdviTimeseriesAnalysis,
    crs: &str,
    coordinate_unit: &str,
    workflow_digest: WorkflowDigest,
    result_digest: String,
    source_snapshots: Vec<SourceMetadata>,
) -> Result<TemporalPlayback, AnalysisError> {
    if analysis.epochs.len() < 2
        || analysis
            .features
            .iter()
            .any(|feature| feature.mean_ndvi_per_epoch.len() != analysis.epochs.len())
    {
        return Err(AnalysisError::Message(
            "NDVI temporal layer values do not reconcile with epochs".into(),
        ));
    }
    let budget = TileEncodingBudget::default();
    let mut epochs = Vec::with_capacity(analysis.epochs.len());
    for (epoch_index, epoch) in analysis.epochs.iter().enumerate() {
        let started = Instant::now();
        let tiles = encode_epoch_tiles(analysis, epoch_index);
        let encode_ns = started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
        let encoded_bytes = tiles.iter().map(|(_, bytes)| bytes.len() as u64).sum();
        let largest_tile_bytes = tiles
            .iter()
            .map(|(_, bytes)| bytes.len() as u64)
            .max()
            .unwrap_or(0);
        let tile_set_digest = digest_tiles(&tiles);
        let budget_passed = !tiles.is_empty()
            && encoded_bytes > 0
            && largest_tile_bytes <= budget.maximum_tile_bytes
            && encoded_bytes <= budget.maximum_layer_bytes
            && encode_ns <= budget.maximum_encode_ns;
        epochs.push(TemporalEpochLayer {
            id: epoch.item_id.clone(),
            datetime: epoch.datetime.clone(),
            value_unit: "NDVI".into(),
            values: analysis
                .features
                .iter()
                .map(|feature| TemporalFeatureValue {
                    id: feature.ward_code.clone(),
                    label: feature.ward_name.clone(),
                    value: feature.mean_ndvi_per_epoch[epoch_index],
                })
                .collect(),
            encoding: TileEncodingReceipt {
                zoom: TEMPORAL_TILE_ZOOM,
                tile_count: tiles.len() as u64,
                encoded_bytes,
                largest_tile_bytes,
                encode_ns,
                tile_set_digest,
                budget: budget.clone(),
                budget_passed,
            },
        });
    }
    let mut playback = TemporalPlayback {
        schema_version: TEMPORAL_PLAYBACK_SCHEMA_VERSION.into(),
        crs: crs.into(),
        coordinate_unit: coordinate_unit.into(),
        workflow_digest,
        result_digest,
        source_snapshots,
        epochs,
        playback_digest: String::new(),
    };
    playback.playback_digest = playback_digest(&playback)?;
    playback.verify()?;
    Ok(playback)
}

fn encode_epoch_tiles(
    analysis: &NdviTimeseriesAnalysis,
    epoch_index: usize,
) -> Vec<((u8, u32, u32), Vec<u8>)> {
    let mut grouped: BTreeMap<(u8, u32, u32), Vec<TilePolygonFeature>> = BTreeMap::new();
    for (index, feature) in analysis.features.iter().enumerate() {
        let bounds = bounds_of(
            feature
                .rings
                .iter()
                .flat_map(|part| part.exterior().iter().copied()),
        );
        for x in lon_to_tile_x(bounds.0, TEMPORAL_TILE_ZOOM)
            ..=lon_to_tile_x(bounds.2, TEMPORAL_TILE_ZOOM)
        {
            for y in lat_to_tile_y(bounds.3, TEMPORAL_TILE_ZOOM)
                ..=lat_to_tile_y(bounds.1, TEMPORAL_TILE_ZOOM)
            {
                grouped
                    .entry((TEMPORAL_TILE_ZOOM, x, y))
                    .or_default()
                    .push(TilePolygonFeature {
                        id: index as u64,
                        parts: feature
                            .rings
                            .iter()
                            .map(|part| TilePolygonPart {
                                exterior: part.exterior().to_vec(),
                                holes: part.holes().to_vec(),
                            })
                            .collect(),
                        properties: vec![
                            (
                                "ward_code".into(),
                                TileValue::Text(feature.ward_code.clone()),
                            ),
                            (
                                "ward_name".into(),
                                TileValue::Text(feature.ward_name.clone()),
                            ),
                            (
                                "mean_ndvi".into(),
                                TileValue::F64(feature.mean_ndvi_per_epoch[epoch_index]),
                            ),
                        ],
                    });
            }
        }
    }
    grouped
        .into_iter()
        .filter_map(|(tile, features)| {
            let bytes = encode_polygon_tile(
                "ndvi_epoch",
                tile.0,
                tile.1,
                tile.2,
                DEFAULT_EXTENT,
                &features,
            );
            (!bytes.is_empty()).then_some((tile, bytes))
        })
        .collect()
}

fn bounds_of(points: impl Iterator<Item = (f64, f64)>) -> (f64, f64, f64, f64) {
    let mut bounds = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for (x, y) in points {
        bounds.0 = bounds.0.min(x);
        bounds.1 = bounds.1.min(y);
        bounds.2 = bounds.2.max(x);
        bounds.3 = bounds.3.max(y);
    }
    if bounds.0 > bounds.2 {
        (0.0, 0.0, 0.0, 0.0)
    } else {
        bounds
    }
}

fn digest_tiles(tiles: &[((u8, u32, u32), Vec<u8>)]) -> String {
    let mut hasher = Sha256::new();
    for ((z, x, y), bytes) in tiles {
        hasher.update([*z]);
        hasher.update(x.to_le_bytes());
        hasher.update(y.to_le_bytes());
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn playback_digest(playback: &TemporalPlayback) -> Result<String, AnalysisError> {
    let value = serde_json::json!({
        "schema_version": playback.schema_version,
        "crs": playback.crs,
        "coordinate_unit": playback.coordinate_unit,
        "workflow_digest": playback.workflow_digest,
        "result_digest": playback.result_digest,
        "source_snapshots": playback.source_snapshots,
        "epochs": playback.epochs,
    });
    let bytes =
        serde_json::to_vec(&value).map_err(|error| AnalysisError::Message(error.to_string()))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_ordered_ndvi_epochs_with_per_layer_budgets() {
        let density = crate::run_nagoya_population_density(crate::default_nagoya_data_path())
            .expect("ward geometry fixture");
        let analysis = crate::NdviTimeseriesAnalysis {
            workflow: density.workflow.clone(),
            features: density
                .features
                .iter()
                .enumerate()
                .map(|(index, feature)| {
                    let first = 0.1 + index as f64 / 100.0;
                    let second = first + 0.03;
                    crate::NdviFeature {
                        ward_code: feature.ward_code.clone(),
                        ward_name: feature.ward_name.clone(),
                        population: feature.population,
                        mean_ndvi_per_epoch: vec![first, second],
                        delta_ndvi: second - first,
                        change_class: "gain".into(),
                        sampled_pixels: 16,
                        rings: feature.rings.clone(),
                        color: feature.color,
                    }
                })
                .collect(),
            style: density.style,
            verification: density.verification,
            citations: vec![],
            epochs: vec![
                crate::NdviEpochSummary {
                    item_id: "ndvi-2025-04".into(),
                    datetime: "2025-04-15T01:00:00Z".into(),
                    mean_ndvi: 0.2,
                },
                crate::NdviEpochSummary {
                    item_id: "ndvi-2025-10".into(),
                    datetime: "2025-10-15T01:00:00Z".into(),
                    mean_ndvi: 0.23,
                },
            ],
            width: 64,
            height: 64,
        };
        let playback = build_ndvi_temporal_playback(
            &analysis,
            "EPSG:4326",
            "degrees",
            WorkflowDigest::new("sha256:workflow"),
            "sha256:result".into(),
            vec![],
        )
        .expect("temporal playback");
        assert_eq!(playback.epochs.len(), 2);
        assert!(playback.epochs.iter().all(|epoch| {
            epoch.encoding.budget_passed
                && epoch.encoding.tile_count > 0
                && epoch.values.len() == analysis.features.len()
        }));
        playback.verify().expect("verified playback");

        let mut tampered = playback;
        tampered.epochs[0].values[0].value += 0.1;
        assert!(tampered.verify().is_err());
    }
}
