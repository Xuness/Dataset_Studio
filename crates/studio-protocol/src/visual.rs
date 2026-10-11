use crate::AssetKey;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct VisualRuntimeConfig {
    pub python: String,
    pub materials_root: String,
    pub model_root: String,
    pub device: String,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct VisualRuntimeStatus {
    pub configured: bool,
    pub config: Option<VisualRuntimeConfig>,
    pub worker_running: bool,
    pub busy: bool,
    pub description: Option<VisualModelDescription>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct VisualRecipe {
    pub id: String,
    pub model: String,
    pub dimension: u32,
    pub spec: serde_json::Value,
    pub path: String,
    pub weights: String,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct VisualRecipes {
    pub items: Vec<VisualRecipe>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct VisualModelDescription {
    pub recipes: Vec<VisualRecipe>,
    pub device: String,
    pub device_name: String,
    pub packages: std::collections::BTreeMap<String, String>,
    pub hash_recipe: String,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct VisualSource {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub index_root: Option<String>,
    pub media_root: Option<String>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct VisualSources {
    pub items: Vec<VisualSource>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct VisualSourceVersion {
    pub source_id: String,
    pub catalog_revision: String,
    pub analysis_sequence: Option<String>,
    pub consistency: String,
    pub semantics_version: Option<String>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct VisualSourceSnapshot {
    pub source: VisualSource,
    pub version: VisualSourceVersion,
    pub total_objects: u64,
    pub after_row: u64,
    pub discovered: u64,
    pub complete: bool,
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct VisualRunRequest {
    pub source_ids: Vec<String>,
    /// Immutable recipe IDs from /visual/models or /visual/recipes. Empty runs pHash only.
    pub recipe_ids: Vec<String>,
    pub max_objects_per_source: Option<u64>,
    #[serde(default = "batch_size")]
    pub batch_size: u32,
}
fn batch_size() -> u32 {
    256
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct VisualRun {
    pub id: String,
    pub ordinal: u64,
    pub status: String,
    pub request: VisualRunRequest,
    pub sources: Vec<VisualSourceSnapshot>,
    pub planned_objects: u64,
    pub completed_objects: u64,
    /// Already covered source bindings skipped during catalog planning.
    pub reused_bindings: u64,
    pub failed_features: u64,
    pub bytes_read: u64,
    pub error: Option<String>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct VisualRuns {
    pub items: Vec<VisualRun>,
    pub next_cursor: Option<u64>,
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct VisualAction {
    pub action: String,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct VisualCoverage {
    pub source_id: String,
    pub recipe_id: String,
    /// Objects inventoried by visual jobs. This is not a whole-lake count.
    pub discovered: u64,
    pub succeeded: u64,
    pub failed: u64,
    pub pending: u64,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct VisualCoverageResponse {
    pub items: Vec<VisualCoverage>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct VisualFailure {
    pub object_id: i64,
    pub recipe_id: String,
    pub status: String,
    pub error: Option<String>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct VisualFailures {
    pub items: Vec<VisualFailure>,
    pub next_cursor: Option<i64>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct VisualMatch {
    pub object_id: i64,
    pub sha256: String,
    pub assets: Vec<AssetKey>,
    pub distance: Option<f64>,
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct VisualSearchRequest {
    pub recipe_id: String,
    pub asset: Option<AssetKey>,
    pub vector: Option<Vec<f32>>,
    #[serde(default)]
    pub source_ids: Vec<String>,
    pub limit: u32,
    #[serde(default)]
    pub exact: bool,
    pub nprobes: Option<u32>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct VisualSearchResult {
    pub recipe_id: String,
    pub version: u64,
    pub method: String,
    pub score_type: String,
    pub items: Vec<VisualMatch>,
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct VisualDuplicatesRequest {
    /// Omit for cross-source SHA groups; supply for pHash neighbours.
    pub asset: Option<AssetKey>,
    pub after: Option<i64>,
    pub limit: u32,
    pub radius: Option<u32>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct VisualDuplicates {
    pub kind: String,
    pub recipe_id: String,
    pub items: Vec<VisualMatch>,
    /// Resume even an empty page: crowded hash buckets are examined in bounded pages.
    pub next_cursor: Option<i64>,
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct VisualIndexRequest {
    pub recipe_id: String,
    pub partitions: Option<u32>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct VisualIndexResult {
    pub version: u64,
    pub rows: u64,
    pub partitions: u32,
    pub index_type: String,
}
