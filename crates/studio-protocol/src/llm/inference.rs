use super::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use studio_domain::llm as domain;
use utoipa::ToSchema;
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LlmMessage {
    pub role: LlmRole,
    pub content: Vec<LlmContent>,
}
impl From<domain::LlmMessage> for LlmMessage {
    fn from(v: domain::LlmMessage) -> Self {
        Self {
            role: v.role.into(),
            content: v.content.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<LlmMessage> for domain::LlmMessage {
    fn from(v: LlmMessage) -> Self {
        Self {
            role: v.role.into(),
            content: v.content.into_iter().map(Into::into).collect(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LlmTool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub strict: bool,
}
impl From<domain::LlmTool> for LlmTool {
    fn from(v: domain::LlmTool) -> Self {
        Self {
            name: v.name,
            description: v.description,
            parameters: v.parameters,
            strict: v.strict,
        }
    }
}
impl From<LlmTool> for domain::LlmTool {
    fn from(v: LlmTool) -> Self {
        Self {
            name: v.name,
            description: v.description,
            parameters: v.parameters,
            strict: v.strict,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LlmInvocationRequest {
    pub invocation_id: String,
    pub model_id: String,
    pub expected_model_revision: Option<u64>,
    pub expected_provider_revision: Option<u64>,
    pub preset_id: Option<String>,
    pub expected_preset_revision: Option<u64>,
    #[serde(default)]
    pub overrides: BTreeMap<String, Value>,
    pub messages: Vec<LlmMessage>,
    #[serde(default)]
    pub tools: Vec<LlmTool>,
}
impl From<domain::LlmInvocationRequest> for LlmInvocationRequest {
    fn from(v: domain::LlmInvocationRequest) -> Self {
        Self {
            invocation_id: v.invocation_id,
            model_id: v.model_id,
            expected_model_revision: v.expected_model_revision,
            expected_provider_revision: v.expected_provider_revision,
            preset_id: v.preset_id,
            expected_preset_revision: v.expected_preset_revision,
            overrides: v.overrides,
            messages: v.messages.into_iter().map(Into::into).collect(),
            tools: v.tools.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<LlmInvocationRequest> for domain::LlmInvocationRequest {
    fn from(v: LlmInvocationRequest) -> Self {
        Self {
            invocation_id: v.invocation_id,
            model_id: v.model_id,
            expected_model_revision: v.expected_model_revision,
            expected_provider_revision: v.expected_provider_revision,
            preset_id: v.preset_id,
            expected_preset_revision: v.expected_preset_revision,
            overrides: v.overrides,
            messages: v.messages.into_iter().map(Into::into).collect(),
            tools: v.tools.into_iter().map(Into::into).collect(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LlmInvocationSnapshot {
    pub schema_version: u32,
    pub invocation_id: String,
    pub provider_id: String,
    pub provider_revision: u64,
    pub provider_kind: LlmProviderKind,
    pub base_url: String,
    pub model_id: String,
    pub model_revision: u64,
    pub remote_model_id: String,
    pub protocol: LlmProtocol,
    pub preset_id: Option<String>,
    pub preset_revision: Option<u64>,
    #[serde(default)]
    pub parameters: BTreeMap<String, Value>,
    pub messages: Vec<LlmMessage>,
    #[serde(default)]
    pub tools: Vec<LlmTool>,
    pub warnings: Vec<String>,
}
impl From<domain::LlmInvocationSnapshot> for LlmInvocationSnapshot {
    fn from(v: domain::LlmInvocationSnapshot) -> Self {
        Self {
            schema_version: v.schema_version,
            invocation_id: v.invocation_id,
            provider_id: v.provider_id,
            provider_revision: v.provider_revision,
            provider_kind: v.provider_kind.into(),
            base_url: v.base_url,
            model_id: v.model_id,
            model_revision: v.model_revision,
            remote_model_id: v.remote_model_id,
            protocol: v.protocol.into(),
            preset_id: v.preset_id,
            preset_revision: v.preset_revision,
            parameters: v.parameters,
            messages: v.messages.into_iter().map(Into::into).collect(),
            tools: v.tools.into_iter().map(Into::into).collect(),
            warnings: v.warnings,
        }
    }
}
impl From<LlmInvocationSnapshot> for domain::LlmInvocationSnapshot {
    fn from(v: LlmInvocationSnapshot) -> Self {
        Self {
            schema_version: v.schema_version,
            invocation_id: v.invocation_id,
            provider_id: v.provider_id,
            provider_revision: v.provider_revision,
            provider_kind: v.provider_kind.into(),
            base_url: v.base_url,
            model_id: v.model_id,
            model_revision: v.model_revision,
            remote_model_id: v.remote_model_id,
            protocol: v.protocol.into(),
            preset_id: v.preset_id,
            preset_revision: v.preset_revision,
            parameters: v.parameters,
            messages: v.messages.into_iter().map(Into::into).collect(),
            tools: v.tools.into_iter().map(Into::into).collect(),
            warnings: v.warnings,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LlmUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
}
impl From<domain::LlmUsage> for LlmUsage {
    fn from(v: domain::LlmUsage) -> Self {
        Self {
            input_tokens: v.input_tokens,
            output_tokens: v.output_tokens,
            total_tokens: v.total_tokens,
            cached_input_tokens: v.cached_input_tokens,
            reasoning_tokens: v.reasoning_tokens,
        }
    }
}
impl From<LlmUsage> for domain::LlmUsage {
    fn from(v: LlmUsage) -> Self {
        Self {
            input_tokens: v.input_tokens,
            output_tokens: v.output_tokens,
            total_tokens: v.total_tokens,
            cached_input_tokens: v.cached_input_tokens,
            reasoning_tokens: v.reasoning_tokens,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LlmOutput {
    pub index: u32,
    pub content: Vec<LlmContent>,
    pub finish_reason: Option<String>,
}
impl From<domain::LlmOutput> for LlmOutput {
    fn from(v: domain::LlmOutput) -> Self {
        Self {
            index: v.index,
            content: v.content.into_iter().map(Into::into).collect(),
            finish_reason: v.finish_reason,
        }
    }
}
impl From<LlmOutput> for domain::LlmOutput {
    fn from(v: LlmOutput) -> Self {
        Self {
            index: v.index,
            content: v.content.into_iter().map(Into::into).collect(),
            finish_reason: v.finish_reason,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LlmResponse {
    pub snapshot: LlmInvocationSnapshot,
    pub provider_request_id: Option<String>,
    pub response_id: Option<String>,
    pub model: Option<String>,
    pub outputs: Vec<LlmOutput>,
    pub usage: LlmUsage,
}
impl From<domain::LlmResponse> for LlmResponse {
    fn from(v: domain::LlmResponse) -> Self {
        Self {
            snapshot: v.snapshot.into(),
            provider_request_id: v.provider_request_id,
            response_id: v.response_id,
            model: v.model,
            outputs: v.outputs.into_iter().map(Into::into).collect(),
            usage: v.usage.into(),
        }
    }
}
impl From<LlmResponse> for domain::LlmResponse {
    fn from(v: LlmResponse) -> Self {
        Self {
            snapshot: v.snapshot.into(),
            provider_request_id: v.provider_request_id,
            response_id: v.response_id,
            model: v.model,
            outputs: v.outputs.into_iter().map(Into::into).collect(),
            usage: v.usage.into(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LlmFailure {
    pub code: String,
    pub message: String,
    pub http_status: Option<u16>,
    pub provider_request_id: Option<String>,
    pub retryable: bool,
    pub outcome_unknown: bool,
}
impl From<domain::LlmFailure> for LlmFailure {
    fn from(v: domain::LlmFailure) -> Self {
        Self {
            code: v.code,
            message: v.message,
            http_status: v.http_status,
            provider_request_id: v.provider_request_id,
            retryable: v.retryable,
            outcome_unknown: v.outcome_unknown,
        }
    }
}
impl From<LlmFailure> for domain::LlmFailure {
    fn from(v: LlmFailure) -> Self {
        Self {
            code: v.code,
            message: v.message,
            http_status: v.http_status,
            provider_request_id: v.provider_request_id,
            retryable: v.retryable,
            outcome_unknown: v.outcome_unknown,
        }
    }
}
