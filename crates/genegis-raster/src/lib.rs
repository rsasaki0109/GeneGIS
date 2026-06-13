//! GeneGIS raster engine — COG / GeoTIFF read (Phase 3 alpha).

pub mod cog;
pub mod error;

pub use cog::{
    read_cog_bytes, read_cog_path, read_cog_uri, read_cog_uri_with_options, read_cog_window_u8,
    read_cog_window_uri, smoke_demo_cog_path, CogInfo,
};
pub use geotiff_reader::cog::HttpOpenOptions;
pub use error::RasterError;
