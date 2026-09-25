//! General-purpose GIS toolkit for GeneGIS.
//!
//! Brings user data in (GeoJSON, CSV, Shapefile, GeoPackage, GeoParquet),
//! runs generic spatial operations as Workflow Graph nodes through the
//! Command boundary, plans those graphs from natural language, resolves
//! arbitrary places, inspects attributes, and exports results — with CRS,
//! units, sources, and provenance recorded at every step.

pub mod error;
pub mod execute;
pub mod export;
pub mod expr;
pub mod geojson_io;
pub mod geoparquet_io;
pub mod gpkg_io;
pub mod import;
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
