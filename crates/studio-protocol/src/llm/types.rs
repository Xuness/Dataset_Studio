use serde::{Deserialize, Serialize};
use serde_json::Value;
use studio_domain::llm as domain;
use utoipa::ToSchema;
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LlmProtocol {
    OpenaiChat,
    OpenaiResponses,
    Gemini,
}
impl From<domain::LlmProtocol> for LlmProtocol {
    fn from(v: domain::LlmProtocol) -> Self {
        match v {
            domain::LlmProtocol::OpenaiChat => Self::OpenaiChat,
            domain::LlmProtocol::OpenaiResponses => Self::OpenaiResponses,
            domain::LlmProtocol::Gemini => Self::Gemini,
        }
    }
}
impl From<LlmProtocol> for domain::LlmProtocol {
    fn from(v: LlmProtocol) -> Self {
        match v {
            LlmProtocol::OpenaiChat => Self::OpenaiChat,
            LlmProtocol::OpenaiResponses => Self::OpenaiResponses,
            LlmProtocol::Gemini => Self::Gemini,
        }
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LlmProviderKind {
    OpenaiCompatible,
    Openai,
    Openrouter,
    Gemini,
}
impl From<domain::LlmProviderKind> for LlmProviderKind {
    fn from(v: domain::LlmProviderKind) -> Self {
        match v {
            domain::LlmProviderKind::OpenaiCompatible => Self::OpenaiCompatible,
            domain::LlmProviderKind::Openai => Self::Openai,
            domain::LlmProviderKind::Openrouter => Self::Openrouter,
            domain::LlmProviderKind::Gemini => Self::Gemini,
        }
    }
}
impl From<LlmProviderKind> for domain::LlmProviderKind {
    fn from(v: LlmProviderKind) -> Self {
        match v {
            LlmProviderKind::OpenaiCompatible => Self::OpenaiCompatible,
            LlmProviderKind::Openai => Self::Openai,
            LlmProviderKind::Openrouter => Self::Openrouter,
            LlmProviderKind::Gemini => Self::Gemini,
        }
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LlmSupport {
    Supported,
    Unsupported,
    Unknown,
}
impl From<domain::LlmSupport> for LlmSupport {
    fn from(v: domain::LlmSupport) -> Self {
        match v {
            domain::LlmSupport::Supported => Self::Supported,
            domain::LlmSupport::Unsupported => Self::Unsupported,
            domain::LlmSupport::Unknown => Self::Unknown,
        }
    }
}
impl From<LlmSupport> for domain::LlmSupport {
    fn from(v: LlmSupport) -> Self {
        match v {
            LlmSupport::Supported => Self::Supported,
            LlmSupport::Unsupported => Self::Unsupported,
            LlmSupport::Unknown => Self::Unknown,
        }
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LlmRole {
    System,
    Developer,
    User,
    Assistant,
    Tool,
}
impl From<domain::LlmRole> for LlmRole {
    fn from(v: domain::LlmRole) -> Self {
        match v {
            domain::LlmRole::System => Self::System,
            domain::LlmRole::Developer => Self::Developer,
            domain::LlmRole::User => Self::User,
            domain::LlmRole::Assistant => Self::Assistant,
            domain::LlmRole::Tool => Self::Tool,
        }
    }
}
impl From<LlmRole> for domain::LlmRole {
    fn from(v: LlmRole) -> Self {
        match v {
            LlmRole::System => Self::System,
            LlmRole::Developer => Self::Developer,
            LlmRole::User => Self::User,
            LlmRole::Assistant => Self::Assistant,
            LlmRole::Tool => Self::Tool,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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
impl From<domain::LlmContent> for LlmContent {
    fn from(v: domain::LlmContent) -> Self {
        match v {
            domain::LlmContent::Text { text } => Self::Text { text },
            domain::LlmContent::Image { url, detail } => Self::Image { url, detail },
            domain::LlmContent::ToolCall {
                id,
                name,
                arguments,
                signature,
            } => Self::ToolCall {
                id,
                name,
                arguments,
                signature,
            },
            domain::LlmContent::ToolResult { id, name, value } => {
                Self::ToolResult { id, name, value }
            }
            domain::LlmContent::Reasoning { text } => Self::Reasoning { text },
            domain::LlmContent::Refusal { text } => Self::Refusal { text },
        }
    }
}
impl From<LlmContent> for domain::LlmContent {
    fn from(v: LlmContent) -> Self {
        match v {
            LlmContent::Text { text } => Self::Text { text },
            LlmContent::Image { url, detail } => Self::Image { url, detail },
            LlmContent::ToolCall {
                id,
                name,
                arguments,
                signature,
            } => Self::ToolCall {
                id,
                name,
                arguments,
                signature,
            },
            LlmContent::ToolResult { id, name, value } => Self::ToolResult { id, name, value },
            LlmContent::Reasoning { text } => Self::Reasoning { text },
            LlmContent::Refusal { text } => Self::Refusal { text },
        }
    }
}
