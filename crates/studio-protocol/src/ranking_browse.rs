use crate::{RankingEligibility, RankingFilter, RankingOrder, ScopeRef, domain};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct RankedScope {
    pub current_rating_filter: bool,
    pub view_key: String,
    pub schema_version: u32,
    pub workset_id: String,
    pub artifact_id: String,
    pub artifact_name: String,
    pub count: u64,
    pub saved_filter: RankingFilter,
}
impl From<domain::RankedScope> for RankedScope {
    fn from(value: domain::RankedScope) -> Self {
        Self {
            current_rating_filter: value.current_rating_filter,
            view_key: value.view_key,
            schema_version: value.schema_version,
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
pub struct RankingBrowseLease {
    pub scope: ScopeRef,
    pub lease_id: String,
    #[serde(default)]
    pub release: bool,
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
    /// Positive, one-based position in the current scope and viewing direction.
    /// With start_rating, this is the original rank within that frozen Rating instead.
    /// Mutually exclusive with start_post_id; the located member is included.
    pub start_rank: Option<String>,
    /// Frozen scoring Rating (g, s, q, e). Requires start_rank and a ranking order other than input.
    /// Only locates a member; it does not filter or change the scope.
    pub start_rating: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Serialize, ToSchema)]
pub struct AssetRanking {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub v2: Option<crate::RankingV2Scores>,
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
