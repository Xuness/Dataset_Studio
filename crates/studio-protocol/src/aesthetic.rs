use crate::{AssetKey, QuerySourceVersion, llm::*};
use serde::{Deserialize, Serialize};
use studio_domain::aesthetic as domain;
use utoipa::ToSchema;
mod execution;
pub use execution::*;

#[derive(Serialize, ToSchema)]
pub struct AestheticExecution {
    pub template_version: String,
    pub business_schema_version: u32,
    pub encoder_version: String,
    pub sampler_version: String,
    pub input_version: String,
}
impl From<domain::AestheticExecution> for AestheticExecution {
    fn from(v: domain::AestheticExecution) -> Self {
        Self {
            template_version: v.template_version,
            business_schema_version: v.business_schema_version,
            encoder_version: v.encoder_version,
            sampler_version: v.sampler_version,
            input_version: v.input_version,
        }
    }
}
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AestheticDisposition {
    Active,
    NeedsReview,
    Rejudge,
    Excluded,
}
impl From<domain::AestheticDisposition> for AestheticDisposition {
    fn from(v: domain::AestheticDisposition) -> Self {
        match v {
            domain::AestheticDisposition::Active => Self::Active,
            domain::AestheticDisposition::NeedsReview => Self::NeedsReview,
            domain::AestheticDisposition::Rejudge => Self::Rejudge,
            domain::AestheticDisposition::Excluded => Self::Excluded,
        }
    }
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AestheticDispositionAction {
    Rejudge,
    Exclude,
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AestheticCandidateDecision {
    pub idempotency_key: String,
    pub action: AestheticDispositionAction,
    pub reason: String,
}
impl From<AestheticCandidateDecision> for domain::AestheticCandidateDecision {
    fn from(v: AestheticCandidateDecision) -> Self {
        Self {
            idempotency_key: v.idempotency_key,
            action: match v.action {
                AestheticDispositionAction::Rejudge => domain::AestheticDispositionAction::Rejudge,
                AestheticDispositionAction::Exclude => domain::AestheticDispositionAction::Exclude,
            },
            reason: v.reason,
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct AestheticCapabilities {
    pub version: u32,
    pub max_stage_candidates: u64,
    pub batch_size: u32,
    pub max_image_bytes: u64,
    pub max_request_bytes: u64,
    pub default_request_bytes: u64,
}
impl From<domain::AestheticCapabilities> for AestheticCapabilities {
    fn from(v: domain::AestheticCapabilities) -> Self {
        Self {
            version: v.version,
            max_stage_candidates: v.max_stage_candidates,
            batch_size: v.batch_size,
            max_image_bytes: v.max_image_bytes,
            max_request_bytes: v.max_request_bytes,
            default_request_bytes: v.default_request_bytes,
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct AestheticPreflight {
    pub capabilities: AestheticCapabilities,
    pub total: u64,
    pub input_version: String,
    pub admitted: bool,
    pub rejection_code: Option<String>,
    pub rejection_reason: Option<String>,
    pub available_storage_bytes: u64,
    pub minimum_calls_lower_bound: u64,
}
impl From<domain::AestheticPreflight> for AestheticPreflight {
    fn from(v: domain::AestheticPreflight) -> Self {
        Self {
            capabilities: v.capabilities.into(),
            total: v.total,
            input_version: v.input_version,
            admitted: v.admitted,
            rejection_code: v.rejection_code,
            rejection_reason: v.rejection_reason,
            available_storage_bytes: v.available_storage_bytes,
            minimum_calls_lower_bound: v.minimum_calls_lower_bound,
        }
    }
}

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_input_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_request_mib: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampling: Option<AestheticSamplingPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_policy: Option<AestheticExecutionPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_mode: Option<String>,
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
            expected_input_version: v.expected_input_version,
            max_request_mib: v.max_request_mib,
            sampling: v.sampling.map(Into::into),
            execution_policy: v.execution_policy.map(Into::into),
            budget_mode: v.budget_mode,
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
            expected_input_version: v.expected_input_version,
            max_request_mib: v.max_request_mib,
            sampling: v.sampling.map(Into::into),
            execution_policy: v.execution_policy.map(Into::into),
            budget_mode: v.budget_mode,
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
    pub execution: Option<AestheticExecution>,
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
            execution: v.execution.map(Into::into),
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
    pub comparable: u64,
    pub excluded: u64,
    pub unresolved: u64,
    pub attempts: u64,
    pub accepted: u64,
    pub invalid: u64,
    pub unknown: u64,
    pub protected: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub usage_unknown: u64,
    pub error: Option<String>,
    pub sampling: Option<AestheticSamplingStatus>,
    pub archived: bool,
    pub execution_settings: Option<AestheticExecutionSettings>,
    pub progress: AestheticStageProgress,
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
            comparable: v.comparable,
            excluded: v.excluded,
            unresolved: v.unresolved,
            attempts: v.attempts,
            accepted: v.accepted,
            invalid: v.invalid,
            unknown: v.unknown,
            protected: v.protected,
            input_tokens: v.input_tokens,
            output_tokens: v.output_tokens,
            usage_unknown: v.usage_unknown,
            error: v.error,
            sampling: v.sampling.map(Into::into),
            archived: v.archived,
            execution_settings: v.execution_settings.map(Into::into),
            progress: v.progress.into(),
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
    pub disposition: AestheticDisposition,
    pub disposition_reason: Option<String>,
    pub blocked: bool,
    pub blocking_batch: Option<u64>,
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
            disposition: v.disposition.into(),
            disposition_reason: v.disposition_reason,
            blocked: v.blocked,
            blocking_batch: v.blocking_batch,
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
    pub stage_sequence: u64,
    pub stage_id: String,
    pub rating: String,
    pub state: String,
    pub members: Vec<AestheticMember>,
    pub attempt_id: Option<String>,
    pub error: Option<String>,
    pub observation: Option<AestheticObservation>,
    pub parent_sequence: Option<u64>,
    pub replacement_sequences: Vec<u64>,
    pub sampling: Option<AestheticBatchSampling>,
    pub attempt_count: u32,
    pub retry_at: Option<String>,
    pub recovery_deadline: Option<String>,
    pub resolution_reason: Option<String>,
    pub last_failure: Option<LlmFailure>,
    pub has_raw_receipt: bool,
    pub transfer: Option<AestheticTransfer>,
}
impl From<domain::AestheticBatch> for AestheticBatch {
    fn from(v: domain::AestheticBatch) -> Self {
        Self {
            sequence: v.sequence,
            stage_sequence: v.stage_sequence,
            attempt_count: v.attempt_count,
            retry_at: v.retry_at,
            recovery_deadline: v.recovery_deadline,
            resolution_reason: v.resolution_reason,
            last_failure: v.last_failure.map(Into::into),
            has_raw_receipt: v.has_raw_receipt,
            transfer: v.transfer.map(Into::into),
            parent_sequence: v.parent_sequence,
            replacement_sequences: v.replacement_sequences,
            sampling: v.sampling.map(Into::into),
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
    pub semantic_request_hash: Option<String>,
    pub raw_receipt: Option<AestheticRawSummary>,
    pub execution_settings: Option<AestheticExecutionSettings>,
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
            semantic_request_hash: v.semantic_request_hash,
            execution_settings: v.execution_settings.map(Into::into),
            raw_receipt: v.raw_receipt.map(|r| AestheticRawSummary {
                sha256: r.sha256,
                bytes: r.bytes,
                complete: r.complete,
                http_status: r.http_status,
            }),
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
    pub queued_write_count: u64,
    pub oldest_write_wait_ms: u64,
    pub last_write_commit_ms: u64,
    pub reserved_receipt_write_bytes: u64,
    pub peak_write_bytes: u64,
    pub dispatch_health: String,
    pub storage_error_code: Option<String>,
    pub retained_outcomes: u64,
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
            queued_write_count: v.queued_write_count,
            oldest_write_wait_ms: v.oldest_write_wait_ms,
            last_write_commit_ms: v.last_write_commit_ms,
            reserved_receipt_write_bytes: v.reserved_receipt_write_bytes,
            peak_write_bytes: v.peak_write_bytes,
            dispatch_health: v.dispatch_health,
            storage_error_code: v.storage_error_code,
            retained_outcomes: v.retained_outcomes,
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct AestheticRawSummary {
    pub sha256: String,
    pub bytes: u64,
    pub complete: bool,
    pub http_status: u16,
}

#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AestheticSamplingPolicy {
    /// balanced or adaptive; both use frozen rounds and cross-batch mixing.
    pub mode: String,
    pub min_exposures: u32,
    pub max_exposures: u32,
    /// Maximum consecutive percentile movement for empirical stability (0..1).
    pub rank_tolerance: f64,
    pub seed: u32,
}
impl From<domain::AestheticSamplingPolicy> for AestheticSamplingPolicy {
    fn from(v: domain::AestheticSamplingPolicy) -> Self {
        Self {
            mode: v.mode,
            min_exposures: v.min_exposures,
            max_exposures: v.max_exposures,
            rank_tolerance: v.rank_tolerance,
            seed: v.seed,
        }
    }
}
impl From<AestheticSamplingPolicy> for domain::AestheticSamplingPolicy {
    fn from(v: AestheticSamplingPolicy) -> Self {
        Self {
            mode: v.mode,
            min_exposures: v.min_exposures,
            max_exposures: v.max_exposures,
            rank_tolerance: v.rank_tolerance,
            seed: v.seed,
        }
    }
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AestheticSamplingRequest {
    pub idempotency_key: String,
    pub policy: AestheticSamplingPolicy,
    pub additional_calls: u32,
}
impl From<domain::AestheticSamplingRequest> for AestheticSamplingRequest {
    fn from(v: domain::AestheticSamplingRequest) -> Self {
        Self {
            idempotency_key: v.idempotency_key,
            policy: v.policy.into(),
            additional_calls: v.additional_calls,
        }
    }
}
impl From<AestheticSamplingRequest> for domain::AestheticSamplingRequest {
    fn from(v: AestheticSamplingRequest) -> Self {
        Self {
            idempotency_key: v.idempotency_key,
            policy: v.policy.into(),
            additional_calls: v.additional_calls,
        }
    }
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct AestheticSamplingStatus {
    pub plan_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_plan_id: Option<String>,
    pub version: String,
    pub policy: AestheticSamplingPolicy,
    pub call_limit: u64,
    pub round: u32,
    pub evidence_watermark: u64,
    pub state: String,
    pub reason: Option<String>,
    pub eligible: u64,
    pub covered: u64,
    pub stable: u64,
    pub components: u64,
    pub unresolved: u64,
}
impl From<domain::AestheticSamplingStatus> for AestheticSamplingStatus {
    fn from(v: domain::AestheticSamplingStatus) -> Self {
        Self {
            plan_id: v.plan_id,
            previous_plan_id: v.previous_plan_id,
            version: v.version,
            policy: v.policy.into(),
            call_limit: v.call_limit,
            round: v.round,
            evidence_watermark: v.evidence_watermark,
            state: v.state,
            reason: v.reason,
            eligible: v.eligible,
            covered: v.covered,
            stable: v.stable,
            components: v.components,
            unresolved: v.unresolved,
        }
    }
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct AestheticSamplingDiagnostic {
    pub ordinal: u64,
    pub exposures: u32,
    /// Exact count, capped at 32; not a count of independent judgments.
    pub distinct_opponents: u32,
    pub component: Option<u64>,
    pub component_size: u64,
    pub percentile: Option<f64>,
    pub rank_delta: Option<f64>,
    /// Whole-batch influence sensitivity in percentile units, not a confidence interval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank_sensitivity: Option<f64>,
    pub stable_rounds: u32,
    pub reason: String,
}
impl From<domain::AestheticSamplingDiagnostic> for AestheticSamplingDiagnostic {
    fn from(v: domain::AestheticSamplingDiagnostic) -> Self {
        Self {
            ordinal: v.ordinal,
            exposures: v.exposures,
            distinct_opponents: v.distinct_opponents,
            component: v.component,
            component_size: v.component_size,
            percentile: v.percentile,
            rank_delta: v.rank_delta,
            rank_sensitivity: v.rank_sensitivity,
            stable_rounds: v.stable_rounds,
            reason: v.reason,
        }
    }
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct AestheticSamplingMemberReason {
    pub ordinal: u64,
    pub reason: String,
}
impl From<domain::AestheticSamplingMemberReason> for AestheticSamplingMemberReason {
    fn from(v: domain::AestheticSamplingMemberReason) -> Self {
        Self {
            ordinal: v.ordinal,
            reason: v.reason,
        }
    }
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct AestheticBatchSampling {
    pub plan_id: String,
    pub round: u32,
    pub evidence_watermark: u64,
    pub members: Vec<AestheticSamplingMemberReason>,
}
impl From<domain::AestheticBatchSampling> for AestheticBatchSampling {
    fn from(v: domain::AestheticBatchSampling) -> Self {
        Self {
            plan_id: v.plan_id,
            round: v.round,
            evidence_watermark: v.evidence_watermark,
            members: v.members.into_iter().map(Into::into).collect(),
        }
    }
}
