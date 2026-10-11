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
    #[serde(default)]
    pub request_id: Option<String>,
    pub source_ids: Vec<String>,
    pub recipe_ids: Vec<String>,
    pub max_objects_per_source: Option<u64>,
    #[serde(default = "batch_size")]
    pub batch_size: u32,
    /// Frozen membership of an earlier fully discovered run. No member copy.
    #[serde(default)]
    pub scope_run_id: Option<String>,
    /// Explicit bounded selection; mutually exclusive with scope_run_id.
    #[serde(default)]
    pub assets: Vec<crate::AssetKey>,
    /// Performance knobs, keyed by model name; do not change feature semantics.
    #[serde(default)]
    pub micro_batch_sizes: std::collections::BTreeMap<String, u32>,
    /// Continue source discovery after its durable generation-specific cursor.
    #[serde(default)]
    pub incremental: bool,
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
    #[serde(default)]
    pub discovery_namespace: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisualDiscoveryCursor {
    pub source_id: String,
    pub namespace: String,
    pub after_row: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisualIndexRequest {
    #[serde(default)]
    pub request_id: Option<String>,
    pub recipe_id: String,
    pub partitions: Option<u32>,
    #[serde(default = "index_iterations")]
    pub max_iterations: u32,
    #[serde(default = "index_sample_rate")]
    pub sample_rate: u32,
}
fn index_iterations() -> u32 {
    30
}
fn index_sample_rate() -> u32 {
    128
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct VisualIndexProgress {
    pub phase: String,
    pub completed_units: Option<u64>,
    pub total_units: Option<u64>,
    pub unit: String,
    pub updated_at_ms: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisualIndexJob {
    pub id: String,
    pub ordinal: u64,
    pub status: String,
    pub request: VisualIndexRequest,
    pub input_version: u64,
    pub progress: VisualIndexProgress,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
    pub worker_running: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisualRetentionPolicy {
    pub automatic: bool,
    pub keep_last_versions: u32,
    pub min_age_hours: u32,
}
impl Default for VisualRetentionPolicy {
    fn default() -> Self {
        Self {
            automatic: true,
            keep_last_versions: 8,
            min_age_hours: 168,
        }
    }
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
    #[serde(default)]
    pub micro_batch_sizes: std::collections::BTreeMap<String, u32>,
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
    #[serde(default)]
    pub metrics: VisualMetrics,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VisualMetrics {
    pub elapsed_ms: u64,
    pub lookup_ms: u64,
    pub read_ms: u64,
    pub sha256_ms: u64,
    pub decode_ms: u64,
    pub phash_ms: u64,
    pub write_ms: u64,
    pub model_ms: std::collections::BTreeMap<String, u64>,
    pub model_load_ms: std::collections::BTreeMap<String, u64>,
    pub model_preprocess_ms: std::collections::BTreeMap<String, u64>,
    pub model_inference_ms: std::collections::BTreeMap<String, u64>,
    pub model_images: std::collections::BTreeMap<String, u64>,
    pub model_batches: std::collections::BTreeMap<String, u64>,
    pub peak_rss_bytes: u64,
    pub peak_cuda_allocated_bytes: u64,
    pub peak_cuda_reserved_bytes: u64,
    pub opens: u64,
    pub decoded_images: u64,
    pub batches: u64,
}
impl VisualMetrics {
    pub fn accumulate(&mut self, other: &Self) {
        macro_rules! add { ($($field:ident),*) => {$(self.$field += other.$field;)*} }
        add!(
            elapsed_ms,
            lookup_ms,
            read_ms,
            sha256_ms,
            decode_ms,
            phash_ms,
            write_ms,
            opens,
            decoded_images,
            batches
        );
        macro_rules! maps { ($($field:ident),*) => {$(for (k,v) in &other.$field {
            *self.$field.entry(k.clone()).or_default() += v;
        })*} }
        maps!(
            model_ms,
            model_load_ms,
            model_preprocess_ms,
            model_inference_ms,
            model_images,
            model_batches
        );
        self.peak_rss_bytes = self.peak_rss_bytes.max(other.peak_rss_bytes);
        self.peak_cuda_allocated_bytes = self
            .peak_cuda_allocated_bytes
            .max(other.peak_cuda_allocated_bytes);
        self.peak_cuda_reserved_bytes = self
            .peak_cuda_reserved_bytes
            .max(other.peak_cuda_reserved_bytes);
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisualSnapshot {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub versions: std::collections::BTreeMap<String, u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisualFeaturePageRequest {
    pub recipe_id: String,
    pub snapshot_id: Option<String>,
    pub run_id: Option<String>,
    #[serde(default)]
    pub after: i64,
    pub limit: u32,
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
    #[serde(default)]
    pub snapshot_id: Option<String>,
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
