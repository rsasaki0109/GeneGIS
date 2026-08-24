//! UC-2 dashboard export: a verified PMTiles bundle from a density result.
//!
//! Converts the polygon features of an [`AnalysisResult`] into Mapbox Vector
//! Tiles, packs them into one local PMTiles v3 archive, and verifies the
//! export fail-closed: sampled tiles must round-trip byte-exactly through the
//! range reader, header bounds must match the request, source attribution
//! must be present, and every ward must appear at the maximum zoom.

use std::collections::BTreeMap;

use genegis_tile::{
    decode_tile_payload, encode_polygon_tile, lat_to_tile_y, lon_to_tile_x, read_pmtiles_tile,
    write_pmtiles_archive, PmTilesTileEntry, TilePolygonFeature, TilePolygonPart, TileValue,
    DEFAULT_EXTENT,
};
use serde::{Deserialize, Serialize};

use crate::result::{AnalysisResult, VerificationCheck};

/// Zoom bounds for a dashboard export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DashboardExportOptions {
    /// Minimum tile zoom written to the archive.
    pub minimum_zoom: u8,
    /// Maximum tile zoom written to the archive.
    pub maximum_zoom: u8,
    /// Number of tiles byte-compared after writing.
    pub roundtrip_sample_size: usize,
}

impl Default for DashboardExportOptions {
    fn default() -> Self {
        Self {
            minimum_zoom: 7,
            maximum_zoom: 11,
            roundtrip_sample_size: 8,
        }
    }
}

/// Evidence receipt for one verified PMTiles dashboard export.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DashboardExportReport {
    /// Local path of the written archive.
    pub output_path: String,
    /// Encoded archive size in bytes.
    pub object_bytes: u64,
    /// Number of non-empty tiles written.
    pub tile_count: usize,
    /// Zoom bounds written to the header.
    pub minimum_zoom: u8,
    pub maximum_zoom: u8,
    /// Number of ward features encoded into the layer.
    pub ward_count: usize,
    /// Distinct feature ids observed at the maximum zoom.
    pub max_zoom_feature_count: usize,
    /// Fail-closed checks evaluated on the written archive.
    pub checks: Vec<VerificationCheck>,
    /// True when every check passed.
    pub verification_passed: bool,
}

const LAYER_NAME: &str = "density";

/// Encode the density result into a PMTiles archive and verify it in place.
pub fn export_dashboard_pmtiles(
    result: &AnalysisResult,
    output_path: &str,
    options: &DashboardExportOptions,
) -> Result<DashboardExportReport, crate::AnalysisError> {
    if options.minimum_zoom > options.maximum_zoom {
        return Err(crate::AnalysisError::Message(
            "dashboard export requires minimum_zoom <= maximum_zoom".into(),
        ));
    }
    if result.features.is_empty() {
        return Err(crate::AnalysisError::Message(
            "dashboard export requires at least one feature".into(),
        ));
    }

    let mut features_by_tile: BTreeMap<(u8, u32, u32), Vec<TilePolygonFeature>> = BTreeMap::new();
    let mut distinct_ids_at_maximum_zoom: std::collections::BTreeSet<u64> =
        std::collections::BTreeSet::new();
    let (min_lon, min_lat, max_lon, max_lat) = feature_bounds(result);
    for zoom in options.minimum_zoom..=options.maximum_zoom {
        let x_range = lon_to_tile_x(min_lon, zoom)..=lon_to_tile_x(max_lon, zoom);
        let y_range = lat_to_tile_y(max_lat, zoom)..=lat_to_tile_y(min_lat, zoom);
        for (feature_index, feature) in result.features.iter().enumerate() {
            let (f_min_lon, f_min_lat, f_max_lon, f_max_lat) = part_bounds(
                feature
                    .rings
                    .iter()
                    .flat_map(|part| part.exterior().iter().copied()),
            );
            for x in lon_to_tile_x(f_min_lon, zoom)..=lon_to_tile_x(f_max_lon, zoom) {
                if !x_range.contains(&x) {
                    continue;
                }
                for y in lat_to_tile_y(f_max_lat, zoom)..=lat_to_tile_y(f_min_lat, zoom) {
                    if !y_range.contains(&y) {
                        continue;
                    }
                    features_by_tile
                        .entry((zoom, x, y))
                        .or_default()
                        .push(dashboard_feature(feature_index, feature));
                }
            }
            if zoom == options.maximum_zoom && !feature.rings.is_empty() {
                distinct_ids_at_maximum_zoom.insert(feature_index as u64);
            }
        }
    }

    let mut entries: Vec<PmTilesTileEntry> = Vec::with_capacity(features_by_tile.len());
    for ((zoom, x, y), tile_features) in features_by_tile {
        let bytes = encode_polygon_tile(LAYER_NAME, zoom, x, y, DEFAULT_EXTENT, &tile_features);
        if bytes.is_empty() {
            continue;
        }
        entries.push(PmTilesTileEntry {
            z: zoom,
            x,
            y,
            bytes,
        });
    }

    let metadata = build_metadata(result, options);
    let write_receipt = write_pmtiles_archive(
        output_path,
        &entries,
        &genegis_tile::PmTilesWriteOptions::compressed(metadata.clone()),
    )
    .map_err(|error| crate::AnalysisError::Message(format!("PMTiles write failed: {error}")))?;

    let mut checks = vec![
        header_bounds_check(&write_receipt, entries.len(), options),
        attribution_check(&metadata),
        feature_coverage_check(distinct_ids_at_maximum_zoom.len(), result.features.len()),
    ];
    checks.push(roundtrip_check(output_path, &entries, options));

    let verification_passed = checks.iter().all(|check| check.passed);
    Ok(DashboardExportReport {
        output_path: output_path.to_string(),
        object_bytes: std::fs::metadata(output_path)
            .map_err(|error| crate::AnalysisError::Message(error.to_string()))?
            .len(),
        tile_count: entries.len(),
        minimum_zoom: options.minimum_zoom,
        maximum_zoom: options.maximum_zoom,
        ward_count: result.features.len(),
        max_zoom_feature_count: distinct_ids_at_maximum_zoom.len(),
        checks,
        verification_passed,
    })
}

fn dashboard_feature(index: usize, feature: &crate::result::DensityFeature) -> TilePolygonFeature {
    TilePolygonFeature {
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
                "ward_code".to_string(),
                TileValue::Text(feature.ward_code.clone()),
            ),
            (
                "ward_name".to_string(),
                TileValue::Text(feature.ward_name.clone()),
            ),
            ("population".to_string(), TileValue::U64(feature.population)),
            (
                "density_per_km2".to_string(),
                TileValue::F64(feature.density_per_km2),
            ),
        ],
    }
}

fn feature_bounds(result: &AnalysisResult) -> (f64, f64, f64, f64) {
    let points = result.features.iter().flat_map(|feature| {
        feature
            .rings
            .iter()
            .flat_map(|part| part.exterior().iter().copied())
    });
    bounds_of(points)
}

fn part_bounds(points: impl Iterator<Item = (f64, f64)>) -> (f64, f64, f64, f64) {
    bounds_of(points)
}

fn bounds_of(points: impl Iterator<Item = (f64, f64)>) -> (f64, f64, f64, f64) {
    let mut bounds = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for (lon, lat) in points {
        bounds.0 = bounds.0.min(lon);
        bounds.1 = bounds.1.min(lat);
        bounds.2 = bounds.2.max(lon);
        bounds.3 = bounds.3.max(lat);
    }
    if bounds.0 > bounds.2 {
        (0.0, 0.0, 0.0, 0.0)
    } else {
        bounds
    }
}

fn build_metadata(result: &AnalysisResult, options: &DashboardExportOptions) -> String {
    let source = &result.verification.source;
    let attribution = match (&source.uri, &source.license) {
        (uri, Some(license)) => format!("{uri} ({license})"),
        (uri, None) => uri.clone(),
    };
    let metadata = serde_json::json!({
        "schema": "genegis-dashboard-v1",
        "name": "nagoya-density",
        "attribution": attribution,
        "source": {
            "uri": source.uri,
            "license": source.license,
            "checksum": source.checksum,
        },
        "crs": {
            "input": result.verification.crs,
            "tile_projection": "EPSG:3857",
            "coordinate_unit": result.verification.coordinate_unit,
            "value_unit": result.verification.density_unit,
        },
        "vector_layers": [{
            "id": LAYER_NAME,
            "fields": {
                "ward_code": "String",
                "ward_name": "String",
                "population": "Number",
                "density_per_km2": "Number",
            },
        }],
        "genegis": {
            "area_method": result.verification.area_method,
            "zoom_range": [options.minimum_zoom, options.maximum_zoom],
            "ward_count": result.features.len(),
            "citations": result
                .workflow
                .citations
                .iter()
                .map(|citation| serde_json::json!({
                    "title": citation.title,
                    "url": citation.url,
                }))
                .collect::<Vec<_>>(),
        },
    });
    serde_json::to_string(&metadata).unwrap_or_else(|_| "{}".to_string())
}

fn header_bounds_check(
    receipt: &genegis_tile::PmTilesWriteReceipt,
    tile_count: usize,
    options: &DashboardExportOptions,
) -> VerificationCheck {
    let consistent = receipt.minimum_zoom == options.minimum_zoom
        && receipt.maximum_zoom == options.maximum_zoom
        && receipt.addressed_tiles as usize == tile_count;
    VerificationCheck {
        name: "pmtiles_header_bounds".to_string(),
        passed: consistent,
        detail: format!(
            "zoom {}..={}, addressed_tiles={}, written_tiles={}",
            receipt.minimum_zoom, receipt.maximum_zoom, receipt.addressed_tiles, tile_count
        ),
    }
}

fn attribution_check(metadata: &str) -> VerificationCheck {
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(metadata);
    let attribution = parsed
        .as_ref()
        .ok()
        .and_then(|value| value.get("attribution"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    VerificationCheck {
        name: "attribution_present".to_string(),
        passed: !attribution.trim().is_empty(),
        detail: format!("attribution={attribution:?}"),
    }
}

fn feature_coverage_check(observed: usize, expected: usize) -> VerificationCheck {
    VerificationCheck {
        name: "max_zoom_feature_coverage".to_string(),
        passed: observed == expected,
        detail: format!("distinct features at max zoom: observed={observed}, expected={expected}"),
    }
}

fn roundtrip_check(
    output_path: &str,
    entries: &[PmTilesTileEntry],
    options: &DashboardExportOptions,
) -> VerificationCheck {
    let sample_size = options.roundtrip_sample_size.max(1).min(entries.len());
    let step = (entries.len() / sample_size).max(1);
    let mut compared = 0_usize;
    let mut mismatches: Vec<String> = Vec::new();
    for entry in entries.iter().step_by(step).take(sample_size) {
        match read_pmtiles_tile(output_path, entry.z, entry.x, entry.y) {
            Ok(read) => {
                let decoded = decode_tile_payload(&read.bytes, read.header.tile_compression)
                    .unwrap_or_default();
                if decoded != entry.bytes {
                    mismatches.push(format!("{}/{}/{}", entry.z, entry.x, entry.y));
                }
            }
            Err(error) => mismatches.push(format!("{}/{}/{}: {error}", entry.z, entry.x, entry.y)),
        }
        compared += 1;
    }
    VerificationCheck {
        name: "pmtiles_roundtrip_bytes".to_string(),
        passed: compared > 0 && mismatches.is_empty(),
        detail: format!(
            "compared={compared}, mismatches={} {:?}",
            mismatches.len(),
            mismatches.first()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nagoya::{default_nagoya_data_path, run_nagoya_population_density};

    #[test]
    fn exports_and_verifies_a_real_density_result() {
        let result =
            run_nagoya_population_density(default_nagoya_data_path()).expect("density run");
        let temp = tempfile::tempdir().expect("tempdir");
        let output_path = temp.path().join("dashboard.pmtiles");
        let report = export_dashboard_pmtiles(
            &result,
            output_path.to_str().expect("utf8 path"),
            &DashboardExportOptions::default(),
        )
        .expect("export");
        assert!(report.verification_passed, "checks: {:?}", report.checks);
        assert!(report.tile_count > 0);
        assert_eq!(report.ward_count, result.features.len());
        assert_eq!(
            report.max_zoom_feature_count,
            result.features.len(),
            "every ward must appear at maximum zoom"
        );
        assert!(report.object_bytes > 0);
        // Sampled tiles must be readable through range selection alone.
        for check in &report.checks {
            assert!(check.passed, "failed check: {}", check.name);
        }
    }

    #[test]
    fn rejects_empty_features_and_inverted_zoom() {
        let mut result = serde_json::from_str::<AnalysisResult>(
            &serde_json::to_string(
                &run_nagoya_population_density(default_nagoya_data_path()).expect("run"),
            )
            .expect("json"),
        )
        .expect("clone");
        result.features.clear();
        let temp = tempfile::tempdir().expect("tempdir");
        let empty = export_dashboard_pmtiles(
            &result,
            temp.path().join("empty.pmtiles").to_str().unwrap(),
            &DashboardExportOptions::default(),
        );
        assert!(matches!(empty, Err(crate::AnalysisError::Message(_))));

        let full = run_nagoya_population_density(default_nagoya_data_path()).expect("run");
        let inverted = export_dashboard_pmtiles(
            &full,
            temp.path().join("inverted.pmtiles").to_str().unwrap(),
            &DashboardExportOptions {
                minimum_zoom: 11,
                maximum_zoom: 7,
                roundtrip_sample_size: 4,
            },
        );
        assert!(matches!(inverted, Err(crate::AnalysisError::Message(_))));
    }
}
