//! Mutable execution policy is separate from the frozen aesthetic evidence standard.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AestheticExecutionPolicy {
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
}
impl Default for AestheticExecutionPolicy {
    fn default() -> Self {
        Self {
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
