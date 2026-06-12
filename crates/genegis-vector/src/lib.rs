//! Vector data engine — GeoJSON read, GeoParquet read, feature model, attributes.

pub mod dataset;
pub mod error;
pub mod geojson;
pub mod geoparquet;
mod geometry;

pub use dataset::{FeatureRecord, VectorDataset};
pub use error::VectorError;
pub use geojson::{read_geojson_path, read_geojson_str};
pub use geoparquet::{read_geoparquet_bytes, read_geoparquet_path, read_geoparquet_uri};
