use thiserror::Error;

/// Errors raised by point cloud IO.
#[derive(Debug, Error)]
pub enum PointcloudError {
    /// COPC read failed.
    #[error("copc read error: {0}")]
    Copc(String),
}
