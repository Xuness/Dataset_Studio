use serde::{Deserialize, Serialize};
use studio_domain::llm as domain;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LlmSystemPromptConfig {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Literal system instructions, preserved without trimming or variable substitution.
    pub text: String,
}
impl From<domain::LlmSystemPromptConfig> for LlmSystemPromptConfig {
    fn from(v: domain::LlmSystemPromptConfig) -> Self {
        Self {
            name: v.name,
            description: v.description,
            text: v.text,
        }
    }
}
impl From<LlmSystemPromptConfig> for domain::LlmSystemPromptConfig {
    fn from(v: LlmSystemPromptConfig) -> Self {
        Self {
            name: v.name,
            description: v.description,
            text: v.text,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LlmSystemPrompt {
    pub id: String,
    pub revision: u64,
    pub config: LlmSystemPromptConfig,
}
impl From<domain::LlmSystemPrompt> for LlmSystemPrompt {
    fn from(v: domain::LlmSystemPrompt) -> Self {
        Self {
            id: v.id,
            revision: v.revision,
            config: v.config.into(),
        }
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveLlmSystemPrompt {
    pub id: Option<String>,
    pub expected_revision: u64,
    pub config: LlmSystemPromptConfig,
}

#[derive(Serialize, ToSchema)]
pub struct LlmSystemPrompts {
    pub items: Vec<LlmSystemPrompt>,
}
