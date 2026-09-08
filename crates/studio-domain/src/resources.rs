use serde::{Deserialize, Serialize};

/// Shared admission and native execution limits, in bytes.
pub const METADATA_MEMORY_BYTES: u64 = 256 << 20;
pub const QUERY_MEMORY_BYTES: u64 = 12 << 30;
pub const QUERY_TEMP_BYTES: u64 = 8 << 30;
pub const QUERY_STAGE_BYTES: u64 = 8 << 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadClass {
    Index,
    Media,
    Decode,
    NativeQuery,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadPriority {
    Interactive,
    Background,
    Prefetch,
}
#[derive(Debug, Clone, Copy)]
pub struct ReadRequest {
    pub class: ReadClass,
    pub priority: ReadPriority,
    /// Admission reservation, bounded independently for each resource class.
    pub bytes: u64,
}
#[derive(Debug, Clone, Serialize)]
pub struct ReadBudget {
    pub class: ReadClass,
    pub concurrency: usize,
    pub queue_limit: usize,
    pub bytes: u64,
}
#[derive(Debug, Clone, Serialize)]
pub struct ReadMetrics {
    pub budget: ReadBudget,
    pub active: usize,
    pub queued: usize,
    pub reserved_bytes: u64,
    pub peak_reserved_bytes: u64,
    pub started: u64,
    pub completed: u64,
    pub cancelled_waiting: u64,
    pub rejected: u64,
    pub wait_ms: u64,
    pub max_wait_ms: u64,
    pub work_ms: u64,
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct PhysicalReadStats {
    pub bytes: u64,
    pub opens: u64,
    pub seeks: u64,
    pub cancelled: u64,
    pub cancelled_before_read: u64,
    pub trace: Vec<PhysicalReadTrace>,
}
#[derive(Debug, Clone, Serialize)]
pub struct PhysicalReadTrace {
    pub asset_id: String,
    pub pack: String,
    pub offset: u64,
    pub bytes: u64,
}
