//! Public offline-analysis DTOs. Domain conversion remains fallible and bounded.
use crate::AssetKey;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Serialize, Deserialize, ToSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AestheticEstimator {
    /// davidson_v1 (batch-normalized composite objective) or borda_v1 (baseline).
    pub kind: String,
    pub iterations: u32,
    pub regularization: f64,
    pub tie_strength: f64,
}
#[derive(Clone, Serialize, Deserialize, ToSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AestheticFit {
    pub stage_id: String,
    pub estimator: AestheticEstimator,
    /// Refit two deterministic, disjoint sets of whole batches. Not a confidence interval.
    pub stability_seed: Option<u32>,
}
#[derive(Clone, Serialize, Deserialize, ToSchema, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AestheticRankingFilter {
    #[serde(default)]
    pub ratings: Vec<String>,
    pub component: Option<u64>,
    pub top_percent: Option<f64>,
    pub rank_from: Option<u64>,
    pub rank_to: Option<u64>,
    pub year_from: Option<i32>,
    pub year_to: Option<i32>,
    pub max_exposures: Option<u32>,
    pub min_split_delta: Option<f64>,
    #[serde(default)]
    pub protected_only: bool,
    #[serde(default)]
    pub needs_review: bool,
    /// Union protected candidates into the selection after quality filters;
    /// Rating and year restrictions still apply.
    #[serde(default)]
    pub include_protected: bool,
}
#[derive(Clone, Serialize, Deserialize, ToSchema, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AestheticAnalysisSpec {
    Fit {
        config: AestheticFit,
        experiment_id: Option<String>,
        variant: Option<String>,
    },
    Compare {
        left: String,
        right: String,
    },
    Derive {
        snapshot_id: String,
        filter: AestheticRankingFilter,
        review_watermark: Option<u64>,
    },
}
#[derive(Clone, Serialize, Deserialize, ToSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AestheticAnalysisCreate {
    pub idempotency_key: String,
    pub name: String,
    pub spec: AestheticAnalysisSpec,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct AestheticAnalysisInput {
    pub stage_id: String,
    pub stage_config_hash: String,
    pub evidence_watermark: u64,
    pub observations: u64,
    pub candidates: u64,
    pub review_watermark: u64,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct AestheticAnalysisJob {
    pub id: String,
    pub created_at: String,
    pub state: String,
    pub phase: String,
    pub progress: u64,
    pub total: u64,
    pub request: AestheticAnalysisCreate,
    pub input: AestheticAnalysisInput,
    pub result: Option<AestheticAnalysisSummary>,
    pub error: Option<String>,
}
#[derive(Clone, Serialize, Deserialize, ToSchema, Default)]
pub struct AestheticRatingSummary {
    pub rating: String,
    pub candidates: u64,
    pub judged: u64,
    pub compared: u64,
    pub components: u64,
    pub protected: u64,
    pub cross_year_batches: u64,
    pub split_comparable: u64,
    pub fully_connected: bool,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct AestheticFitSummary {
    pub estimator_version: String,
    pub groups: Vec<AestheticRatingSummary>,
    pub iterations_completed: u32,
    pub converged: bool,
    pub max_update: f64,
    pub working_bytes_estimate: u64,
    pub stability_method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validity: Option<AestheticValidity>,
}
#[derive(Clone, Serialize, Deserialize, ToSchema, Default)]
pub struct AestheticComparisonGroup {
    pub rating: String,
    pub matched: u64,
    pub comparable: bool,
    pub rank_correlation: Option<f64>,
    pub mean_absolute_percentile_delta: Option<f64>,
    pub middle_mean_absolute_percentile_delta: Option<f64>,
    pub top20_jaccard: Option<f64>,
    pub elite_disagreements: u64,
    pub reason: Option<String>,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AestheticAnalysisSummary {
    Fit(AestheticFitSummary),
    Compare {
        groups: Vec<AestheticComparisonGroup>,
    },
    Derive {
        collection_id: String,
        count: u64,
    },
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct AestheticRankingRow {
    /// Pagination position only, never a cross-Rating/global aesthetic rank.
    pub position: u64,
    pub ordinal: u64,
    pub key: AssetKey,
    pub rating: String,
    pub year: Option<i32>,
    pub content_version: String,
    pub score: Option<f64>,
    pub component: Option<u64>,
    pub component_size: u64,
    pub rank_min: Option<u64>,
    pub rank_max: Option<u64>,
    pub rating_rank_min: Option<u64>,
    pub rating_rank_max: Option<u64>,
    pub percentile: Option<f64>,
    pub component_percentile: Option<f64>,
    pub exposures: u32,
    pub unjudgeable: u32,
    pub protected: bool,
    pub opponent_bins: u32,
    pub opponent_diversity_estimate: Option<f64>,
    pub cross_year_exposures: u32,
    pub split_percentile_delta: Option<f64>,
    pub disagreement: Option<f64>,
    pub needs_review: bool,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct AestheticComparisonRow {
    pub position: u64,
    pub key: AssetKey,
    pub rating: String,
    pub comparable: bool,
    pub left_percentile: Option<f64>,
    pub right_percentile: Option<f64>,
    pub percentile_delta: Option<f64>,
    pub left_protected: bool,
    pub right_protected: Option<bool>,
    pub reason: Option<String>,
}
#[derive(Clone, Serialize, Deserialize, ToSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AestheticExperimentVariant {
    pub label: String,
    pub fit: AestheticFit,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AestheticRankingQuery {
    pub filter: AestheticRankingFilter,
    pub after: Option<String>,
    pub limit: Option<u32>,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct AestheticSelectionRow {
    pub ranking: AestheticRankingRow,
    pub effective_protected: bool,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct AestheticRankingSelection {
    pub items: Vec<AestheticSelectionRow>,
    pub next_cursor: Option<String>,
    pub review_watermark: u64,
    pub scanned: u64,
}
#[derive(Clone, Serialize, Deserialize, ToSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AestheticExperimentCreate {
    pub idempotency_key: String,
    pub name: String,
    pub description: String,
    pub variants: Vec<AestheticExperimentVariant>,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct AestheticExperiment {
    pub id: String,
    pub created_at: String,
    pub request: AestheticExperimentCreate,
    pub inputs: Vec<AestheticAnalysisInput>,
}
#[derive(Clone, Serialize, Deserialize, ToSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AestheticReviewCreate {
    pub idempotency_key: String,
    pub snapshot_id: String,
    pub ordinal: u64,
    /// protect, confirm_elite, release, defer. Decisions never change statistical scores.
    pub decision: String,
    pub reviewer: String,
    pub reason: String,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct AestheticReview {
    pub sequence: u64,
    pub created_at: String,
    pub request: AestheticReviewCreate,
}

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct AestheticValidity {
    pub version: u32,
    pub numerical: String,
    pub groups: Vec<AestheticRatingValidity>,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct AestheticRatingValidity {
    pub rating: String,
    pub ranking_scope: String,
    pub coverage: String,
    pub connection: String,
    pub stability: String,
    pub compared: u64,
    pub candidates: u64,
    pub stability_covered: u64,
}
