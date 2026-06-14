use std::path::{Path, PathBuf};

use genegis_geometry::BoundingBox;
use genegis_storage::read_asset_bytes;
use serde_json::Value;

use crate::dataset::{DatasetFormat, DatasetRecord};
use crate::error::CatalogError;
use crate::stac::{StacCollection, StacItem};

/// Default overlay path for imported STAC items.
pub const CATALOG_OVERLAY_PATH: &str = ".genegis/catalog-overlay.json";

/// Fetch JSON bytes from an HTTP(S) URL or local filesystem path.
pub fn fetch_json_bytes(uri: &str) -> Result<Vec<u8>, CatalogError> {
    let normalized = resolve_catalog_url(uri);
    read_asset_bytes(&normalized).map_err(|err| CatalogError::Remote(err.to_string()))
}

/// Resolve repo-relative catalog paths for offline smoke fixtures.
pub fn resolve_catalog_url(uri: &str) -> String {
    let normalized = normalize_fetch_uri(uri);
    if normalized.starts_with("examples/") {
        return crate::catalog::repo_root()
            .join(&normalized)
            .to_string_lossy()
            .into_owned();
    }
    normalized
}

/// Fetch and parse an external STAC Collection document.
pub fn fetch_stac_collection(uri: &str) -> Result<StacCollection, CatalogError> {
    let bytes = fetch_json_bytes(uri)?;
    parse_stac_collection(&bytes)
}

/// Fetch and parse an external STAC Item document.
pub fn fetch_stac_item(uri: &str) -> Result<StacItem, CatalogError> {
    let bytes = fetch_json_bytes(uri)?;
    parse_stac_item(&bytes)
}

pub fn parse_stac_collection(bytes: &[u8]) -> Result<StacCollection, CatalogError> {
    serde_json::from_slice(bytes)
        .map_err(|err| CatalogError::InvalidStac(format!("collection: {err}")))
}

pub fn parse_stac_item(bytes: &[u8]) -> Result<StacItem, CatalogError> {
    serde_json::from_slice(bytes)
        .map_err(|err| CatalogError::InvalidStac(format!("item: {err}")))
}

/// Convert a STAC Item into a catalog record using the primary data asset.
pub fn stac_item_to_dataset_record(item: &StacItem) -> Result<DatasetRecord, CatalogError> {
    let (asset_key, asset) = select_data_asset(item)?;
    let format = format_from_media_type(&asset.media_type);
    let title = item
        .properties
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or(&item.id)
        .to_string();
    let description = item
        .properties
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("Imported STAC item")
        .to_string();

    Ok(DatasetRecord {
        id: item.id.clone(),
        title,
        description,
        format,
        crs: "EPSG:4326".into(),
        bbox: BoundingBox::new(item.bbox[0], item.bbox[1], item.bbox[2], item.bbox[3]),
        uri: asset.href.clone(),
        license: item
            .properties
            .get("license")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .into(),
        tags: vec![
            "stac".into(),
            "imported".into(),
            format!("asset:{asset_key}"),
        ],
    })
}

fn select_data_asset(item: &StacItem) -> Result<(&String, &crate::stac::StacAsset), CatalogError> {
    item.assets
        .iter()
        .find(|(_, asset)| asset.roles.iter().any(|role| role == "data"))
        .or_else(|| item.assets.iter().next())
        .ok_or_else(|| CatalogError::InvalidStac("STAC item has no assets".into()))
}

fn format_from_media_type(media_type: &str) -> DatasetFormat {
    if media_type.contains("parquet") {
        DatasetFormat::geoparquet()
    } else if media_type.contains("tiff") || media_type.contains("geotiff") {
        DatasetFormat::cog()
    } else {
        DatasetFormat::geojson()
    }
}

/// Load imported dataset records from the overlay file.
pub fn load_catalog_overlay() -> Vec<DatasetRecord> {
    let path = Path::new(CATALOG_OVERLAY_PATH);
    if !path.exists() {
        return Vec::new();
    }
    let json = match std::fs::read_to_string(path) {
        Ok(json) => json,
        Err(_) => return Vec::new(),
    };
    serde_json::from_str(&json).unwrap_or_default()
}

/// Persist imported dataset records to the overlay file.
pub fn save_catalog_overlay(records: &[DatasetRecord]) -> Result<(), CatalogError> {
    let path = Path::new(CATALOG_OVERLAY_PATH);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| CatalogError::Remote(format!("overlay dir: {err}")))?;
    }
    let json = serde_json::to_string_pretty(records)
        .map_err(|err| CatalogError::InvalidStac(format!("overlay json: {err}")))?;
    std::fs::write(path, json).map_err(|err| CatalogError::Remote(format!("overlay write: {err}")))
}

/// Fetch a STAC Item and append it to the catalog overlay.
pub fn import_stac_item_url(uri: &str) -> Result<DatasetRecord, CatalogError> {
    let item = fetch_stac_item(uri)?;
    let mut record = stac_item_to_dataset_record(&item)?;
    record.uri = resolve_asset_href(&record.uri, uri);
    let mut overlay = load_catalog_overlay();
    overlay.retain(|existing| existing.id != record.id);
    overlay.push(record.clone());
    save_catalog_overlay(&overlay)?;
    Ok(record)
}

fn resolve_asset_href(href: &str, item_uri: &str) -> String {
    if href.starts_with("http://")
        || href.starts_with("https://")
        || href.starts_with("file://")
        || Path::new(href).is_absolute()
    {
        return normalize_fetch_uri(href);
    }

    let base = normalize_fetch_uri(item_uri);
    let base_path = PathBuf::from(base);
    let parent = base_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    parent.join(href).to_string_lossy().into_owned()
}

fn normalize_fetch_uri(uri: &str) -> String {
    if let Some(rest) = uri.strip_prefix("file://") {
        if let Some(path) = rest.strip_prefix('/') {
            if rest.len() > 2 && rest.as_bytes()[2] == b':' {
                return rest.to_string();
            }
            return format!("/{path}");
        }
        return rest.to_string();
    }
    uri.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::repo_root;

    fn sample_collection_path() -> String {
        repo_root()
            .join("examples/stac/sample-collection.json")
            .to_string_lossy()
            .into_owned()
    }

    fn sample_item_path() -> String {
        repo_root()
            .join("examples/stac/sample-item.json")
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn fetches_sample_stac_collection() {
        let collection = fetch_stac_collection(&sample_collection_path()).expect("collection");
        assert_eq!(collection.id, "genegis-sample-external");
        assert_eq!(collection.collection_type, "Collection");
    }

    #[test]
    fn fetches_sample_stac_item() {
        let item = fetch_stac_item(&sample_item_path()).expect("item");
        assert_eq!(item.id, "genegis-sample-external-item");
        assert!(item.assets.contains_key("data"));
    }

    #[test]
    fn imports_sample_stac_item_to_overlay() {
        let overlay_path = Path::new(CATALOG_OVERLAY_PATH);
        let backup = overlay_path.exists().then(|| std::fs::read(overlay_path).expect("read"));
        let _guard = RestoreOverlay { backup };

        let record = import_stac_item_url(&sample_item_path()).expect("import");
        assert_eq!(record.id, "genegis-sample-external-item");
        assert!(record.uri.ends_with("nagoya-wards.geojson"));

        let overlay = load_catalog_overlay();
        assert!(overlay.iter().any(|entry| entry.id == record.id));
    }

    struct RestoreOverlay {
        backup: Option<Vec<u8>>,
    }

    impl Drop for RestoreOverlay {
        fn drop(&mut self) {
            let path = Path::new(CATALOG_OVERLAY_PATH);
            match &self.backup {
                Some(bytes) => {
                    let _ = std::fs::write(path, bytes);
                }
                None => {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
    }
}
