use super::LlmParameters;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmProtocol {
    OpenaiChat,
    OpenaiResponses,
    Gemini,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmProviderKind {
    OpenaiCompatible,
    Openai,
    Openrouter,
    Gemini,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmSupport {
    Supported,
    Unsupported,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmNetworkSettings {
    pub connect_timeout_ms: u32,
    pub request_timeout_ms: u32,
    pub idle_timeout_ms: u32,
    pub max_concurrency: u32,
    pub min_interval_ms: u32,
    /// Only explicit HTTP 429 responses may be retried for inference.
    pub rate_limit_retries: u32,
    /// None uses the operating environment; empty string disables proxy use.
    pub proxy_url: Option<String>,
}
impl Default for LlmNetworkSettings {
    fn default() -> Self {
        Self {
            connect_timeout_ms: 15_000,
            request_timeout_ms: 180_000,
            idle_timeout_ms: 60_000,
            max_concurrency: 2,
            min_interval_ms: 0,
            rate_limit_retries: 0,
            proxy_url: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmConnectionConfig {
    pub name: String,
    pub kind: LlmProviderKind,
    pub base_url: String,
    pub enabled: bool,
    #[serde(default)]
    pub network: LlmNetworkSettings,
    /// Non-secret headers only. Authentication is exclusively supplied by the vault.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmProvider {
    pub id: String,
    pub revision: u64,
    pub config: LlmConnectionConfig,
    pub credential_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmModelConfig {
    pub name: String,
    pub remote_model_id: String,
    pub protocol: LlmProtocol,
    pub enabled: bool,
    #[serde(default)]
    pub parameters: LlmParameters,
    /// Explicit user observations, never inferred from a model's name.
    #[serde(default)]
    pub capability_overrides: BTreeMap<String, LlmSupport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmModel {
    pub id: String,
    pub provider_id: String,
    pub revision: u64,
    pub config: LlmModelConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmPresetConfig {
    pub name: String,
    pub protocol: LlmProtocol,
    pub parameters: LlmParameters,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmPreset {
    pub id: String,
    pub revision: u64,
    pub config: LlmPresetConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCatalogModel {
    pub id: String,
    pub name: String,
    pub input_token_limit: Option<u64>,
    pub output_token_limit: Option<u64>,
    pub input_modalities: Vec<String>,
    pub output_modalities: Vec<String>,
    pub capabilities: BTreeMap<String, LlmSupport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCatalog {
    pub provider_id: String,
    pub provider_revision: u64,
    pub fetched_at: String,
    pub models: Vec<LlmCatalogModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmParameterSpec {
    pub key: String,
    pub label: String,
    pub description: String,
    pub value_type: String,
    pub group: String,
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub choices: Vec<String>,
    pub support: LlmSupport,
    pub evidence: String,
}
