//! Mutable execution policy is separate from the frozen aesthetic evidence standard.
use serde::{Deserialize, Serialize};
use studio_domain::aesthetic as domain;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AestheticExecutionSettings {
    pub revision: u64,
    pub updated_at: String,
    pub provider_revision: u64,
    pub model_revision: u64,
    pub policy: AestheticExecutionPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AestheticExecutionUpdate {
    pub idempotency_key: String,
    pub expected_revision: u64,
    pub policy: AestheticExecutionPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AestheticTransfer {
    pub phase: String,
    pub started_at: Option<String>,
    pub last_data_at: Option<String>,
    pub received_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AestheticStageMetadata {
    pub name: String,
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AestheticBatchActionItem {
    pub sequence: u64,
    pub stage_sequence: u64,
    pub state: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AestheticBatchActionResult {
    pub id: String,
    pub completed: bool,
    pub processed: u64,
    pub succeeded: u64,
    pub failed: u64,
    /// Bounded results of the most recently processed page.
    pub items: Vec<AestheticBatchActionItem>,
}

impl From<domain::AestheticExecutionPolicy> for AestheticExecutionPolicy {
    fn from(v: domain::AestheticExecutionPolicy) -> Self {
        Self {
            stream: v.stream,
            concurrency: v.concurrency,
            connect_timeout_ms: v.connect_timeout_ms,
            first_response_timeout_ms: v.first_response_timeout_ms,
            idle_timeout_ms: v.idle_timeout_ms,
            request_timeout_ms: v.request_timeout_ms,
            batch_timeout_ms: v.batch_timeout_ms,
            max_retries: v.max_retries,
            retry_unknown: v.retry_unknown,
            exhausted: v.exhausted,
        }
    }
}

impl From<AestheticExecutionPolicy> for domain::AestheticExecutionPolicy {
    fn from(v: AestheticExecutionPolicy) -> Self {
        Self {
            stream: v.stream,
            concurrency: v.concurrency,
            connect_timeout_ms: v.connect_timeout_ms,
            first_response_timeout_ms: v.first_response_timeout_ms,
            idle_timeout_ms: v.idle_timeout_ms,
            request_timeout_ms: v.request_timeout_ms,
            batch_timeout_ms: v.batch_timeout_ms,
            max_retries: v.max_retries,
            retry_unknown: v.retry_unknown,
            exhausted: v.exhausted,
        }
    }
}

impl From<domain::AestheticExecutionSettings> for AestheticExecutionSettings {
    fn from(v: domain::AestheticExecutionSettings) -> Self {
        Self {
            revision: v.revision,
            updated_at: v.updated_at,
            provider_revision: v.provider_revision,
            model_revision: v.model_revision,
            policy: v.policy.into(),
        }
    }
}

impl From<domain::AestheticExecutionUpdate> for AestheticExecutionUpdate {
    fn from(v: domain::AestheticExecutionUpdate) -> Self {
        Self {
            idempotency_key: v.idempotency_key,
            expected_revision: v.expected_revision,
            policy: v.policy.into(),
        }
    }
}

impl From<AestheticExecutionUpdate> for domain::AestheticExecutionUpdate {
    fn from(v: AestheticExecutionUpdate) -> Self {
        Self {
            idempotency_key: v.idempotency_key,
            expected_revision: v.expected_revision,
            policy: v.policy.into(),
        }
    }
}

impl From<domain::AestheticStageProgress> for AestheticStageProgress {
    fn from(v: domain::AestheticStageProgress) -> Self {
        Self {
            round_planned: v.round_planned,
            round_unclaimed: v.round_unclaimed,
            round_accepted: v.round_accepted,
            preparing: v.preparing,
            in_flight: v.in_flight,
            queued: v.queued,
            retry_waiting: v.retry_waiting,
            failed: v.failed,
            deferred: v.deferred,
            blocked: v.blocked,
            covered: v.covered,
            exposed_once: v.exposed_once,
        }
    }
}

impl From<domain::AestheticTransfer> for AestheticTransfer {
    fn from(v: domain::AestheticTransfer) -> Self {
        Self {
            phase: v.phase,
            started_at: v.started_at,
            last_data_at: v.last_data_at,
            received_bytes: v.received_bytes,
        }
    }
}

impl From<domain::AestheticStageMetadata> for AestheticStageMetadata {
    fn from(v: domain::AestheticStageMetadata) -> Self {
        Self {
            name: v.name,
            archived: v.archived,
        }
    }
}

impl From<AestheticStageMetadata> for domain::AestheticStageMetadata {
    fn from(v: AestheticStageMetadata) -> Self {
        Self {
            name: v.name,
            archived: v.archived,
        }
    }
}

impl From<domain::AestheticBatchAction> for AestheticBatchAction {
    fn from(v: domain::AestheticBatchAction) -> Self {
        Self {
            idempotency_key: v.idempotency_key,
            action: v.action,
            batches: v.batches,
            acknowledge_possible_charge: v.acknowledge_possible_charge,
            reason: v.reason,
        }
    }
}

impl From<AestheticBatchAction> for domain::AestheticBatchAction {
    fn from(v: AestheticBatchAction) -> Self {
        Self {
            idempotency_key: v.idempotency_key,
            action: v.action,
            batches: v.batches,
            acknowledge_possible_charge: v.acknowledge_possible_charge,
            reason: v.reason,
        }
    }
}

impl From<domain::AestheticBatchActionItem> for AestheticBatchActionItem {
    fn from(v: domain::AestheticBatchActionItem) -> Self {
        Self {
            sequence: v.sequence,
            stage_sequence: v.stage_sequence,
            state: v.state,
            error: v.error,
        }
    }
}

impl From<domain::AestheticBatchActionResult> for AestheticBatchActionResult {
    fn from(v: domain::AestheticBatchActionResult) -> Self {
        Self {
            id: v.id,
            completed: v.completed,
            processed: v.processed,
            succeeded: v.succeeded,
            failed: v.failed,
            items: v.items.into_iter().map(Into::into).collect(),
        }
    }
}
