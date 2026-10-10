use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PinterestSeed {
    pub kind: String,
    pub id: String,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PinterestAccess {
    pub mode: String,
    pub language: String,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PinterestScope {
    pub media_types: Vec<String>,
    pub ai_policy: String,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PinterestDiscovery {
    pub entrypoints: Vec<String>,
    pub max_depth: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_sections: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_pending_downloads: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_requests: Option<std::collections::BTreeMap<String, u32>>,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PinterestMetadataPlan {
    pub detail_enrichment: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample_size: Option<u32>,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PinterestImagePolicy {
    pub profile: String,
    pub existing: String,
    pub allow_sample: bool,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PinterestReuse {
    pub mode: String,
    pub max_age_hours: u32,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PinterestMediaPlan {
    pub image_policy: PinterestImagePolicy,
    pub retain_original: bool,
    pub reuse: PinterestReuse,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PinterestBudget {
    pub api_requests: u64,
    pub admitted_pins: u64,
    pub admitted_boards: u64,
    pub detail_requests: u64,
    pub download_bytes: u64,
    pub wall_seconds: u64,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PinterestDefinition {
    pub version: u32,
    pub collector: String,
    pub library_id: String,
    pub seeds: Vec<PinterestSeed>,
    pub access: PinterestAccess,
    pub scope: PinterestScope,
    pub discovery: PinterestDiscovery,
    pub metadata: PinterestMetadataPlan,
    pub media: PinterestMediaPlan,
    pub run_budget: PinterestBudget,
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreatePinterestJob {
    pub request_key: String,
    pub definition: PinterestDefinition,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct PinterestPreview {
    pub definition: PinterestDefinition,
    pub definition_sha256: String,
    pub known_pins: u32,
    pub network_requests: u32,
    pub warnings: Vec<String>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct PinterestCapabilities {
    pub contract_version: u32,
    pub site: String,
    pub collector: String,
    pub seed_kinds: Vec<String>,
    pub media_types: Vec<String>,
    pub access_modes: Vec<String>,
    pub archive_format: u32,
    pub online_format: u32,
    pub discovery: bool,
    pub schedules: bool,
    pub image_profiles: Vec<String>,
    pub max_seeds: u32,
    pub entrypoints: Vec<String>,
    pub detail_enrichment_modes: Vec<String>,
    pub reuse_modes: Vec<String>,
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SavePinterestSchedule {
    pub request_key: String,
    pub id: String,
    pub expected_revision: u64,
    pub definition: PinterestDefinition,
    pub every_seconds: u64,
    pub first_run_at: String,
    pub enabled: bool,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct PinterestSchedule {
    pub id: String,
    pub definition: PinterestDefinition,
    pub every_seconds: u64,
    pub next_run_at: String,
    pub enabled: bool,
    pub revision: u64,
    pub last_job: Option<String>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct PinterestSchedules {
    pub items: Vec<PinterestSchedule>,
    pub next_cursor: Option<String>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct PinterestCount {
    pub kind: String,
    pub state: String,
    pub n: u64,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct PinterestJob {
    pub id: String,
    pub library_id: String,
    pub definition: PinterestDefinition,
    pub definition_sha256: String,
    pub state: String,
    pub desired_state: String,
    pub revision: u64,
    pub execution_epoch: u64,
    pub created_at: String,
    pub updated_at: String,
    pub retry_at: f64,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub api_requests: u64,
    pub detail_requests: u64,
    pub budget_round: u64,
    pub budget_usage: std::collections::BTreeMap<String, f64>,
    pub totals: std::collections::BTreeMap<String, f64>,
    pub metrics: std::collections::BTreeMap<String, u64>,
    pub media_complete: bool,
    pub enrichment_pending: u64,
    pub download_bytes: u64,
    pub elapsed_seconds: f64,
    pub archive_seq: u64,
    pub served_seq: u64,
    pub cleanup_state: String,
    pub counts: Vec<PinterestCount>,
    pub phase: String,
    pub actions: Vec<String>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct PinterestItem {
    pub task_id: String,
    pub kind: String,
    pub pin_id: String,
    pub state: String,
    pub attempts: u64,
    pub reason: Option<String>,
    pub updated_at: String,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct PinterestJobs {
    pub items: Vec<PinterestJob>,
    pub next_cursor: Option<String>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct PinterestItems {
    pub items: Vec<PinterestItem>,
    pub next_cursor: Option<String>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct PinterestStream {
    pub scan_id: String,
    pub entrypoint: String,
    pub subject_id: String,
    pub root: PinterestSeed,
    pub depth: u32,
    pub state: String,
    pub reason: Option<String>,
    pub pages: u64,
    pub members: u64,
    pub unique_pins: u64,
    pub total: Option<u64>,
    pub force_detail: u32,
    pub samples_checked: u64,
    pub mismatches: u64,
    pub has_cursor: bool,
    pub updated_at: String,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct PinterestStreams {
    pub items: Vec<PinterestStream>,
    pub next_cursor: Option<String>,
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PinterestJobAction {
    pub action: String,
    pub expected_revision: u64,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct PinterestStatus {
    pub counts: std::collections::BTreeMap<String, u64>,
    pub active: Vec<PinterestJob>,
}

#[derive(Serialize, Deserialize, ToSchema, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in=Query)]
pub struct PinterestItemsQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}
