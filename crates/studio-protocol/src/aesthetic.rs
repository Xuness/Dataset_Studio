use crate::{AssetKey, QuerySourceVersion, llm::*};
use serde::{Deserialize, Serialize};
use studio_domain::aesthetic as domain;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AestheticCreate {
    pub idempotency_key: String,
    pub name: String,
    pub collection_id: String,
    pub model_id: String,
    pub system_prompt_id: String,
    #[serde(default)]
    pub overrides: std::collections::BTreeMap<String, serde_json::Value>,
    pub exposures: u32,
    pub max_calls: u32,
    pub concurrency: u32,
}
impl From<domain::AestheticCreate> for AestheticCreate {
    fn from(v: domain::AestheticCreate) -> Self {
        Self {
            idempotency_key: v.idempotency_key,
            name: v.name,
            collection_id: v.collection_id,
            model_id: v.model_id,
            system_prompt_id: v.system_prompt_id,
            overrides: v.overrides,
            exposures: v.exposures,
            max_calls: v.max_calls,
            concurrency: v.concurrency,
        }
    }
}
impl From<AestheticCreate> for domain::AestheticCreate {
    fn from(v: AestheticCreate) -> Self {
        Self {
            idempotency_key: v.idempotency_key,
            name: v.name,
            collection_id: v.collection_id,
            model_id: v.model_id,
            system_prompt_id: v.system_prompt_id,
            overrides: v.overrides,
            exposures: v.exposures,
            max_calls: v.max_calls,
            concurrency: v.concurrency,
        }
    }
}
#[derive(Serialize, ToSchema)]
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
impl From<domain::AestheticConfig> for AestheticConfig {
    fn from(v: domain::AestheticConfig) -> Self {
        Self {
            version: v.version,
            request: v.request.into(),
            model: v.model.into(),
            sources: v.sources.into_iter().map(Into::into).collect(),
            image_policy: v.image_policy,
            grouping_policy: v.grouping_policy,
            observation_policy: v.observation_policy,
            max_image_bytes: v.max_image_bytes,
            max_request_bytes: v.max_request_bytes,
        }
    }
}
#[derive(Serialize, ToSchema)]
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
impl From<domain::AestheticStage> for AestheticStage {
    fn from(v: domain::AestheticStage) -> Self {
        Self {
            id: v.id,
            name: v.name,
            state: v.state,
            created_at: v.created_at,
            config: v.config.into(),
            config_hash: v.config_hash,
            total: v.total,
            frozen: v.frozen,
            eligible: v.eligible,
            attempts: v.attempts,
            accepted: v.accepted,
            invalid: v.invalid,
            unknown: v.unknown,
            protected: v.protected,
            input_tokens: v.input_tokens,
            output_tokens: v.output_tokens,
            usage_unknown: v.usage_unknown,
            error: v.error,
        }
    }
}
#[derive(Serialize, ToSchema)]
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
impl From<domain::AestheticCandidate> for AestheticCandidate {
    fn from(v: domain::AestheticCandidate) -> Self {
        Self {
            ordinal: v.ordinal,
            key: v.key.into(),
            rating: v.rating,
            year: v.year,
            basis: v.basis,
            content_version: v.content_version,
            bytes: v.bytes,
            exposures: v.exposures,
            protected: v.protected,
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct AestheticMember {
    pub label: String,
    pub candidate: AestheticCandidate,
    pub image_sha256: Option<String>,
}
impl From<domain::AestheticMember> for AestheticMember {
    fn from(v: domain::AestheticMember) -> Self {
        Self {
            label: v.label,
            candidate: v.candidate.into(),
            image_sha256: v.image_sha256,
        }
    }
}
#[derive(Serialize, ToSchema)]
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
impl From<domain::AestheticBatch> for AestheticBatch {
    fn from(v: domain::AestheticBatch) -> Self {
        Self {
            sequence: v.sequence,
            stage_id: v.stage_id,
            rating: v.rating,
            state: v.state,
            members: v.members.into_iter().map(Into::into).collect(),
            attempt_id: v.attempt_id,
            error: v.error,
            observation: v.observation.map(Into::into),
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct AestheticUnjudgeable {
    pub id: String,
    pub reason: String,
}
impl From<domain::AestheticUnjudgeable> for AestheticUnjudgeable {
    fn from(v: domain::AestheticUnjudgeable) -> Self {
        Self {
            id: v.id,
            reason: v.reason,
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct AestheticObservation {
    pub schema_version: u32,
    pub tiers: Vec<Vec<String>>,
    pub elite_candidates: Vec<String>,
    pub unjudgeable: Vec<AestheticUnjudgeable>,
}
impl From<domain::AestheticObservation> for AestheticObservation {
    fn from(v: domain::AestheticObservation) -> Self {
        Self {
            schema_version: v.schema_version,
            tiers: v.tiers,
            elite_candidates: v.elite_candidates,
            unjudgeable: v.unjudgeable.into_iter().map(Into::into).collect(),
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct AestheticReceipt {
    pub provider_request_id: Option<String>,
    pub response_id: Option<String>,
    pub model: Option<String>,
    pub outputs: Vec<LlmOutput>,
    pub usage: LlmUsage,
}
impl From<domain::AestheticReceipt> for AestheticReceipt {
    fn from(v: domain::AestheticReceipt) -> Self {
        Self {
            provider_request_id: v.provider_request_id,
            response_id: v.response_id,
            model: v.model,
            outputs: v.outputs.into_iter().map(Into::into).collect(),
            usage: v.usage.into(),
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct AestheticAttempt {
    pub id: String,
    pub batch: u64,
    pub state: String,
    pub created_at: String,
    pub receipt: Option<AestheticReceipt>,
    pub failure: Option<LlmFailure>,
}
impl From<domain::AestheticAttempt> for AestheticAttempt {
    fn from(v: domain::AestheticAttempt) -> Self {
        Self {
            id: v.id,
            batch: v.batch,
            state: v.state,
            created_at: v.created_at,
            receipt: v.receipt.map(Into::into),
            failure: v.failure.map(Into::into),
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct AestheticMetrics {
    pub active_requests: u64,
    pub reserved_request_bytes: u64,
    pub peak_request_bytes: u64,
    pub upload_budget_bytes_per_second: u64,
    pub uploaded_body_bytes: u64,
    pub queued_write_bytes: u64,
    pub peak_write_bytes: u64,
}
impl From<domain::AestheticMetrics> for AestheticMetrics {
    fn from(v: domain::AestheticMetrics) -> Self {
        Self {
            active_requests: v.active_requests,
            reserved_request_bytes: v.reserved_request_bytes,
            peak_request_bytes: v.peak_request_bytes,
            upload_budget_bytes_per_second: v.upload_budget_bytes_per_second,
            uploaded_body_bytes: v.uploaded_body_bytes,
            queued_write_bytes: v.queued_write_bytes,
            peak_write_bytes: v.peak_write_bytes,
        }
    }
}
