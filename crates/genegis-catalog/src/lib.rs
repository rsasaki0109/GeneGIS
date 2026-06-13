//! GeneGIS catalog — dataset metadata registry (Phase 2 alpha).

pub mod catalog;
pub mod dataset;
pub mod error;
pub mod lookup;
pub mod stac;

pub use catalog::{
    alpha_catalog, nagoya_wards_geojson_path, Catalog, LOCAL_COG_DEMO_ID, NAGOYA_WARDS_DENSITY_ID,
    REMOTE_COG_DEMO_ID,
};
pub use dataset::{DatasetFormat, DatasetRecord};
pub use error::CatalogError;
pub use lookup::CatalogMatch;
pub use stac::{
    bind_stac_item, browse_alpha_stac_collection, StacAsset, StacCollection, StacItem, StacLink,
    ALPHA_STAC_COLLECTION_ID,
};
