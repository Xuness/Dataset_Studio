use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct ResourceStatus {
    pub class: String,
    pub concurrency: usize,
    pub queue_limit: usize,
    pub byte_budget: String,
    pub active: usize,
    pub queued: usize,
    pub reserved_bytes: String,
    pub peak_reserved_bytes: String,
    pub started: u64,
    pub completed: u64,
    pub cancelled_waiting: u64,
    pub rejected: u64,
    pub wait_ms: u64,
    pub max_wait_ms: u64,
    pub work_ms: u64,
}
impl From<studio_domain::ReadMetrics> for ResourceStatus {
    fn from(m: studio_domain::ReadMetrics) -> Self {
        use studio_domain::ReadClass;
        Self {
            class: match m.budget.class {
                ReadClass::Index => "index",
                ReadClass::Media => "media",
                ReadClass::Decode => "decode",
                ReadClass::NativeQuery => "native_query",
            }
            .into(),
            concurrency: m.budget.concurrency,
            queue_limit: m.budget.queue_limit,
            byte_budget: m.budget.bytes.to_string(),
            active: m.active,
            queued: m.queued,
            reserved_bytes: m.reserved_bytes.to_string(),
            peak_reserved_bytes: m.peak_reserved_bytes.to_string(),
            started: m.started,
            completed: m.completed,
            cancelled_waiting: m.cancelled_waiting,
            rejected: m.rejected,
            wait_ms: m.wait_ms,
            max_wait_ms: m.max_wait_ms,
            work_ms: m.work_ms,
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct PreviewCacheStatus {
    pub directory: String,
    pub quota_bytes: String,
    pub bytes: String,
    pub entries: u64,
    pub pinned: usize,
    pub hits: u64,
    pub misses: u64,
    pub corrupt: u64,
    pub evicted: u64,
    pub writes: u64,
    pub read_bytes: String,
    pub maintenance_removed: u64,
    pub maintenance_pending: bool,
    pub index_rebuilt: bool,
    pub clear_pending: bool,
}
#[derive(Serialize, ToSchema)]
pub struct PreviewActivity {
    pub shared: u64,
    pub queued: usize,
    pub active_subscriptions: usize,
    pub generated: u64,
    pub cancelled_last: u64,
    pub cancelled_before_read: u64,
    pub source_bytes: String,
    pub pack_opens: u64,
    pub seeks: u64,
    pub decode_ms: u64,
    pub read_ms: u64,
    pub batches: u64,
    pub max_batch: usize,
    pub queue_wait_ms: u64,
    pub max_queue_wait_ms: u64,
    pub cancelled_finished: u64,
    pub max_cancel_latency_ms: u64,
}
#[derive(Serialize, ToSchema)]
pub struct ReadServiceStatus {
    pub protocol_version: u32,
    pub resources: Vec<ResourceStatus>,
    pub cache: PreviewCacheStatus,
    pub previews: PreviewActivity,
    pub process_memory: Option<ReadProcessMemory>,
    pub online_sqlite: OnlineSqliteStatus,
    pub query_limits: QueryResourceLimits,
    pub query_cache: QueryCacheStatus,
}
#[derive(Serialize, ToSchema)]
pub struct OnlineSqliteStatus {
    pub busy_errors: u64,
    pub protocol_errors: u64,
    pub lease_retries: u64,
    pub max_lease_write_ms: u64,
}
#[derive(Serialize, ToSchema)]
pub struct QueryCacheStatus {
    pub quota_bytes: String,
    pub max_age_days: u32,
    pub retained_queries: u64,
    pub member_versions: u64,
    pub result_storage_bytes: String,
    pub database_free_bytes: String,
    pub protected_results: u64,
    pub active_views: u64,
    pub reused_results: u64,
    pub incremental_results: u64,
    pub source_index_bytes: String,
    pub source_indexes: u64,
    pub ranked_index_bytes: String,
    pub ranked_indexes: u64,
    pub ranked_index_builds: u64,
    pub ranked_index_reuses: u64,
    pub cleanup_pending: bool,
    pub reclaimed_queries: u64,
}
#[derive(Deserialize, ToSchema)]
pub struct SetQueryCache {
    pub quota_mib: u32,
    pub max_age_days: u32,
}
#[derive(Serialize, ToSchema)]
pub struct QueryResourceLimits {
    pub metadata_memory_bytes: String,
    pub query_memory_bytes: String,
    pub native_query_memory_bytes: String,
    pub result_work_memory_bytes: String,
    pub active_query_memory_bytes: Option<String>,
    pub temporary_disk_bytes: String,
    pub result_staging_disk_bytes: String,
}
#[derive(Deserialize, ToSchema)]
pub struct SetQueryMemory {
    pub memory_gib: u32,
}
#[derive(Serialize, ToSchema)]
pub struct ReadProcessMemory {
    pub resident_bytes: String,
    pub peak_resident_bytes: String,
    pub private_bytes: String,
}
#[derive(Deserialize, ToSchema)]
pub struct SetCacheQuota {
    pub quota_mib: u32,
}
