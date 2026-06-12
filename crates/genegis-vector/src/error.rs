use thiserror::Error;

#[derive(Debug, Error)]
pub enum VectorError {
    #[error("geojson parse error: {0}")]
    GeoJson(String),
    #[error("geoparquet read error: {0}")]
    GeoParquet(String),
    #[error("unsupported geometry: {0}")]
    UnsupportedGeometry(String),
    #[error("missing property: {0}")]
    MissingProperty(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
