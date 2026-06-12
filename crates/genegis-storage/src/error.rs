use thiserror::Error;

/// Errors raised by cloud and local asset IO.
#[derive(Debug, Error)]
pub enum StorageError {
    /// The URI scheme is not supported.
    #[error("unsupported URI scheme: {0}")]
    UnsupportedScheme(String),
    /// Local filesystem IO failed.
    #[error("local read failed: {0}")]
    Local(String),
    /// HTTP transport or response handling failed.
    #[error("HTTP read failed: {0}")]
    Http(String),
    /// Byte range bounds are invalid.
    #[error("invalid byte range: {0}")]
    InvalidRange(String),
}
