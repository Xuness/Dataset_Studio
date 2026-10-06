use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use utoipa::ToSchema;

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PrepareLakeUpdateInputs {
    pub request_key: String,
    pub scope: crate::ScopeRef,
    pub label: Option<String>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct LakeUpdatePreparedInput {
    pub library_id: String,
    pub input_id: String,
    pub count: u64,
    pub sealed: bool,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct LakeUpdatePreparation {
    pub id: String,
    pub project_id: String,
    #[serde(default)]
    pub project_name: String,
    #[serde(default)]
    pub label: String,
    pub state: String,
    pub processed: u64,
    pub total: Option<u64>,
    pub inputs: Vec<LakeUpdatePreparedInput>,
    pub error: Option<String>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct LakeUpdatePreparations {
    pub items: Vec<LakeUpdatePreparation>,
}

#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LakeUpdateSite {
    Danbooru,
    Yandere,
    Gelbooru,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LakeUpdateAction {
    Pause,
    Resume,
    Retry,
    Replay,
    Cancel,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LakeUpdateJobState {
    Queued,
    Running,
    Paused,
    Cancelled,
    Completed,
    CompletedWithExclusions,
    WaitingRetry,
    WaitingSpace,
    WaitingCredentials,
    NeedsReview,
}

#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LakeUpdateRange {
    Tags {
        query: LakeTagQuery,
        start_id: Option<u64>,
        end_id: Option<u64>,
    },
    Input {
        input_id: String,
    },
    Ids {
        ids: Vec<u64>,
    },
    IdRange {
        start: u64,
        end: u64,
    },
    New {
        after_id: Option<u64>,
    },
    Changes {
        after: u64,
        start_id: Option<u64>,
        end_id: Option<u64>,
    },
    Created {
        start: String,
        end: String,
        timezone: String,
        start_id: Option<u64>,
        end_id: Option<u64>,
    },
    Updated {
        start: String,
        end: String,
        timezone: String,
        start_id: Option<u64>,
        end_id: Option<u64>,
    },
    Local {
        start_id: Option<u64>,
        end_id: Option<u64>,
        observed_before: Option<String>,
        missing_media: Option<bool>,
    },
}
#[derive(Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct LakeTagQuery {
    pub all: Vec<String>,
    pub any: Vec<String>,
    pub none: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LakeImageProfile {
    MetadataOnly,
    Original,
    #[serde(rename = "webp-2048-q95")]
    Webp2048Q95,
    Custom,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LakeImageFormat {
    Webp,
    Jpeg,
    Png,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LakeImageAnimation {
    Preserve,
    FirstFrame,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LakeImageAlpha {
    Preserve,
    Flatten,
    Reject,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LakeImageEncoding {
    pub version: u32,
    pub format: LakeImageFormat,
    pub max_edge: Option<u32>,
    pub animation: LakeImageAnimation,
    pub alpha: LakeImageAlpha,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lossless: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optimize: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subsampling: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compress_level: Option<u8>,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LakeImagePolicy {
    pub profile: LakeImageProfile,
    #[serde(default)]
    pub allow_sample: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub existing: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<LakeImageEncoding>,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LakeUpdateDefinition {
    pub library_id: String,
    pub range: LakeUpdateRange,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media: Option<LakeImagePolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_budget: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_budget: Option<u32>,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LakeSitePipeline {
    pub download_concurrency: u32,
    pub api_requests_per_second: f64,
    pub image_requests_per_second: Option<f64>,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LakePipelineConfig {
    pub version: u32,
    pub scan_mode: String,
    pub active_lakes: u32,
    pub sites: BTreeMap<String, LakeSitePipeline>,
    pub encode_concurrency: u32,
    pub buffer_images: u32,
    pub metadata_prefetch_records: u32,
    pub spool_mib: u64,
    pub reserve_mib: u64,
    pub max_download_mib: u64,
    pub max_image_pixels: u64,
    pub decode_memory_mib: u64,
    pub publish_items: u32,
    pub publish_mib: u64,
    pub publish_interval_seconds: f64,
    pub download_mib_per_second: Option<f64>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct LakePipelineSettings {
    pub revision: u64,
    pub value: LakePipelineConfig,
    pub defaults: LakePipelineConfig,
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveLakePipelineSettings {
    pub expected_revision: u64,
    pub value: LakePipelineConfig,
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfigureLakeUpdates {
    pub python: String,
    /// Accepted for older clients; worker sources always come from Studio.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store_root: Option<String>,
    pub state_root: String,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct LakeUpdateServiceStatus {
    pub configured: bool,
    pub protocol_version: u32,
    pub worker_recent: bool,
    pub runtime: LakeUpdateRuntimeHealth,
    pub credentials: Vec<LakeCredentialStatus>,
    #[serde(default)]
    pub activity: LakeUpdateActivity,
    #[serde(default)]
    pub preparation_count: u64,
    #[serde(default)]
    pub preparation_attention_count: u64,
    #[serde(default)]
    pub preparations: Vec<LakeUpdatePreparation>,
}
#[derive(Default, Serialize, Deserialize, ToSchema)]
pub struct LakeUpdateRuntimeHealth {
    pub state: String,
    pub python: Option<String>,
    pub state_root: Option<String>,
    pub error_code: Option<String>,
    pub message: Option<String>,
    pub failures: u32,
    pub next_retry_ms: Option<u64>,
}
#[derive(Default, Serialize, Deserialize, ToSchema)]
pub struct LakeUpdateActivity {
    pub counts: Vec<LakeUpdateCount>,
    pub active: Vec<LakeUpdateJob>,
    pub attention: Vec<LakeUpdateJob>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct LakeUpdateCount {
    pub lake_id: String,
    pub state: LakeUpdateJobState,
    pub n: u64,
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RegisterUpdateLake {
    pub library_id: String,
    pub site: LakeUpdateSite,
    pub media_root: String,
    pub index_root: String,
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateUpdateLake {
    pub request_key: String,
    pub site: LakeUpdateSite,
    pub media_root: String,
    pub index_root: String,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct UpdateLake {
    pub id: String,
    pub site: LakeUpdateSite,
    pub media: String,
    pub index_root: String,
    pub registered_at: String,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct UpdateLakes {
    pub items: Vec<UpdateLake>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct LakeRelocation {
    pub id: String,
    pub lake_id: String,
    pub phase: String,
    pub old_media: String,
    pub old_index: String,
    pub media_root: Option<String>,
    pub index_root: Option<String>,
    pub created_at: String,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct LakeRelocations {
    pub items: Vec<LakeRelocation>,
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PrepareLakeRelocation {
    pub library_id: String,
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplyLakeRelocation {
    pub media_root: String,
    pub index_root: String,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct LakeUpdateCapability {
    pub site: LakeUpdateSite,
    pub adapter_version: u32,
    pub page_size: u32,
    pub id_ranges: bool,
    pub id_lists: bool,
    #[serde(default)]
    pub tag_queries: bool,
    pub created_range: String,
    pub updated_range: bool,
    pub change_sequence: bool,
    pub full_change_history: bool,
    pub deletion_discovery: String,
    pub credential_set: bool,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct LakeUpdateCapabilities {
    pub items: Vec<LakeUpdateCapability>,
}
/// No Debug implementation: key material must never enter tracing.
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(tag = "site", rename_all = "snake_case", deny_unknown_fields)]
pub enum SetLakeCredentials {
    Danbooru { login: String, api_key: String },
    Gelbooru { user_id: String, api_key: String },
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct LakeCredentialStatus {
    pub site: LakeUpdateSite,
    pub credential_set: bool,
    pub revision: Option<u64>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct LakeApiProbe {
    pub site: LakeUpdateSite,
    pub status: u32,
    pub records: u32,
    pub range_verified: bool,
    pub fields: Vec<String>,
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateLakeUpdate {
    pub request_key: String,
    pub definition: LakeUpdateDefinition,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct LakeUpdatePreview {
    pub definition: LakeUpdateDefinition,
    pub known_candidates: Option<u64>,
    pub scan_strategy: String,
    pub note: Option<String>,
}
#[derive(Default, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct LakeUpdateProgress {
    pub initialized: bool,
    pub metadata_complete: bool,
    pub pages: u64,
    pub slice_pages: u64,
    pub slice_items: u64,
    pub next_id: Option<u64>,
    pub upper: Option<u64>,
    pub position: Option<u64>,
    pub baseline: Option<u64>,
    pub input_seq: Option<u64>,
    pub change_through: Option<u64>,
    pub scope: Option<String>,
    pub completed_at: Option<String>,
    pub tag_anchors: Vec<String>,
    pub tag_branch: u64,
    pub metadata_records: u64,
    pub matched_records: u64,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct LakeUpdateJob {
    pub id: String,
    pub lake_id: String,
    pub request_key: String,
    pub definition: LakeUpdateDefinition,
    pub state: LakeUpdateJobState,
    pub cursor: LakeUpdateProgress,
    pub counts: BTreeMap<String, u64>,
    pub created_at: String,
    pub updated_at: String,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub retry_at: f64,
    pub execution: u64,
    #[serde(default)]
    pub execution_active: bool,
    #[serde(default)]
    pub cleanup: Option<LakeUpdateCleanup>,
    #[serde(default)]
    pub telemetry: LakeUpdateTelemetry,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct LakeUpdateCleanup {
    pub phase: LakeUpdateCleanupPhase,
    pub retry_at: f64,
    pub error_code: Option<String>,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LakeUpdateCleanupPhase {
    Pending,
    Reconciled,
    Complete,
}

#[derive(Default, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct LakeUpdateTelemetry {
    pub phase: Option<String>,
    pub current_post_id: Option<u64>,
    pub current_bytes: Option<u64>,
    pub current_total_bytes: Option<u64>,
    pub downloaded_bytes: Option<u64>,
    pub metadata_bytes: Option<u64>,
    pub download_rate_bps: Option<f64>,
    pub sampled_at: Option<String>,
    pub publish_rate_images_per_second: Option<f64>,
    pub rate_window_seconds: Option<f64>,
    pub api_requests: Option<u64>,
    pub image_requests: Option<u64>,
    pub throttled_requests: Option<u64>,
    pub resumed_requests: Option<u64>,
    pub transport_failures: Option<u64>,
    pub recovery_count: Option<u64>,
    pub last_recovery_at: Option<String>,
    pub last_recovery_reason: Option<String>,
    pub last_transfer_error: Option<LakeTransferError>,
    pub active_downloads: Option<u32>,
    pub waiting_download: Option<u64>,
    pub active_encodes: Option<u32>,
    pub waiting_encode: Option<u32>,
    pub ready_images: Option<u32>,
    pub publishing_images: Option<u32>,
    pub waiting_staging: Option<u32>,
    pub metadata_active: Option<bool>,
    pub metadata_retries: Option<u64>,
    pub metadata_retry_at: Option<f64>,
    pub metadata_error_code: Option<String>,
    pub metadata_error_message: Option<String>,
    pub staging_bytes: Option<u64>,
    pub staging_reserved_bytes: Option<u64>,
    pub staging_limit_bytes: Option<u64>,
    pub decode_reserved_bytes: Option<u64>,
    pub decode_limit_bytes: Option<u64>,
    pub timings_seconds: BTreeMap<String, f64>,
    pub files: Vec<LakeImageTransfer>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct LakeImageTransfer {
    pub post_id: u64,
    pub phase: String,
    pub current_bytes: Option<u64>,
    pub current_total_bytes: Option<u64>,
    pub resume_from: Option<u64>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct LakeTransferError {
    pub post_id: u64,
    pub exception: String,
    pub received_bytes: u64,
    pub resumable_bytes: u64,
    pub elapsed_seconds: f64,
    pub at: String,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct LakeUpdateJobs {
    pub items: Vec<LakeUpdateJob>,
    pub next_cursor: Option<String>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct LakeUpdateItem {
    pub post_id: u64,
    pub observation_id: Option<String>,
    pub state: String,
    pub reason: Option<String>,
    pub asset_id: Option<String>,
    pub attempts: u32,
    pub retry_at: f64,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct LakeUpdateItems {
    pub items: Vec<LakeUpdateItem>,
    pub next_cursor: Option<u64>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct LakeUpdateCoverage {
    pub job_id: String,
    pub state: LakeUpdateJobState,
    pub coverage: Option<Value>,
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LakeUpdateActionRequest {
    pub action: LakeUpdateAction,
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveLakeUpdateSchedule {
    pub spec: LakeUpdateDefinition,
    pub every_seconds: Option<u32>,
    pub first_run_at: String,
    #[serde(default)]
    pub enabled: bool,
    pub identity: Option<String>,
    pub revision: Option<u64>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct LakeUpdateScheduleSaved {
    pub id: String,
    pub revision: u64,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct LakeUpdateSchedule {
    pub id: String,
    pub definition: LakeUpdateDefinition,
    pub every_seconds: Option<u32>,
    pub next_run_at: String,
    pub enabled: bool,
    pub revision: u64,
    pub last_job: Option<String>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct LakeUpdateSchedules {
    pub items: Vec<LakeUpdateSchedule>,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateLakeUpdateInput {
    pub library_id: String,
    pub source_version: Option<String>,
    pub provenance: Option<Value>,
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AppendLakeUpdateInput {
    pub post_ids: Option<Vec<u64>>,
    pub object_sha256s: Option<Vec<String>>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct LakeUpdateInput {
    pub id: String,
    pub lake_id: String,
    pub state: String,
    pub source_version: String,
    pub provenance: Value,
    pub created_at: String,
    pub count: u64,
    pub sha256: Option<String>,
}
