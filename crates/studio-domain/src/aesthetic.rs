//! Durable aesthetic observations. Scores and ranking estimators are separate projections.
use crate::{AssetKey, QuerySourceVersion, llm::*};
use serde::{Deserialize, Serialize};

pub const AESTHETIC_VERSION: u32 = 1;
pub const AESTHETIC_BATCH_SIZE: usize = 16;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AestheticCreate {
    pub idempotency_key: String,
    pub name: String,
    pub collection_id: String,
    pub model_id: String,
    pub system_prompt_id: String,
    #[serde(default)]
    pub overrides: LlmParameters,
    pub exposures: u32,
    pub max_calls: u32,
    pub concurrency: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AestheticConfig {
    pub version: u32,
    pub request: AestheticCreate,
    /// Credential-free, resolved configuration; contains no image bytes.
    pub model: LlmInvocationSnapshot,
    pub sources: Vec<QuerySourceVersion>,
    pub image_policy: String,
    pub grouping_policy: String,
    pub observation_policy: String,
    pub max_image_bytes: u64,
    pub max_request_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AestheticStage {
    pub id: String,
    pub name: String,
    pub state: String,
    pub created_at: String,
    pub config: AestheticConfig,
    pub config_hash: String,
    pub total: u64,
    pub frozen: u64,
    pub eligible: u64,
    pub attempts: u64,
    pub accepted: u64,
    pub invalid: u64,
    pub unknown: u64,
    pub protected: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub usage_unknown: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AestheticCandidate {
    pub ordinal: u64,
    pub key: AssetKey,
    pub rating: String,
    pub year: Option<i32>,
    pub basis: String,
    pub content_version: String,
    pub bytes: u64,
    pub exposures: u32,
    pub protected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AestheticMember {
    pub label: String,
    pub candidate: AestheticCandidate,
    pub image_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AestheticBatch {
    pub sequence: u64,
    pub stage_id: String,
    pub rating: String,
    pub state: String,
    pub members: Vec<AestheticMember>,
    pub attempt_id: Option<String>,
    pub error: Option<String>,
    pub observation: Option<AestheticObservation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AestheticUnjudgeable {
    pub id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AestheticObservation {
    pub schema_version: u32,
    pub tiers: Vec<Vec<String>>,
    pub elite_candidates: Vec<String>,
    pub unjudgeable: Vec<AestheticUnjudgeable>,
}

/// Normalized provider output, persisted before business parsing; excludes input images.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AestheticReceipt {
    pub provider_request_id: Option<String>,
    pub response_id: Option<String>,
    pub model: Option<String>,
    pub outputs: Vec<LlmOutput>,
    pub usage: LlmUsage,
}
impl From<LlmResponse> for AestheticReceipt {
    fn from(v: LlmResponse) -> Self {
        Self {
            provider_request_id: v.provider_request_id,
            response_id: v.response_id,
            model: v.model,
            outputs: v.outputs,
            usage: v.usage,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AestheticAttempt {
    pub id: String,
    pub batch: u64,
    pub state: String,
    pub created_at: String,
    pub receipt: Option<AestheticReceipt>,
    pub failure: Option<LlmFailure>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AestheticMetrics {
    pub active_requests: u64,
    pub reserved_request_bytes: u64,
    pub peak_request_bytes: u64,
    pub upload_budget_bytes_per_second: u64,
    pub uploaded_body_bytes: u64,
    pub queued_write_bytes: u64,
    pub peak_write_bytes: u64,
}
