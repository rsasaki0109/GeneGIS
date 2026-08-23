//! Shared evidence and budget gates for cloud-native spatial I/O.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Cloud-native format whose selection was measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudFormat {
    /// Cloud Optimized GeoTIFF.
    Cog,
    /// GeoParquet.
    GeoParquet,
    /// Cloud Optimized Point Cloud.
    Copc,
    /// PMTiles archive.
    PmTiles,
}

/// Semantic subset selected from an object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum IoSelection {
    /// Raster pixel window at an overview level.
    CogWindow {
        /// Overview level, zero for base resolution.
        level: u32,
        /// First selected row.
        row_offset: u32,
        /// First selected column.
        column_offset: u32,
        /// Selected row count.
        rows: u32,
        /// Selected column count.
        columns: u32,
    },
    /// Selected Parquet row groups and optional bbox predicate.
    GeoParquetRowGroups {
        /// Zero-based row-group identities.
        row_groups: Vec<usize>,
        /// Decimal bbox coordinates, retained without floating-point normalization.
        bbox: Option<[String; 4]>,
    },
    /// Selected COPC hierarchy keys.
    CopcHierarchyNodes {
        /// Canonical COPC voxel keys.
        node_keys: Vec<String>,
    },
    /// One PMTiles z/x/y tile.
    PmTilesTile {
        /// Zoom level.
        z: u8,
        /// Tile column.
        x: u32,
        /// Tile row.
        y: u32,
    },
}

/// One request/seek observed by the fixture server or local range source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IoRequestEvidence {
    /// Inclusive byte offset.
    pub start: u64,
    /// Inclusive byte offset.
    pub end: u64,
    /// Actual response bytes.
    pub response_bytes: u64,
    /// HTTP status; absent for a local range seek.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
}

/// Measured GPU upload and presentation metrics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GpuFrameMetrics {
    /// Adapter name reported by wgpu.
    pub adapter: String,
    /// Backend reported by wgpu.
    pub backend: String,
    /// Bytes uploaded for the selected view.
    pub upload_bytes: u64,
    /// Upload duration.
    pub upload_ns: u64,
    /// Time from selection start to first frame.
    pub first_frame_ns: u64,
    /// Steady-state frames per second.
    pub steady_state_fps: f64,
}

/// Complete evidence for one selected-view read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IoReceipt {
    /// Receipt schema version.
    pub schema_version: String,
    /// Format.
    pub format: CloudFormat,
    /// Immutable fixture/object digest.
    pub object_digest: String,
    /// Encoded object bytes.
    pub object_bytes: u64,
    /// Logical decoded dataset bytes declared by the fixture manifest.
    pub logical_dataset_bytes: u64,
    /// Exact semantic subset.
    pub selection: IoSelection,
    /// Requests in observed order.
    pub requests: Vec<IoRequestEvidence>,
    /// Total transferred bytes.
    pub transferred_bytes: u64,
    /// Largest response.
    pub maximum_response_bytes: u64,
    /// Whether a whole-object path was used at any point.
    pub whole_object_fallback: bool,
    /// Records, pixels, points, or tile features decoded.
    pub decoded_items: u64,
    /// Wall duration for the selected operation.
    pub elapsed_ns: u64,
    /// Process high-water RSS sampled from the runner.
    pub peak_rss_bytes: u64,
    /// GPU evidence where the operation includes rendering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu: Option<GpuFrameMetrics>,
}

impl IoReceipt {
    /// Construct and normalize totals from exact request evidence.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        format: CloudFormat,
        object_digest: String,
        object_bytes: u64,
        logical_dataset_bytes: u64,
        selection: IoSelection,
        requests: Vec<IoRequestEvidence>,
        whole_object_fallback: bool,
        decoded_items: u64,
        elapsed_ns: u64,
        peak_rss_bytes: u64,
        gpu: Option<GpuFrameMetrics>,
    ) -> Self {
        let transferred_bytes = requests.iter().map(|request| request.response_bytes).sum();
        let maximum_response_bytes = requests
            .iter()
            .map(|request| request.response_bytes)
            .max()
            .unwrap_or(0);
        Self {
            schema_version: "0.1.0".into(),
            format,
            object_digest,
            object_bytes,
            logical_dataset_bytes,
            selection,
            requests,
            transferred_bytes,
            maximum_response_bytes,
            whole_object_fallback,
            decoded_items,
            elapsed_ns,
            peak_rss_bytes,
            gpu,
        }
    }

    /// Transfer ratio in the closed interval 0–1 for a non-empty object.
    pub fn transfer_ratio(&self) -> f64 {
        self.transferred_bytes as f64 / self.object_bytes.max(1) as f64
    }

    /// Stable digest over the complete receipt.
    pub fn digest(&self) -> Result<String, serde_json::Error> {
        let value = serde_json::to_value(self)?;
        Ok(format!(
            "sha256:{:x}",
            Sha256::digest(canonical_json(&value).as_bytes())
        ))
    }

    /// Evaluate every budget independently; an optimized result cannot hide fallback.
    pub fn validate(&self, budget: &IoBudget) -> Vec<IoBudgetFailure> {
        let mut failures = Vec::new();
        if self.object_bytes < budget.minimum_object_bytes {
            failures.push(IoBudgetFailure::ObjectTooSmall);
        }
        if self.logical_dataset_bytes < budget.minimum_logical_dataset_bytes {
            failures.push(IoBudgetFailure::LogicalDatasetTooSmall);
        }
        if self.whole_object_fallback {
            failures.push(IoBudgetFailure::WholeObjectFallback);
        }
        if self.transfer_ratio() > budget.maximum_transfer_ratio {
            failures.push(IoBudgetFailure::TransferRatioExceeded);
        }
        if self.maximum_response_bytes > budget.maximum_response_bytes {
            failures.push(IoBudgetFailure::ResponseExceeded);
        }
        if self.peak_rss_bytes > budget.maximum_peak_rss_bytes {
            failures.push(IoBudgetFailure::PeakRssExceeded);
        }
        if let Some(gpu) = &self.gpu {
            if gpu.first_frame_ns > budget.maximum_first_frame_ns {
                failures.push(IoBudgetFailure::FirstFrameExceeded);
            }
            if !gpu.steady_state_fps.is_finite() || gpu.steady_state_fps <= 0.0 {
                failures.push(IoBudgetFailure::InvalidGpuMetrics);
            }
        } else if budget.require_gpu {
            failures.push(IoBudgetFailure::GpuMetricsMissing);
        }
        if self.requests.iter().any(|request| {
            request.end < request.start
                || request.response_bytes != request.end - request.start + 1
                || request.response_bytes > budget.maximum_response_bytes
        }) {
            failures.push(IoBudgetFailure::MalformedRequestEvidence);
        }
        failures.sort();
        failures.dedup();
        failures
    }
}

/// Performance/resource acceptance budget.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IoBudget {
    /// Minimum encoded object size.
    pub minimum_object_bytes: u64,
    /// Minimum logical decoded dataset size.
    pub minimum_logical_dataset_bytes: u64,
    /// Maximum selected transfer/object ratio.
    pub maximum_transfer_ratio: f64,
    /// Maximum one response.
    pub maximum_response_bytes: u64,
    /// Maximum process high-water RSS.
    pub maximum_peak_rss_bytes: u64,
    /// Maximum first-frame latency.
    pub maximum_first_frame_ns: u64,
    /// Whether GPU evidence is mandatory.
    pub require_gpu: bool,
}

impl IoBudget {
    /// RFC 0004 full-fixture baseline.
    pub fn phase_12_full(require_gpu: bool) -> Self {
        Self {
            minimum_object_bytes: 256 * 1024 * 1024,
            minimum_logical_dataset_bytes: 1024 * 1024 * 1024,
            maximum_transfer_ratio: 0.2,
            maximum_response_bytes: 8 * 1024 * 1024,
            maximum_peak_rss_bytes: 1024 * 1024 * 1024,
            maximum_first_frame_ns: 2_000_000_000,
            require_gpu,
        }
    }

    /// Deterministic CI lane with the same ratio, response, fallback, RSS, and frame gates.
    pub fn ci(require_gpu: bool) -> Self {
        Self {
            minimum_object_bytes: 1,
            minimum_logical_dataset_bytes: 1,
            ..Self::phase_12_full(require_gpu)
        }
    }
}

/// A reason an I/O receipt cannot pass the selected budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IoBudgetFailure {
    /// Encoded fixture is below the declared lane size.
    ObjectTooSmall,
    /// Logical dataset is below the declared lane size.
    LogicalDatasetTooSmall,
    /// A full download/read occurred.
    WholeObjectFallback,
    /// Aggregate selected transfer exceeded the ratio.
    TransferRatioExceeded,
    /// One response exceeded the maximum.
    ResponseExceeded,
    /// Process high-water memory exceeded budget.
    PeakRssExceeded,
    /// First frame exceeded latency budget.
    FirstFrameExceeded,
    /// GPU rendering was required but not measured.
    GpuMetricsMissing,
    /// GPU metrics were nonsensical.
    InvalidGpuMetrics,
    /// A range or its byte count was internally inconsistent.
    MalformedRequestEvidence,
}

fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut entries = map.iter().collect::<Vec<_>>();
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

    fn receipt(fallback: bool) -> IoReceipt {
        IoReceipt::new(
            CloudFormat::Cog,
            format!("sha256:{}", "a".repeat(64)),
            1000,
            4000,
            IoSelection::CogWindow {
                level: 0,
                row_offset: 0,
                column_offset: 0,
                rows: 10,
                columns: 10,
            },
            vec![IoRequestEvidence {
                start: 0,
                end: 99,
                response_bytes: 100,
                http_status: Some(206),
            }],
            fallback,
            100,
            1,
            1000,
            None,
        )
    }

    #[test]
    fn optimized_receipt_passes_and_digest_is_stable() {
        let receipt = receipt(false);
        assert!(receipt.validate(&IoBudget::ci(false)).is_empty());
        assert_eq!(receipt.digest().unwrap(), receipt.digest().unwrap());
    }

    #[test]
    fn fallback_oversize_missing_gpu_and_malformed_range_fail_independently() {
        let mut receipt = receipt(true);
        receipt.requests[0].response_bytes = 99;
        receipt.transferred_bytes = 900;
        let failures = receipt.validate(&IoBudget::ci(true));
        assert!(failures.contains(&IoBudgetFailure::WholeObjectFallback));
        assert!(failures.contains(&IoBudgetFailure::TransferRatioExceeded));
        assert!(failures.contains(&IoBudgetFailure::GpuMetricsMissing));
        assert!(failures.contains(&IoBudgetFailure::MalformedRequestEvidence));
    }
}
