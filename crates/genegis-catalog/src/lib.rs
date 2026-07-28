//! GeneGIS catalog — dataset metadata registry (Phase 2 alpha).

pub mod catalog;
pub mod dataset;
pub mod error;
pub mod external_stac;
pub mod federated;
pub mod lookup;
pub mod registry;
pub mod stac;

pub use catalog::{
    alpha_catalog, extended_catalog, nagoya_wards_geojson_path, nagoya_wards_geoparquet_path,
    repo_root, Catalog, EXTERNAL_STAC_DEMO_ID, LOCAL_COG_DEMO_ID, NAGOYA_WARDS_DENSITY_ID,
    NAGOYA_WARDS_GEOPARQUET_ID, REMOTE_COG_DEMO_ID,
};
pub use dataset::{DatasetFormat, DatasetRecord};
pub use error::CatalogError;
pub use external_stac::{
    catalog_overlay_path, fetch_stac_collection, fetch_stac_item, import_stac_item_url,
    load_catalog_overlay, resolve_catalog_url, CATALOG_OVERLAY_ENV, CATALOG_OVERLAY_PATH,
};
pub use federated::{
    AssetBindingReceipt, AssetCandidate, AssetRequirements, AssetVerification,
    EndpointSearchOutcome, FederatedCatalog, FederatedSearchResult, FederatedStacItem,
    StacAuthentication, StacEndpoint, StacItemCollection, StacSearchRequest,
};
pub use lookup::CatalogMatch;
pub use registry::{
    endpoint_registry_path, EndpointRegistry, ENDPOINT_REGISTRY_ENV, ENDPOINT_REGISTRY_PATH,
};
pub use stac::{
    bind_stac_item, browse_alpha_stac_collection, StacAsset, StacCollection, StacItem, StacLink,
    ALPHA_STAC_COLLECTION_ID,
};
