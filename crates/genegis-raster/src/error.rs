use thiserror::Error;

#[derive(Debug, Error)]
pub enum RasterError {
    #[error("cog read error: {0}")]
    Cog(String),
    #[error("invalid raster operation: {0}")]
    Invalid(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
