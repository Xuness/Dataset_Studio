use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use studio_domain as domain;
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RankingMode {
    #[default]
    Rank,
    Select,
}
impl From<RankingMode> for domain::RankingMode {
    fn from(v: RankingMode) -> Self {
        match v {
            RankingMode::Rank => Self::Rank,
            RankingMode::Select => Self::Select,
        }
    }
}
impl From<domain::RankingMode> for RankingMode {
    fn from(v: domain::RankingMode) -> Self {
        match v {
            domain::RankingMode::Rank => Self::Rank,
            domain::RankingMode::Select => Self::Select,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RankingEligibility {
    #[default]
    Eligible,
    MetadataUnavailable,
    RatingUnknown,
    RatingExcluded,
    DimensionsUnknown,
    DimensionsExcluded,
    PolicyExcluded,
    Duplicate,
}
impl From<RankingEligibility> for domain::RankingEligibility {
    fn from(v: RankingEligibility) -> Self {
        match v {
            RankingEligibility::Eligible => Self::Eligible,
            RankingEligibility::MetadataUnavailable => Self::MetadataUnavailable,
            RankingEligibility::RatingUnknown => Self::RatingUnknown,
            RankingEligibility::RatingExcluded => Self::RatingExcluded,
            RankingEligibility::DimensionsUnknown => Self::DimensionsUnknown,
            RankingEligibility::DimensionsExcluded => Self::DimensionsExcluded,
            RankingEligibility::PolicyExcluded => Self::PolicyExcluded,
            RankingEligibility::Duplicate => Self::Duplicate,
        }
    }
}
impl From<domain::RankingEligibility> for RankingEligibility {
    fn from(v: domain::RankingEligibility) -> Self {
        match v {
            domain::RankingEligibility::Eligible => Self::Eligible,
            domain::RankingEligibility::MetadataUnavailable => Self::MetadataUnavailable,
            domain::RankingEligibility::RatingUnknown => Self::RatingUnknown,
            domain::RankingEligibility::RatingExcluded => Self::RatingExcluded,
            domain::RankingEligibility::DimensionsUnknown => Self::DimensionsUnknown,
            domain::RankingEligibility::DimensionsExcluded => Self::DimensionsExcluded,
            domain::RankingEligibility::PolicyExcluded => Self::PolicyExcluded,
            domain::RankingEligibility::Duplicate => Self::Duplicate,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RankingRoute {
    #[default]
    Ineligible,
    Ranked,
    Main,
    Rescue,
    Audit,
    BudgetRejected,
}
impl From<RankingRoute> for domain::RankingRoute {
    fn from(v: RankingRoute) -> Self {
        match v {
            RankingRoute::Ineligible => Self::Ineligible,
            RankingRoute::Ranked => Self::Ranked,
            RankingRoute::Main => Self::Main,
            RankingRoute::Rescue => Self::Rescue,
            RankingRoute::Audit => Self::Audit,
            RankingRoute::BudgetRejected => Self::BudgetRejected,
        }
    }
}
impl From<domain::RankingRoute> for RankingRoute {
    fn from(v: domain::RankingRoute) -> Self {
        match v {
            domain::RankingRoute::Ineligible => Self::Ineligible,
            domain::RankingRoute::Ranked => Self::Ranked,
            domain::RankingRoute::Main => Self::Main,
            domain::RankingRoute::Rescue => Self::Rescue,
            domain::RankingRoute::Audit => Self::Audit,
            domain::RankingRoute::BudgetRejected => Self::BudgetRejected,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RankingOrder {
    #[default]
    Main,
    Rescue,
    Input,
}
impl From<RankingOrder> for domain::RankingOrder {
    fn from(v: RankingOrder) -> Self {
        match v {
            RankingOrder::Main => Self::Main,
            RankingOrder::Rescue => Self::Rescue,
            RankingOrder::Input => Self::Input,
        }
    }
}
impl From<domain::RankingOrder> for RankingOrder {
    fn from(v: domain::RankingOrder) -> Self {
        match v {
            domain::RankingOrder::Main => Self::Main,
            domain::RankingOrder::Rescue => Self::Rescue,
            domain::RankingOrder::Input => Self::Input,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RankingParameters {
    pub ratings: Vec<String>,
    pub mode: RankingMode,
    pub quotas: [u32; 3],
    pub seed: String,
    pub minimum_stored_side: Option<u32>,
    pub exclude_banned: bool,
    pub time_enabled: bool,
    pub artist_enabled: bool,
    pub votes_enabled: bool,
    pub damage_enabled: bool,
    pub cohort_minimum: u32,
    pub time_weight: f64,
    pub artist_weight: f64,
    pub vote_weight: f64,
    pub damage_weight: f64,
}
impl From<domain::RankingParameters> for RankingParameters {
    fn from(v: domain::RankingParameters) -> Self {
        Self {
            ratings: v.ratings,
            mode: v.mode.into(),
            quotas: v.quotas,
            seed: v.seed,
            minimum_stored_side: v.minimum_stored_side,
            exclude_banned: v.exclude_banned,
            time_enabled: v.time_enabled,
            artist_enabled: v.artist_enabled,
            votes_enabled: v.votes_enabled,
            damage_enabled: v.damage_enabled,
            cohort_minimum: v.cohort_minimum,
            time_weight: v.time_weight,
            artist_weight: v.artist_weight,
            vote_weight: v.vote_weight,
            damage_weight: v.damage_weight,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RankingInput {
    pub ordinal: u64,
    pub source_id: String,
    pub asset_id: String,
    pub record_id: Option<String>,
    pub observation_id: Option<String>,
    pub post_id: Option<String>,
    pub rating: Option<String>,
    pub created_at_us: Option<String>,
    pub observed_at_us: Option<String>,
    pub updated_at_us: Option<String>,
    pub time_quality: String,
    pub source_priority: Option<String>,
    pub fav_count: Option<String>,
    pub up_score: Option<String>,
    pub down_score: Option<String>,
    pub score: Option<String>,
    pub artists: Vec<String>,
    pub parent_id: Option<String>,
    pub stored_width: Option<u32>,
    pub stored_height: Option<u32>,
    pub dimension_basis: String,
    pub stored_extension: String,
    pub stored_bytes: String,
    pub is_banned: Option<bool>,
    pub is_deleted: Option<bool>,
    pub is_pending: Option<bool>,
    pub is_flagged: Option<bool>,
    pub damage_classes: u8,
    pub tags_known: bool,
    pub record_count: u32,
    pub rating_conflict: bool,
    pub basis_ids: Vec<u32>,
    pub source_issues: Option<String>,
    pub duplicate_of: Option<u64>,
}
impl From<domain::RankingInput> for RankingInput {
    fn from(v: domain::RankingInput) -> Self {
        Self {
            ordinal: v.ordinal,
            source_id: v.source_id,
            asset_id: v.asset_id,
            record_id: v.record_id,
            observation_id: v.observation_id,
            post_id: v.post_id.map(|x| x.to_string()),
            rating: v.rating,
            created_at_us: v.created_at_us.map(|x| x.to_string()),
            observed_at_us: v.observed_at_us.map(|x| x.to_string()),
            updated_at_us: v.updated_at_us.map(|x| x.to_string()),
            time_quality: v.time_quality,
            source_priority: v.source_priority.map(|x| x.to_string()),
            fav_count: v.fav_count.map(|x| x.to_string()),
            up_score: v.up_score.map(|x| x.to_string()),
            down_score: v.down_score.map(|x| x.to_string()),
            score: v.score.map(|x| x.to_string()),
            artists: v.artists,
            parent_id: v.parent_id.map(|x| x.to_string()),
            stored_width: v.stored_width,
            stored_height: v.stored_height,
            dimension_basis: v.dimension_basis,
            stored_extension: v.stored_extension,
            stored_bytes: v.stored_bytes.to_string(),
            is_banned: v.is_banned,
            is_deleted: v.is_deleted,
            is_pending: v.is_pending,
            is_flagged: v.is_flagged,
            damage_classes: v.damage_classes,
            tags_known: v.tags_known,
            record_count: v.record_count,
            rating_conflict: v.rating_conflict,
            basis_ids: v.basis_ids,
            source_issues: v.source_issues,
            duplicate_of: v.duplicate_of,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RankingScores {
    pub ordinal: u64,
    pub rating: Option<String>,
    pub eligibility: RankingEligibility,
    pub missing_flags: Vec<String>,
    pub g: Option<f64>,
    pub c: Option<f64>,
    pub a: Option<f64>,
    pub v: Option<f64>,
    pub t: Option<f64>,
    pub local_percentile: Option<f64>,
    pub local_count: u64,
    pub support_k: Option<f64>,
    pub cohort_level: Option<u32>,
    pub time_reason: String,
    pub artist_support: u64,
    pub main_score: Option<f64>,
    pub rescue_score: Option<f64>,
    pub main_rank: Option<u64>,
    pub rescue_rank: Option<u64>,
    pub selected_route: RankingRoute,
    pub duplicate_of: Option<u64>,
}
impl From<domain::RankingScores> for RankingScores {
    fn from(v: domain::RankingScores) -> Self {
        Self {
            ordinal: v.ordinal,
            rating: v.rating,
            eligibility: v.eligibility.into(),
            missing_flags: v.missing_flags,
            g: v.g,
            c: v.c,
            a: v.a,
            v: v.v,
            t: v.t,
            local_percentile: v.local_percentile,
            local_count: v.local_count,
            support_k: v.support_k,
            cohort_level: v.cohort_level,
            time_reason: v.time_reason,
            artist_support: v.artist_support,
            main_score: v.main_score,
            rescue_score: v.rescue_score,
            main_rank: v.main_rank,
            rescue_rank: v.rescue_rank,
            selected_route: v.selected_route.into(),
            duplicate_of: v.duplicate_of,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RankingRow {
    pub input: RankingInput,
    pub scores: RankingScores,
}
impl From<domain::RankingRow> for RankingRow {
    fn from(v: domain::RankingRow) -> Self {
        Self {
            input: v.input.into(),
            scores: v.scores.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RankingRatingSummary {
    pub rating: String,
    pub eligible: u64,
    pub valid_heat: u64,
    pub q0: f64,
    pub time_used: u64,
    pub time_fallback: u64,
    pub artist_used: u64,
    pub quotas: [u64; 3],
    pub selected: [u64; 3],
    pub cohort_counts: [u64; 5],
    pub favorite_baseline_overlap: u64,
}
impl From<domain::RankingRatingSummary> for RankingRatingSummary {
    fn from(v: domain::RankingRatingSummary) -> Self {
        Self {
            rating: v.rating,
            eligible: v.eligible,
            valid_heat: v.valid_heat,
            q0: v.q0,
            time_used: v.time_used,
            time_fallback: v.time_fallback,
            artist_used: v.artist_used,
            quotas: v.quotas,
            selected: v.selected,
            cohort_counts: v.cohort_counts,
            favorite_baseline_overlap: v.favorite_baseline_overlap,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RankingSummary {
    pub schema_version: u32,
    pub input_count: u64,
    pub eligible_count: u64,
    pub parameters: RankingParameters,
    pub ratings: Vec<RankingRatingSummary>,
    pub eligibility_counts: BTreeMap<String, u64>,
    pub missing_counts: BTreeMap<String, u64>,
    pub input_sha256: String,
    pub created_at: String,
}
impl From<domain::RankingSummary> for RankingSummary {
    fn from(v: domain::RankingSummary) -> Self {
        Self {
            schema_version: v.schema_version,
            input_count: v.input_count,
            eligible_count: v.eligible_count,
            parameters: v.parameters.into(),
            ratings: v.ratings.into_iter().map(Into::into).collect(),
            eligibility_counts: v.eligibility_counts,
            missing_counts: v.missing_counts,
            input_sha256: v.input_sha256,
            created_at: v.created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default)]
#[serde(default, deny_unknown_fields)]
pub struct RankingFilter {
    pub rating: Option<String>,
    pub route: Option<RankingRoute>,
    pub eligibility: Option<RankingEligibility>,
    pub missing_only: bool,
    pub selected_only: bool,
    pub top: Option<u64>,
    pub order: RankingOrder,
}
impl From<domain::RankingFilter> for RankingFilter {
    fn from(v: domain::RankingFilter) -> Self {
        Self {
            rating: v.rating,
            route: v.route.map(Into::into),
            eligibility: v.eligibility.map(Into::into),
            missing_only: v.missing_only,
            selected_only: v.selected_only,
            top: v.top,
            order: v.order.into(),
        }
    }
}
impl From<RankingFilter> for domain::RankingFilter {
    fn from(v: RankingFilter) -> Self {
        Self {
            rating: v.rating,
            route: v.route.map(Into::into),
            eligibility: v.eligibility.map(Into::into),
            missing_only: v.missing_only,
            selected_only: v.selected_only,
            top: v.top,
            order: v.order.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RankingBasis {
    pub index: u32,
    pub result_id: Option<String>,
    pub spec: crate::QuerySpec,
}
impl From<domain::RankingBasis> for RankingBasis {
    fn from(v: domain::RankingBasis) -> Self {
        Self {
            index: v.index,
            result_id: v.result_id,
            spec: v.spec.into(),
        }
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RankingPageRequest {
    #[serde(default)]
    pub filter: RankingFilter,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}
#[derive(Serialize, ToSchema)]
pub struct RankingPage {
    pub artifact_id: String,
    pub items: Vec<RankingRow>,
    pub next_cursor: Option<String>,
    pub count: Option<u64>,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RankingWorksetRequest {
    pub idempotency_key: String,
    pub name: String,
    #[serde(default)]
    pub filter: RankingFilter,
}
#[derive(Serialize, ToSchema)]
pub struct RankingEvidence {
    pub job_run: crate::JobRun,
    pub bases: Vec<RankingBasis>,
    pub metadata_fields: Vec<String>,
}
