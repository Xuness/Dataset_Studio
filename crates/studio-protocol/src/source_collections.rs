use crate::{
    LakeImagePolicy, LakePipelineConfig, LakeSitePipeline, LakeTagQuery, LakeUpdateRuntimeHealth,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use utoipa::{IntoParams, ToSchema};

#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CollectionAccountMode {
    Anonymous,
    Session,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CollectionSeedKind {
    Authors,
    Works,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CollectionJobState {
    Queued,
    Running,
    Pausing,
    Paused,
    WaitingCredentials,
    WaitingRetry,
    WaitingResources,
    WaitingBudget,
    Publishing,
    NeedsReview,
    Cancelling,
    Cancelled,
    Completed,
    CompletedWithGaps,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CollectionAction {
    Pause,
    Resume,
    RetryFailed,
    Cancel,
    ReplayPublication,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CollectionSeeds {
    pub kind: CollectionSeedKind,
    pub ids: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CollectionScope {
    pub work_types: Vec<String>,
    pub ratings: Vec<String>,
    pub include_ai: bool,
    pub include_unknown_markers: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<LakeTagQuery>,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CollectionDiscovery {
    pub entrypoints: Vec<String>,
    pub max_depth: u32,
    pub recommendation_seeds_per_author: u32,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CollectionReuse {
    pub mode: String,
    pub max_age_hours: u32,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CollectionMediaPlan {
    pub image_policy: LakeImagePolicy,
    pub retain_original: bool,
    pub ugoira: String,
    pub reuse: CollectionReuse,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CollectionRunBudget {
    pub api_requests: u64,
    pub admitted_authors: u64,
    pub download_bytes: u64,
    pub wall_seconds: u64,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CollectionRefresh {
    pub mode: String,
    pub max_age_hours: u32,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CollectionJobDefinition {
    pub version: u32,
    pub collector: String,
    pub library_id: String,
    pub account_id: String,
    pub seeds: CollectionSeeds,
    pub scope: CollectionScope,
    pub discovery: CollectionDiscovery,
    pub media: CollectionMediaPlan,
    pub run_budget: CollectionRunBudget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh: Option<CollectionRefresh>,
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateCollectionJob {
    pub request_key: String,
    pub definition: CollectionJobDefinition,
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateCollectionLake {
    pub request_key: String,
    pub site: String,
    pub media_root: String,
    pub index_root: String,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct CollectionLake {
    pub library_id: String,
    pub site: String,
    pub media_root: String,
    pub index_root: String,
    pub archive_format: u32,
    pub online_format: u32,
    pub collector: String,
    pub state: String,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CollectionCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub expires_unix: Option<u64>,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveCollectionAccount {
    pub request_key: String,
    pub expected_revision: Option<u64>,
    pub account_id: String,
    pub label: String,
    pub mode: CollectionAccountMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cookies: Option<Vec<CollectionCookie>>,
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CollectionRevisionCommand {
    pub request_key: String,
    pub expected_revision: u64,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct CollectionAccount {
    pub id: String,
    pub site: String,
    pub label: String,
    pub mode: CollectionAccountMode,
    pub revision: u64,
    pub state: String,
    pub credential_set: bool,
    pub bound_user_id: Option<String>,
    pub last_probe_at: Option<String>,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct CollectionVisibility {
    pub context_id: String,
    pub observed_at: String,
    pub login: String,
    pub r18: String,
    pub r18g: String,
    pub ai_display: String,
    pub coverage_verified: bool,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct CollectionAccountProbe {
    pub account: CollectionAccount,
    pub visibility: CollectionVisibility,
}

/// Native login bridge: no Cookie or browser profile is exposed to the UI.
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct StartCollectionLogin {
    pub request_key: String,
    pub account_id: String,
    pub expected_revision: Option<u64>,
    pub label: String,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CollectionLoginPhase {
    Waiting,
    Verifying,
    Unconfirmed,
    Succeeded,
    Cancelled,
    Expired,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct CollectionLoginError {
    pub code: String,
    pub message: String,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct CollectionLoginSession {
    pub id: String,
    pub account_id: String,
    pub label: String,
    pub phase: CollectionLoginPhase,
    pub window_open: bool,
    pub result: Option<CollectionAccountProbe>,
    pub error: Option<CollectionLoginError>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct CollectionLoginStatus {
    pub available: bool,
    pub session: Option<CollectionLoginSession>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct CollectionIssue {
    pub code: String,
    pub message: String,
    pub severity: String,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct CollectionPreview {
    pub definition: CollectionJobDefinition,
    pub known_seed_count: u64,
    pub known_work_count: Option<u64>,
    pub known_media_count: Option<u64>,
    pub issues: Vec<CollectionIssue>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct CollectionAuthorProgress {
    pub discovered: u64,
    pub admitted: u64,
    pub scanned: u64,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct CollectionWorkProgress {
    pub planned: Option<u64>,
    pub details: u64,
    pub gaps: u64,
    pub retained: u64,
    pub excluded: u64,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct CollectionMediaProgress {
    pub planned: Option<u64>,
    pub downloaded: u64,
    pub historical_reused: u64,
    pub http_validated: u64,
    pub archived: u64,
    pub published: u64,
    pub gaps: u64,
    pub retained: u64,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct CollectionObjectProgress {
    pub stored: u64,
    pub browsable_images: u64,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct CollectionPublicationProgress {
    pub archive_seq: u64,
    pub served_seq: u64,
    pub pending_batches: u64,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct CollectionClosure {
    pub discovery_exhausted: bool,
    pub directories_complete: bool,
    pub manifests_complete: bool,
    pub visibility_verified: bool,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct CollectionProgress {
    pub authors: CollectionAuthorProgress,
    pub works: CollectionWorkProgress,
    pub media: CollectionMediaProgress,
    pub objects: CollectionObjectProgress,
    pub download_bytes: u64,
    pub publication: CollectionPublicationProgress,
    pub closure: CollectionClosure,
    pub access_mode: String,
    pub directory_delta: CollectionDirectoryDelta,
    pub task_gaps: u64,
    pub budget: CollectionBudgetProgress,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct CollectionDirectoryDelta {
    pub added: u64,
    pub no_longer_listed: u64,
    pub unchanged: u64,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct CollectionBudgetProgress {
    pub limits: CollectionRunBudget,
    pub used: BTreeMap<String, f64>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct CollectionJob {
    pub id: String,
    pub library_id: String,
    pub account_id: String,
    pub definition: CollectionJobDefinition,
    pub state: CollectionJobState,
    pub desired_state: String,
    pub revision: u64,
    pub execution_epoch: u64,
    pub execution_active: bool,
    pub wait_reason: Option<String>,
    pub progress: CollectionProgress,
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct CollectionJobResult {
    pub job: CollectionJob,
    pub replayed: bool,
    #[serde(default)]
    pub coalesced: bool,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct CollectionTask {
    pub id: String,
    pub job_id: String,
    pub kind: String,
    pub subject_key: String,
    pub state: String,
    pub attempts: u64,
    pub reason: Option<String>,
    pub retry_at_ms: u64,
    pub summary: Value,
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CollectionJobAction {
    pub request_key: String,
    pub expected_revision: u64,
    pub action: CollectionAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_ids: Option<Vec<String>>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct CollectionCoverage {
    pub job_id: String,
    pub state: String,
    pub scope: CollectionScope,
    pub closure: CollectionClosure,
    pub progress: CollectionProgress,
    pub statement: String,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CollectionPipelineValue {
    pub pixiv: LakeSitePipeline,
    pub metadata_concurrency: u32,
    pub pending_media_limit: u64,
    pub publication_backlog_mib: u64,
    pub time_slice_seconds: u32,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct CollectionPipelineSettings {
    pub revision: u64,
    pub value: CollectionPipelineValue,
    pub shared_limits: LakePipelineConfig,
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveCollectionPipeline {
    pub expected_revision: u64,
    pub value: CollectionPipelineValue,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct CollectionCapabilities {
    pub collector: String,
    pub contract_version: u32,
    pub work_types: Vec<String>,
    pub discovery_entrypoints: Vec<String>,
    pub archive_formats: Vec<u32>,
    pub online_formats: Vec<u32>,
    pub limits: BTreeMap<String, u64>,
    pub authentication_modes: Vec<String>,
    pub refresh_modes: Vec<String>,
    pub periodic_snapshots: bool,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct CollectionServiceStatus {
    pub configured: bool,
    pub protocol_version: u32,
    pub collection_contract_version: u32,
    pub runtime: LakeUpdateRuntimeHealth,
    pub counts: BTreeMap<String, u64>,
    pub active: Vec<CollectionJob>,
}
macro_rules! page {
    ($name:ident,$item:ty) => {
        #[derive(Serialize, Deserialize, ToSchema)]
        pub struct $name {
            pub items: Vec<$item>,
            pub next_cursor: Option<String>,
        }
    };
}
page!(CollectionLakes, CollectionLake);
page!(CollectionAccounts, CollectionAccount);
page!(CollectionJobs, CollectionJob);
page!(CollectionTasks, CollectionTask);

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveCollectionSchedule {
    pub request_key: String,
    pub id: String,
    pub expected_revision: u64,
    pub definition: CollectionJobDefinition,
    pub every_seconds: u64,
    pub first_run_at: String,
    pub enabled: bool,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct CollectionSchedule {
    pub id: String,
    pub definition: CollectionJobDefinition,
    pub every_seconds: u64,
    pub next_run_at: String,
    pub enabled: bool,
    pub revision: u64,
    pub last_job: Option<String>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct CollectionScheduleRemoved {
    pub removed: bool,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct LakeWorkspaceLake {
    pub id: String,
    pub site: String,
    pub media: String,
    pub index_root: String,
    pub registered_at: String,
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(tag = "family", rename_all = "snake_case")]
pub enum LakeWorkspaceJob {
    Update { job: Box<crate::LakeUpdateJob> },
    Collection { job: Box<CollectionJob> },
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(tag = "family", rename_all = "snake_case")]
pub enum LakeWorkspaceSchedule {
    Update { schedule: crate::LakeUpdateSchedule },
    Collection { schedule: CollectionSchedule },
}
page!(CollectionSchedules, CollectionSchedule);
page!(LakeWorkspaceLakes, LakeWorkspaceLake);
page!(LakeWorkspaceJobs, LakeWorkspaceJob);
page!(LakeWorkspaceSchedules, LakeWorkspaceSchedule);

#[derive(Serialize, Deserialize, ToSchema, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in=Query)]
pub struct CollectionSchedulesQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library_id: Option<String>,
}

#[derive(Serialize, Deserialize, ToSchema, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in=Query)]
pub struct CollectionPageQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}
#[derive(Serialize, Deserialize, ToSchema, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in=Query)]
pub struct CollectionJobsQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}
#[derive(Serialize, Deserialize, ToSchema, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in=Query)]
pub struct CollectionTasksQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Serialize, Deserialize, ToSchema, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in=Query)]
pub struct SourceRelationQuery {
    pub version: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
    pub manifest_id: Option<String>,
    pub recipe_id: Option<String>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct SourceWorkDetail {
    pub work_id: String,
    pub observation: Option<Value>,
    pub manifest: Option<Value>,
    pub manifest_state: String,
    pub version: String,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct SourceAuthorDetail {
    pub author_id: String,
    pub observation: Value,
    pub version: String,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct SourceAuthorWork {
    pub work_id: String,
    pub current: Option<Value>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct SourceAuthorWorks {
    pub author_id: String,
    pub snapshot_id: String,
    pub observed_at: String,
    pub context_id: String,
    pub traversal_exhausted: bool,
    pub items: Vec<SourceAuthorWork>,
    pub next_cursor: Option<String>,
    pub version: String,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct MediaBinding {
    pub record_id: String,
    pub object_sha256: String,
    pub representation: String,
    pub recipe_id: String,
    pub evidence: String,
    pub last_verified_at: Option<String>,
    pub browsable_image: bool,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct WorkMediaItem {
    pub media_id: String,
    pub work_id: String,
    pub slot_key: String,
    pub ordinal: u32,
    pub kind: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub source_variant: String,
    pub availability: String,
    pub bindings: Vec<MediaBinding>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct WorkMediaPage {
    pub work_id: String,
    pub manifest_id: Option<String>,
    pub manifest_state: String,
    pub context_id: Option<String>,
    pub observed_at: Option<String>,
    pub items: Vec<WorkMediaItem>,
    pub next_cursor: Option<String>,
    pub version: String,
}
