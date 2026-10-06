//! Mutable execution policy is separate from the frozen aesthetic evidence standard.
use serde::{Deserialize, Serialize};

pub const AESTHETIC_MAX_CONCURRENCY: u32 = 1024;
/// A logical batch keeps at most eight attempts: the first send plus seven automatic retries.
pub const AESTHETIC_MAX_AUTO_RETRIES: u32 = 7;
/// Applied when a stage predates per-stage resource limits.
pub const AESTHETIC_DEFAULT_MEMORY_BUDGET_MIB: u32 = 512;
pub const AESTHETIC_DEFAULT_UPLOAD_BYTES_PER_SECOND: u64 = 3_500_000;
/// Engine-wide limit on stages executing at once, across all projects.
pub const AESTHETIC_DEFAULT_RUNNING_STAGES: u32 = 8;
pub const AESTHETIC_MAX_RUNNING_STAGES: u32 = 32;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AestheticExecutionPolicy {
    /// Local, aspect-preserving downscale before each new API attempt; absent keeps original bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_max_edge: Option<u32>,
    pub stream: bool,
    pub concurrency: u32,
    pub connect_timeout_ms: u32,
    pub first_response_timeout_ms: u32,
    pub idle_timeout_ms: u32,
    pub request_timeout_ms: u32,
    /// Wall-clock recovery budget, starting at the first network dispatch.
    pub batch_timeout_ms: u32,
    pub max_retries: u32,
    pub retry_unknown: bool,
    /// pause or defer; defer closes failed logical batches without accepting evidence.
    pub exhausted: String,
    /// Per-stage request preparation memory budget. Absent on legacy stages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_budget_mib: Option<u32>,
    /// Request admission pacing by serialized body bytes; 0 disables pacing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_bytes_per_second: Option<u64>,
    /// Consecutive failed network attempts that halt dispatch; absent follows concurrency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_halt_threshold: Option<u32>,
}
impl AestheticExecutionPolicy {
    pub fn memory_budget_bytes(&self) -> u64 {
        u64::from(
            self.memory_budget_mib
                .unwrap_or(AESTHETIC_DEFAULT_MEMORY_BUDGET_MIB),
        ) << 20
    }
    pub fn upload_rate(&self) -> u64 {
        self.upload_bytes_per_second
            .unwrap_or(AESTHETIC_DEFAULT_UPLOAD_BYTES_PER_SECOND)
    }
    pub fn failure_halt_threshold(&self) -> u32 {
        self.failure_halt_threshold
            .unwrap_or_else(|| default_failure_halt_threshold(self.concurrency))
    }
}
/// One in-flight wave may fail together without halting; capped so a high concurrency
/// does not keep dispatching into a failing endpoint. Equals the legacy rule up to 32.
pub fn default_failure_halt_threshold(concurrency: u32) -> u32 {
    concurrency.clamp(4, 32)
}
impl Default for AestheticExecutionPolicy {
    fn default() -> Self {
        Self {
            image_max_edge: None,
            stream: true,
            concurrency: 2,
            connect_timeout_ms: 15_000,
            first_response_timeout_ms: 180_000,
            idle_timeout_ms: 60_000,
            request_timeout_ms: 600_000,
            batch_timeout_ms: 900_000,
            max_retries: 2,
            retry_unknown: false,
            exhausted: "pause".into(),
            memory_budget_mib: None,
            upload_bytes_per_second: None,
            failure_halt_threshold: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AestheticExecutionSettings {
    pub revision: u64,
    pub updated_at: String,
    pub provider_revision: u64,
    pub model_revision: u64,
    pub policy: AestheticExecutionPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AestheticExecutionUpdate {
    pub idempotency_key: String,
    pub expected_revision: u64,
    pub policy: AestheticExecutionPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AestheticStageProgress {
    pub round_planned: u64,
    pub round_unclaimed: u64,
    pub round_accepted: u64,
    pub preparing: u64,
    pub in_flight: u64,
    pub queued: u64,
    pub retry_waiting: u64,
    pub failed: u64,
    pub deferred: u64,
    pub blocked: u64,
    pub covered: u64,
    pub exposed_once: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AestheticTransfer {
    pub phase: String,
    pub started_at: Option<String>,
    pub last_data_at: Option<String>,
    pub received_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AestheticStageMetadata {
    pub name: String,
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AestheticBatchAction {
    pub idempotency_key: String,
    /// retry, reparse, or defer. Reparse is handled by the engine.
    pub action: String,
    /// Empty means all matching batches at the operation's frozen upper bound.
    pub batches: Vec<u64>,
    pub acknowledge_possible_charge: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AestheticBatchActionItem {
    pub sequence: u64,
    pub stage_sequence: u64,
    pub state: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AestheticBatchActionResult {
    pub id: String,
    pub completed: bool,
    pub processed: u64,
    pub succeeded: u64,
    pub failed: u64,
    /// Bounded results of the most recently processed page.
    pub items: Vec<AestheticBatchActionItem>,
}
