use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CacheSettings {
    pub total_mib: u32,
    pub long_term_mib: u32,
    pub temporary_mib: u32,
    pub preview_mib: u32,
    pub long_term_idle_days: Option<u32>,
    pub temporary_idle_hours: u32,
    pub temporary_session_only: bool,
}
#[derive(Serialize, ToSchema)]
pub struct CacheStorageOverview {
    pub total_bytes: String,
    pub total_quota_bytes: String,
    pub long_term_bytes: String,
    pub temporary_bytes: String,
    pub preview_bytes: String,
    pub source_index_bytes: String,
    pub rating_basis_bytes: String,
    pub project_member_bytes: String,
    pub ranked_index_bytes: String,
    pub reusable_bytes: String,
    pub fixed_member_bytes: String,
    pub working_temporary_bytes: String,
    pub protected_results: u64,
    pub active_views: u64,
    pub long_term_results: u64,
    pub temporary_results: u64,
    pub cleanup_pending: bool,
}
#[derive(Serialize, ToSchema)]
pub struct SettingsStatus {
    pub cache: CacheSettings,
    pub storage: CacheStorageOverview,
    pub query_limits: crate::QueryResourceLimits,
    pub maintenance: CacheMaintenance,
}
#[derive(Debug, Clone, Default, Serialize, ToSchema)]
pub struct CacheMaintenance {
    pub phase: String,
    pub project_id: Option<String>,
    pub error: Option<String>,
}
#[derive(Serialize, ToSchema)]
pub struct CacheCleanup {
    pub family_id: String,
    pub result_id: String,
    pub spec: Option<crate::QuerySpec>,
    pub state: String,
    pub total: u64,
    pub processed: u64,
    pub removed: u64,
    pub started_millis: String,
    pub updated_millis: String,
    pub error: Option<String>,
}
#[derive(Deserialize, ToSchema)]
pub struct ClearCacheTier {
    pub tier: Option<String>,
}
#[derive(Deserialize, ToSchema)]
pub struct SetResultRetention {
    pub tier: String,
    pub fixed: bool,
}
#[derive(Deserialize, ToSchema)]
pub struct SetRatingRetention {
    pub fixed: bool,
}
#[derive(Serialize, ToSchema)]
pub struct CacheEntry {
    pub project_id: String,
    pub family_id: String,
    pub result_id: String,
    pub spec: crate::QuerySpec,
    pub tier: String,
    pub fixed: bool,
    pub session_only: bool,
    pub last_used_millis: String,
    pub members: u64,
    pub estimated_bytes: Option<String>,
    pub in_use: bool,
    pub protected_results: u64,
}
#[derive(Serialize, ToSchema)]
pub struct CacheEntries {
    pub items: Vec<CacheEntry>,
    pub next_cursor: Option<String>,
    pub cleanups: Vec<CacheCleanup>,
}
#[derive(Serialize, ToSchema)]
pub struct RatingBasis {
    pub source_id: String,
    pub rating: String,
    pub generation: String,
    pub sequence: u64,
    pub records: u64,
    pub bytes: String,
    pub last_used_millis: String,
    pub incremental: bool,
    pub fixed: bool,
    pub active: bool,
}
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RatingBuild {
    pub source_id: String,
    pub state: String,
    pub current_rating: Option<String>,
    pub completed: Vec<String>,
    pub error: Option<String>,
}
#[derive(Serialize, ToSchema)]
pub struct RatingBases {
    pub items: Vec<RatingBasis>,
    pub builds: Vec<RatingBuild>,
}
