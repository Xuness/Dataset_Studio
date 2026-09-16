use super::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use studio_domain::llm as domain;
use utoipa::ToSchema;
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LlmNetworkSettings {
    pub connect_timeout_ms: u32,
    pub request_timeout_ms: u32,
    pub idle_timeout_ms: u32,
    pub max_concurrency: u32,
    pub min_interval_ms: u32,
    pub rate_limit_retries: u32,
    pub proxy_url: Option<String>,
}
impl From<domain::LlmNetworkSettings> for LlmNetworkSettings {
    fn from(v: domain::LlmNetworkSettings) -> Self {
        Self {
            connect_timeout_ms: v.connect_timeout_ms,
            request_timeout_ms: v.request_timeout_ms,
            idle_timeout_ms: v.idle_timeout_ms,
            max_concurrency: v.max_concurrency,
            min_interval_ms: v.min_interval_ms,
            rate_limit_retries: v.rate_limit_retries,
            proxy_url: v.proxy_url,
        }
    }
}
impl From<LlmNetworkSettings> for domain::LlmNetworkSettings {
    fn from(v: LlmNetworkSettings) -> Self {
        Self {
            connect_timeout_ms: v.connect_timeout_ms,
            request_timeout_ms: v.request_timeout_ms,
            idle_timeout_ms: v.idle_timeout_ms,
            max_concurrency: v.max_concurrency,
            min_interval_ms: v.min_interval_ms,
            rate_limit_retries: v.rate_limit_retries,
            proxy_url: v.proxy_url,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LlmConnectionConfig {
    pub name: String,
    pub kind: LlmProviderKind,
    pub base_url: String,
    pub enabled: bool,
    pub network: LlmNetworkSettings,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}
impl From<domain::LlmConnectionConfig> for LlmConnectionConfig {
    fn from(v: domain::LlmConnectionConfig) -> Self {
        Self {
            name: v.name,
            kind: v.kind.into(),
            base_url: v.base_url,
            enabled: v.enabled,
            network: v.network.into(),
            headers: v.headers,
        }
    }
}
impl From<LlmConnectionConfig> for domain::LlmConnectionConfig {
    fn from(v: LlmConnectionConfig) -> Self {
        Self {
            name: v.name,
            kind: v.kind.into(),
            base_url: v.base_url,
            enabled: v.enabled,
            network: v.network.into(),
            headers: v.headers,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LlmModelConfig {
    pub name: String,
    pub remote_model_id: String,
    pub protocol: LlmProtocol,
    pub enabled: bool,
    #[serde(default)]
    pub parameters: BTreeMap<String, Value>,
    #[serde(default)]
    pub capability_overrides: BTreeMap<String, LlmSupport>,
}
impl From<domain::LlmModelConfig> for LlmModelConfig {
    fn from(v: domain::LlmModelConfig) -> Self {
        Self {
            name: v.name,
            remote_model_id: v.remote_model_id,
            protocol: v.protocol.into(),
            enabled: v.enabled,
            parameters: v.parameters,
            capability_overrides: v
                .capability_overrides
                .into_iter()
                .map(|(k, v)| (k, v.into()))
                .collect(),
        }
    }
}
impl From<LlmModelConfig> for domain::LlmModelConfig {
    fn from(v: LlmModelConfig) -> Self {
        Self {
            name: v.name,
            remote_model_id: v.remote_model_id,
            protocol: v.protocol.into(),
            enabled: v.enabled,
            parameters: v.parameters,
            capability_overrides: v
                .capability_overrides
                .into_iter()
                .map(|(k, v)| (k, v.into()))
                .collect(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LlmModel {
    pub id: String,
    pub provider_id: String,
    pub revision: u64,
    pub config: LlmModelConfig,
}
impl From<domain::LlmModel> for LlmModel {
    fn from(v: domain::LlmModel) -> Self {
        Self {
            id: v.id,
            provider_id: v.provider_id,
            revision: v.revision,
            config: v.config.into(),
        }
    }
}
impl From<LlmModel> for domain::LlmModel {
    fn from(v: LlmModel) -> Self {
        Self {
            id: v.id,
            provider_id: v.provider_id,
            revision: v.revision,
            config: v.config.into(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LlmPresetConfig {
    pub name: String,
    pub protocol: LlmProtocol,
    #[serde(default)]
    pub parameters: BTreeMap<String, Value>,
}
impl From<domain::LlmPresetConfig> for LlmPresetConfig {
    fn from(v: domain::LlmPresetConfig) -> Self {
        Self {
            name: v.name,
            protocol: v.protocol.into(),
            parameters: v.parameters,
        }
    }
}
impl From<LlmPresetConfig> for domain::LlmPresetConfig {
    fn from(v: LlmPresetConfig) -> Self {
        Self {
            name: v.name,
            protocol: v.protocol.into(),
            parameters: v.parameters,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LlmPreset {
    pub id: String,
    pub revision: u64,
    pub config: LlmPresetConfig,
}
impl From<domain::LlmPreset> for LlmPreset {
    fn from(v: domain::LlmPreset) -> Self {
        Self {
            id: v.id,
            revision: v.revision,
            config: v.config.into(),
        }
    }
}
impl From<LlmPreset> for domain::LlmPreset {
    fn from(v: LlmPreset) -> Self {
        Self {
            id: v.id,
            revision: v.revision,
            config: v.config.into(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LlmCatalogModel {
    pub id: String,
    pub name: String,
    pub input_token_limit: Option<u64>,
    pub output_token_limit: Option<u64>,
    pub input_modalities: Vec<String>,
    pub output_modalities: Vec<String>,
    pub capabilities: BTreeMap<String, LlmSupport>,
}
impl From<domain::LlmCatalogModel> for LlmCatalogModel {
    fn from(v: domain::LlmCatalogModel) -> Self {
        Self {
            id: v.id,
            name: v.name,
            input_token_limit: v.input_token_limit,
            output_token_limit: v.output_token_limit,
            input_modalities: v.input_modalities,
            output_modalities: v.output_modalities,
            capabilities: v
                .capabilities
                .into_iter()
                .map(|(k, v)| (k, v.into()))
                .collect(),
        }
    }
}
impl From<LlmCatalogModel> for domain::LlmCatalogModel {
    fn from(v: LlmCatalogModel) -> Self {
        Self {
            id: v.id,
            name: v.name,
            input_token_limit: v.input_token_limit,
            output_token_limit: v.output_token_limit,
            input_modalities: v.input_modalities,
            output_modalities: v.output_modalities,
            capabilities: v
                .capabilities
                .into_iter()
                .map(|(k, v)| (k, v.into()))
                .collect(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LlmCatalog {
    pub provider_id: String,
    pub provider_revision: u64,
    pub fetched_at: String,
    pub models: Vec<LlmCatalogModel>,
}
impl From<domain::LlmCatalog> for LlmCatalog {
    fn from(v: domain::LlmCatalog) -> Self {
        Self {
            provider_id: v.provider_id,
            provider_revision: v.provider_revision,
            fetched_at: v.fetched_at,
            models: v.models.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<LlmCatalog> for domain::LlmCatalog {
    fn from(v: LlmCatalog) -> Self {
        Self {
            provider_id: v.provider_id,
            provider_revision: v.provider_revision,
            fetched_at: v.fetched_at,
            models: v.models.into_iter().map(Into::into).collect(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
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
impl From<domain::LlmParameterSpec> for LlmParameterSpec {
    fn from(v: domain::LlmParameterSpec) -> Self {
        Self {
            key: v.key,
            label: v.label,
            description: v.description,
            value_type: v.value_type,
            group: v.group,
            minimum: v.minimum,
            maximum: v.maximum,
            choices: v.choices,
            support: v.support.into(),
            evidence: v.evidence,
        }
    }
}
impl From<LlmParameterSpec> for domain::LlmParameterSpec {
    fn from(v: LlmParameterSpec) -> Self {
        Self {
            key: v.key,
            label: v.label,
            description: v.description,
            value_type: v.value_type,
            group: v.group,
            minimum: v.minimum,
            maximum: v.maximum,
            choices: v.choices,
            support: v.support.into(),
            evidence: v.evidence,
        }
    }
}
