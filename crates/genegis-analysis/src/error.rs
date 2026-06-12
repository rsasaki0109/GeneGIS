use thiserror::Error;
use genegis_vector::VectorError;

use crate::export::ExportError;

#[derive(Debug, Error)]
pub enum AnalysisError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Vector(#[from] VectorError),
    #[error(transparent)]
    Export(#[from] ExportError),
}
