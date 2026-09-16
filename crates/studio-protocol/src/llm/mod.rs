mod configuration;
mod inference;
mod types;
pub use configuration::*;
pub use inference::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub use types::*;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct LlmProviderView {
    pub id: String,
    pub revision: u64,
    pub config: LlmConnectionConfig,
    pub credential_set: bool,
}
impl From<studio_domain::llm::LlmProvider> for LlmProviderView {
    fn from(v: studio_domain::llm::LlmProvider) -> Self {
        Self {
            id: v.id,
            revision: v.revision,
            config: v.config.into(),
            credential_set: v.credential_ref.is_some(),
        }
    }
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveLlmProvider {
    pub id: Option<String>,
    pub expected_revision: u64,
    pub config: LlmConnectionConfig,
    pub api_key: Option<String>,
    #[serde(default)]
    pub clear_credential: bool,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveLlmModel {
    pub id: Option<String>,
    pub provider_id: String,
    pub expected_revision: u64,
    pub config: LlmModelConfig,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveLlmPreset {
    pub id: Option<String>,
    pub expected_revision: u64,
    pub config: LlmPresetConfig,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LlmRevision {
    pub expected_revision: u64,
}
#[derive(Serialize, ToSchema)]
pub struct LlmProviders {
    pub items: Vec<LlmProviderView>,
}
#[derive(Serialize, ToSchema)]
pub struct LlmModels {
    pub items: Vec<LlmModel>,
}
#[derive(Serialize, ToSchema)]
pub struct LlmPresets {
    pub items: Vec<LlmPreset>,
}
#[derive(Serialize, ToSchema)]
pub struct LlmParameters {
    pub items: Vec<LlmParameterSpec>,
}
#[derive(Serialize, ToSchema)]
pub struct LlmCatalogStatus {
    pub catalog: Option<LlmCatalog>,
}
#[derive(Serialize, ToSchema)]
pub struct LlmPrepared {
    pub snapshot: LlmInvocationSnapshot,
    pub native_request: Value,
}
#[derive(Serialize, ToSchema)]
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
impl From<studio_domain::llm::LlmEvent> for LlmEvent {
    fn from(v: studio_domain::llm::LlmEvent) -> Self {
        use studio_domain::llm::LlmEvent as D;
        match v {
            D::Started { invocation_id } => Self::Started { invocation_id },
            D::Delta {
                index,
                kind,
                text,
                tool_call_id,
            } => Self::Delta {
                index,
                kind,
                text,
                tool_call_id,
            },
            D::Completed { response } => Self::Completed {
                response: Box::new((*response).into()),
            },
            D::Failed { error } => Self::Failed {
                error: error.into(),
            },
        }
    }
}
