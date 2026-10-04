//! Durable aesthetic observations. Scores and ranking estimators are separate projections.
use crate::{AssetKey, QuerySourceVersion, llm::*};
use serde::{Deserialize, Serialize};
mod execution;
pub use execution::*;

pub const AESTHETIC_VERSION: u32 = 1;
pub const AESTHETIC_BATCH_SIZE: usize = 16;
/// Shared by paid admission and offline replay; this is a whole-stage limit.
pub const AESTHETIC_MAX_CANDIDATES: u64 = 10_000_000;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_input_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_request_mib: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampling: Option<AestheticSamplingPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_policy: Option<AestheticExecutionPolicy>,
    /// Omitted preserves legacy admission. New clients explicitly choose complete or trial.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AestheticExecution {
    pub template_version: String,
    pub business_schema_version: u32,
    pub encoder_version: String,
    pub sampler_version: String,
    pub input_version: String,
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
    /// Absent on legacy stages. Their stored messages are still authoritative.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<AestheticExecution>,
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
    #[serde(default)]
    pub sampling: Option<AestheticSamplingStatus>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub execution_settings: Option<AestheticExecutionSettings>,
    #[serde(default)]
    pub progress: AestheticStageProgress,
}

impl AestheticStage {
    pub fn call_limit(&self) -> u64 {
        self.sampling
            .as_ref()
            .map_or(u64::from(self.config.request.max_calls), |s| s.call_limit)
    }
}

/// Frozen scheduling policy. Stability is an empirical diagnostic, not a confidence interval.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AestheticSamplingPolicy {
    /// balanced/adaptive retain v1; refine/refine_balanced use v2 neighbor comparisons.
    /// refine allocates a bounded budget; it does not assert calibrated precision.
    pub mode: String,
    pub min_exposures: u32,
    pub max_exposures: u32,
    /// Maximum consecutive percentile movement for empirical stability (0..1).
    pub rank_tolerance: f64,
    pub seed: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AestheticSamplingRequest {
    pub idempotency_key: String,
    pub policy: AestheticSamplingPolicy,
    pub additional_calls: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AestheticSamplingMemberReason {
    pub ordinal: u64,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AestheticBatchSampling {
    pub plan_id: String,
    pub round: u32,
    pub evidence_watermark: u64,
    pub members: Vec<AestheticSamplingMemberReason>,
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
    #[serde(default)]
    pub disposition: AestheticDisposition,
    #[serde(default)]
    pub disposition_reason: Option<String>,
    #[serde(default)]
    pub blocked: bool,
    #[serde(default)]
    pub blocking_batch: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AestheticDisposition {
    #[default]
    Active,
    NeedsReview,
    Rejudge,
    Excluded,
}
impl AestheticDisposition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::NeedsReview => "needs_review",
            Self::Rejudge => "rejudge",
            Self::Excluded => "excluded",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AestheticDispositionAction {
    Rejudge,
    Exclude,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AestheticCandidateDecision {
    pub idempotency_key: String,
    pub action: AestheticDispositionAction,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AestheticCapabilities {
    pub version: u32,
    pub max_stage_candidates: u64,
    pub batch_size: u32,
    pub max_image_bytes: u64,
    pub max_request_bytes: u64,
    pub default_request_bytes: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// A project transaction durably records this before creating the other database's stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AestheticCreationIntent {
    pub config: AestheticConfig,
    pub total: u64,
    pub state: String,
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
    #[serde(default)]
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
    #[serde(default)]
    pub sampling: Option<AestheticBatchSampling>,
    #[serde(default)]
    pub attempt_count: u32,
    #[serde(default)]
    pub retry_at: Option<String>,
    #[serde(default)]
    pub recovery_deadline: Option<String>,
    #[serde(default)]
    pub resolution_reason: Option<String>,
    #[serde(default)]
    pub last_failure: Option<LlmFailure>,
    #[serde(default)]
    pub has_raw_receipt: bool,
    #[serde(default)]
    pub transfer: Option<AestheticTransfer>,
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
    pub semantic_request_hash: Option<String>,
    pub raw_receipt: Option<AestheticRawSummary>,
    #[serde(default)]
    pub execution_settings: Option<AestheticExecutionSettings>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AestheticRawSummary {
    pub sha256: String,
    pub bytes: u64,
    pub complete: bool,
    pub http_status: u16,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
