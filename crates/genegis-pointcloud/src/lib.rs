//! GeneGIS point cloud engine — COPC read (Phase 4 alpha).

pub mod copc;
pub mod error;

mod http_source;
mod runtime;

pub use copc::{read_copc_bytes, read_copc_path, read_copc_uri, CopcInfo};
pub use error::PointcloudError;
