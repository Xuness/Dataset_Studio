//! Provider-neutral inference data. No prompts, project data or transport DTOs.
mod configuration;
mod inference;
pub use configuration::*;
pub use inference::*;

pub const LLM_SCHEMA_VERSION: u32 = 1;
pub type LlmParameters = std::collections::BTreeMap<String, serde_json::Value>;
