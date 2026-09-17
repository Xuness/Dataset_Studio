//! Provider-neutral inference and configuration data. No project data or transport DTOs.
mod configuration;
mod inference;
mod system_prompts;
pub use configuration::*;
pub use inference::*;
pub use system_prompts::*;

pub const LLM_SCHEMA_VERSION: u32 = 2;
pub type LlmParameters = std::collections::BTreeMap<String, serde_json::Value>;
