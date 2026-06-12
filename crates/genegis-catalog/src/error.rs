use thiserror::Error;

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("dataset not found: {0}")]
    NotFound(String),
    #[error("no catalog dataset matches tags: {0:?}")]
    NoMatch(Vec<String>),
    #[error("ambiguous catalog match: {0:?}")]
    AmbiguousMatch(Vec<String>),
}
