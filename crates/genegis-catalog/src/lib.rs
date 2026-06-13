//! GeneGIS catalog — dataset metadata registry (Phase 2 alpha).

pub mod catalog;
pub mod dataset;
pub mod error;
pub mod external_stac;
pub mod lookup;
pub mod stac;

pub use catalog::{
    alpha_catalog, extended_catalog, nagoya_wards_geojson_path, nagoya_wards_geoparquet_path,
    repo_root, Catalog, LOCAL_COG_DEMO_ID, NAGOYA_WARDS_DENSITY_ID, NAGOYA_WARDS_GEOPARQUET_ID,
    REMOTE_COG_DEMO_ID,
};
pub use external_stac::{
    fetch_stac_collection, fetch_stac_item, import_stac_item_url, load_catalog_overlay,
    CATALOG_OVERLAY_PATH,
};
pub use dataset::{DatasetFormat, DatasetRecord};
pub use error::CatalogError;
pub use lookup::CatalogMatch;
pub use stac::{
    bind_stac_item, browse_alpha_stac_collection, StacAsset, StacCollection, StacItem, StacLink,
    ALPHA_STAC_COLLECTION_ID,
};
