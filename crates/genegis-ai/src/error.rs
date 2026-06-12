use thiserror::Error;

#[derive(Debug, Error)]
pub enum AiError {
    #[error("empty prompt")]
    EmptyPrompt,
    #[error("could not resolve intent: {0}")]
    Unresolved(String),
    #[error("ambiguous intent: {0}")]
    Ambiguous(String),
    #[error("LLM configuration error: {0}")]
    LlmConfig(String),
    #[error("LLM transport error: {0}")]
    LlmTransport(String),
    #[error("LLM response error: {0}")]
    LlmResponse(String),
}
