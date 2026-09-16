use super::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmRole {
    System,
    Developer,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum LlmContent {
    Text {
        text: String,
    },
    Image {
        url: String,
        detail: Option<String>,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: Value,
        signature: Option<String>,
    },
    ToolResult {
        id: String,
        name: String,
        value: Value,
    },
    Reasoning {
        text: String,
    },
    Refusal {
        text: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmMessage {
    pub role: LlmRole,
    pub content: Vec<LlmContent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmTool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub strict: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmInvocationRequest {
    pub invocation_id: String,
    pub model_id: String,
    pub expected_model_revision: Option<u64>,
    pub expected_provider_revision: Option<u64>,
    pub preset_id: Option<String>,
    pub expected_preset_revision: Option<u64>,
    #[serde(default)]
    pub overrides: LlmParameters,
    pub messages: Vec<LlmMessage>,
    #[serde(default)]
    pub tools: Vec<LlmTool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub parameters: LlmParameters,
    pub messages: Vec<LlmMessage>,
    pub tools: Vec<LlmTool>,
    pub warnings: Vec<String>,
}

/// Internal execution plan. Kept separate from the public, credential-free snapshot.
#[derive(Clone)]
pub struct LlmInvocationPlan {
    pub provider: LlmProvider,
    pub snapshot: LlmInvocationSnapshot,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmOutput {
    pub index: u32,
    pub content: Vec<LlmContent>,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    pub snapshot: LlmInvocationSnapshot,
    pub provider_request_id: Option<String>,
    pub response_id: Option<String>,
    pub model: Option<String>,
    pub outputs: Vec<LlmOutput>,
    pub usage: LlmUsage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmFailure {
    pub code: String,
    pub message: String,
    pub http_status: Option<u16>,
    pub provider_request_id: Option<String>,
    pub retryable: bool,
    /// The upstream may have accepted the request; automatic replay is unsafe.
    pub outcome_unknown: bool,
}
impl LlmFailure {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            http_status: None,
            provider_request_id: None,
            retryable: false,
            outcome_unknown: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LlmEvent {
    Started {
        invocation_id: String,
    },
    Delta {
        index: u32,
        kind: String,
        text: String,
        tool_call_id: Option<String>,
    },
    Completed {
        response: Box<LlmResponse>,
    },
    Failed {
        error: LlmFailure,
    },
}
