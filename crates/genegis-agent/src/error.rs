use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Ai(#[from] genegis_ai::AiError),
    #[error(transparent)]
    Analysis(#[from] genegis_analysis::AnalysisError),
    #[error("agent JSON error: {0}")]
    Json(String),
}
