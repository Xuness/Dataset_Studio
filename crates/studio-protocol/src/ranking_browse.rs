use crate::{RankingEligibility, RankingFilter, RankingOrder, ScopeRef, domain};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct RankedScope {
    pub workset_id: String,
    pub artifact_id: String,
    pub artifact_name: String,
    pub count: u64,
    pub saved_filter: RankingFilter,
}
impl From<domain::RankedScope> for RankedScope {
    fn from(value: domain::RankedScope) -> Self {
        Self {
            workset_id: value.workset_id,
            artifact_id: value.artifact_id,
            artifact_name: value.artifact_name,
            count: value.count,
            saved_filter: value.saved_filter.into(),
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct RankingBrowseInfo {
    pub ranking: Option<RankedScope>,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RankingBrowseRequest {
    pub scope: ScopeRef,
    /// Omitted means the order saved with the workset. This never changes membership.
    pub order: Option<RankingOrder>,
    #[serde(default)]
    pub descending: bool,
    /// Exact frozen Danbooru post ID; the located row is included as the first item.
    pub start_post_id: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Serialize, ToSchema)]
pub struct AssetRanking {
    pub artifact_id: String,
    pub ordinal: u64,
    pub post_id: Option<String>,
    pub rating: Option<String>,
    pub eligibility: RankingEligibility,
    pub main_score: Option<f64>,
    pub rescue_score: Option<f64>,
    pub main_rank: Option<u64>,
    pub rescue_rank: Option<u64>,
}
