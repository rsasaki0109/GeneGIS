use thiserror::Error;

/// Fail-closed errors raised by the general-purpose GIS toolkit.
#[derive(Debug, Error)]
pub enum ToolkitError {
    /// The input bytes could not be decoded in the declared format.
    #[error("{format} import failed: {reason}")]
    Import {
        /// Format name, for example `geojson` or `shapefile`.
        format: &'static str,
        /// Human-readable explanation.
        reason: String,
    },
    /// The file format could not be recognised from its name or contents.
    #[error("unsupported file format: {0}")]
    UnsupportedFormat(String),
    /// The CRS is missing and cannot be inferred safely; the caller must
    /// supply one explicitly.
    #[error("CRS is required: {0}")]
    CrsRequired(String),
    /// The CRS is known by name but GeneGIS cannot transform it.
    #[error("unsupported CRS {0}")]
    UnsupportedCrs(String),
    /// A coordinate is outside the CRS domain or not finite.
    #[error("invalid coordinate: {0}")]
    InvalidCoordinate(String),
    /// A unit is missing or incompatible with the requested measurement.
    #[error("unit error: {0}")]
    Unit(String),
    /// An operation parameter is missing or invalid.
    #[error("invalid parameter for {operation}: {reason}")]
    Parameter {
        /// Toolkit operation name.
        operation: String,
        /// Human-readable explanation.
        reason: String,
    },
    /// A referenced layer or step does not exist.
    #[error("unknown reference: {0}")]
    UnknownReference(String),
    /// The plan cannot be converted into a valid workflow graph.
    #[error("invalid plan: {0}")]
    Plan(String),
    /// An independent verification check failed.
    #[error("verification failed: {0}")]
    Verification(String),
    /// A filter expression could not be parsed or evaluated.
    #[error("invalid expression: {0}")]
    Expression(String),
    /// A remote provider failed or returned unusable data.
    #[error("provider error: {0}")]
    Provider(String),
    /// A planner could not turn a prompt into a plan.
    #[error("planner could not resolve the request: {0}")]
    Unresolved(String),
    /// Export failed.
    #[error("{format} export failed: {reason}")]
    Export {
        /// Format name.
        format: &'static str,
        /// Human-readable explanation.
        reason: String,
    },
    /// The Command + Workflow boundary rejected the run.
    #[error("command rejected: {0}")]
    Command(String),
    /// Filesystem error.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl ToolkitError {
    pub(crate) fn import(format: &'static str, reason: impl Into<String>) -> Self {
        Self::Import {
            format,
            reason: reason.into(),
        }
    }

    pub(crate) fn export(format: &'static str, reason: impl Into<String>) -> Self {
        Self::Export {
            format,
            reason: reason.into(),
        }
    }

    /// Invalid or missing parameter for an operation or tool.
    pub fn parameter(operation: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Parameter {
            operation: operation.into(),
            reason: reason.into(),
        }
    }
}

/// Toolkit result alias.
pub type Result<T> = std::result::Result<T, ToolkitError>;
