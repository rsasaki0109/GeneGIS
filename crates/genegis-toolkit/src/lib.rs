//! General-purpose GIS toolkit for GeneGIS.
//!
//! Brings user data in (GeoJSON, CSV, Shapefile, GeoPackage, GeoParquet),
//! runs generic spatial operations as Workflow Graph nodes through the
//! Command boundary, plans those graphs from natural language, resolves
//! arbitrary places, inspects attributes, and exports results — with CRS,
//! units, sources, and provenance recorded at every step.

pub mod error;
pub mod estat;
pub mod execute;
pub mod export;
pub mod expr;
pub mod geojson_io;
#[cfg(feature = "native")]
pub mod geoparquet_io;
#[cfg(not(feature = "native"))]
#[path = "unavailable/geoparquet_io.rs"]
pub mod geoparquet_io;
#[cfg(feature = "native")]
pub mod gpkg_io;
#[cfg(not(feature = "native"))]
#[path = "unavailable/gpkg_io.rs"]
pub mod gpkg_io;
pub mod import;
pub mod index;
pub mod layer;
pub mod ops;
pub mod place;
pub mod planner;
pub mod proj;
pub mod samples;
pub mod store;
pub mod table;
pub mod wkb;
pub mod wkt;

/// Whether this build includes the `native` feature (GeoPackage, GeoParquet,
/// network fetches, LLM planner). Browser builds report `false`.
pub const NATIVE: bool = cfg!(feature = "native");

pub use error::{Result, ToolkitError};
pub use execute::{
    import_through_workflow, run_plan, ImportReceipt, PlanStep, RunReceipt, ToolkitPlan, ToolkitRun,
};
pub use export::{export, ExportFile, ExportFormat, MapOptions};
pub use import::{import_bytes, ImportFormat, ImportOptions, ImportReport};
pub use layer::{
    CrsStatus, Feature, Field, FieldType, GeometryKind, Layer, LayerProvenance, LayerSummary,
};
pub use place::{resolve_place, HttpFetcher, PlaceProvider, PlaceRequest};
pub use planner::{plan, LlmConfig, PlannerContext, PlannerMode, PlannerResult};
pub use store::{LayerStore, StoredSummary};
