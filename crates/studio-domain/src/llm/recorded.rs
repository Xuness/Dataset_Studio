use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LlmRecordedOptions {
    pub stream: bool,
    pub connect_timeout_ms: u32,
    pub first_response_timeout_ms: u32,
    pub idle_timeout_ms: u32,
    pub request_timeout_ms: u32,
}

#[derive(Debug, Clone)]
pub struct LlmTransferProgress {
    pub phase: &'static str,
    pub started_at_ms: Option<u64>,
    pub last_data_at_ms: Option<u64>,
    pub received_bytes: u64,
}

#[derive(Debug, Clone, Default)]
pub struct LlmDispatchDecision {
    pub defer: bool,
    pub recovery_deadline_ms: Option<u64>,
}
