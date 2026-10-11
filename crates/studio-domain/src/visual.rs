//! Shared visual materials are application resources, independent of projects.
use crate::{QuerySourceVersion, Source};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const HASH_RECIPE: &str = "phash64-dct32-white-v1";
pub const MEDIA_BATCH_BYTES: u64 = 64 << 20;
pub const DECODE_BATCH_BYTES: u64 = 512 << 20;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisualRuntimeConfig {
    pub python: PathBuf,
    pub materials_root: PathBuf,
    pub model_root: PathBuf,
    pub device: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisualRecipe {
    pub id: String,
    pub model: String,
    pub dimension: u32,
    pub spec: serde_json::Value,
    pub path: String,
    pub weights: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisualRunRequest {
    pub source_ids: Vec<String>,
    pub recipe_ids: Vec<String>,
    pub max_objects_per_source: Option<u64>,
    #[serde(default = "batch_size")]
    pub batch_size: u32,
}
fn batch_size() -> u32 {
    256
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisualSourceSnapshot {
    pub source: Source,
    pub version: QuerySourceVersion,
    pub total_objects: u64,
    pub after_row: u64,
    pub discovered: u64,
    pub complete: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisualRun {
    pub id: String,
    pub ordinal: u64,
    pub status: String,
    pub request: VisualRunRequest,
    pub sources: Vec<VisualSourceSnapshot>,
    pub planned_objects: u64,
    pub completed_objects: u64,
    pub reused_bindings: u64,
    pub failed_features: u64,
    pub bytes_read: u64,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisualMediaRef {
    pub row_id: u64,
    pub sha256: String,
    pub pack: String,
    pub offset: u64,
    pub length: u64,
    pub extension: String,
}
#[derive(Clone, Debug)]
pub struct VisualMediaPage {
    pub items: Vec<VisualMediaRef>,
    pub complete: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisualPlanItem {
    pub object_id: i64,
    pub sha256: String,
    pub source_ids: Vec<i64>,
    pub source_id: String,
    pub media_root: PathBuf,
    pub pack: String,
    pub offset: u64,
    pub length: u64,
    pub hash_done: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisualBatchRequest {
    pub batch_id: String,
    pub items: Vec<VisualPlanItem>,
    pub recipes: Vec<VisualRecipe>,
    pub projection_recipes: Vec<VisualRecipe>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisualOutcome {
    pub object_id: i64,
    pub recipe_id: String,
    pub status: String,
    pub hash: Option<String>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisualBatchReceipt {
    pub batch_id: String,
    pub object_ids: Vec<i64>,
    pub results: Vec<VisualOutcome>,
    pub versions: std::collections::BTreeMap<String, u64>,
    pub bytes_read: u64,
    pub elapsed_ms: u64,
    #[serde(default)]
    pub model_images: std::collections::BTreeMap<String, u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisualCoverage {
    pub source_id: String,
    pub recipe_id: String,
    /// Catalog objects discovered by visual runs, not the whole lake.
    pub discovered: u64,
    pub succeeded: u64,
    pub failed: u64,
    pub pending: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisualMatch {
    pub object_id: i64,
    pub sha256: String,
    pub assets: Vec<crate::AssetKey>,
    pub distance: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisualSearchRequest {
    pub recipe_id: String,
    pub asset: Option<crate::AssetKey>,
    pub vector: Option<Vec<f32>>,
    #[serde(default)]
    pub source_ids: Vec<String>,
    pub limit: u32,
    #[serde(default)]
    pub exact: bool,
    pub nprobes: Option<u32>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisualSearchResult {
    pub recipe_id: String,
    pub version: u64,
    pub method: String,
    pub score_type: String,
    pub items: Vec<VisualMatch>,
}
