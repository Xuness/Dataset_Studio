use serde::{Deserialize, Serialize};

/// Literal, reusable system instructions. Task-specific user input is never stored here.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmSystemPromptConfig {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmSystemPrompt {
    pub id: String,
    pub revision: u64,
    pub config: LlmSystemPromptConfig,
}
