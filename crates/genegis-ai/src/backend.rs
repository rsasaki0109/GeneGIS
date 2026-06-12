use std::env;

/// Which planner backend to use for `genegis ask`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlannerBackend {
    #[default]
    RuleBased,
    Llm,
}

impl PlannerBackend {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_lowercase().as_str() {
            "rule" | "rules" | "rule-based" | "rule_based" => Some(Self::RuleBased),
            "llm" | "model" => Some(Self::Llm),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::RuleBased => "rule",
            Self::Llm => "llm",
        }
    }
}

/// Runtime configuration for workflow planning.
#[derive(Debug, Clone)]
pub struct PlannerConfig {
    pub backend: PlannerBackend,
    pub llm_api_key: Option<String>,
    pub llm_base_url: String,
    pub llm_model: String,
    /// When LLM fails, retry with the rule-based resolver.
    pub fallback_to_rules: bool,
}

impl Default for PlannerConfig {
    fn default() -> Self {
        Self {
            backend: PlannerBackend::RuleBased,
            llm_api_key: env::var("GENEGIS_LLM_API_KEY")
                .ok()
                .filter(|value| !value.trim().is_empty()),
            llm_base_url: env::var("GENEGIS_LLM_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".into()),
            llm_model: env::var("GENEGIS_LLM_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into()),
            fallback_to_rules: true,
        }
    }
}

impl PlannerConfig {
    pub fn with_backend(mut self, backend: PlannerBackend) -> Self {
        self.backend = backend;
        self
    }

    pub fn llm_ready(&self) -> bool {
        self.llm_api_key
            .as_ref()
            .is_some_and(|key| !key.trim().is_empty())
    }
}
