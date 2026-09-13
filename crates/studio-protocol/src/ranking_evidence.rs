use serde::{Deserialize, Serialize};
use studio_domain as domain;
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateHeat {
    Highest,
    Sum,
}
impl From<domain::DuplicateHeat> for DuplicateHeat {
    fn from(value: domain::DuplicateHeat) -> Self {
        match value {
            domain::DuplicateHeat::Highest => Self::Highest,
            domain::DuplicateHeat::Sum => Self::Sum,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RankingObservation {
    pub record_id: String,
    pub observation_id: String,
    pub post_id: Option<String>,
    pub rating: Option<String>,
    pub fav_count: Option<String>,
    pub up_score: Option<String>,
    pub down_score: Option<String>,
    pub score: Option<String>,
    pub created_at_us: Option<String>,
    pub observed_at_us: Option<String>,
    pub time_quality: String,
    pub updated_at_us: Option<String>,
    pub is_deleted: Option<bool>,
}
impl From<domain::RankingObservation> for RankingObservation {
    fn from(v: domain::RankingObservation) -> Self {
        Self {
            record_id: v.record_id,
            observation_id: v.observation_id,
            post_id: v.post_id.map(|n| n.to_string()),
            rating: v.rating,
            fav_count: v.fav_count.map(|n| n.to_string()),
            up_score: v.up_score.map(|n| n.to_string()),
            down_score: v.down_score.map(|n| n.to_string()),
            score: v.score.map(|n| n.to_string()),
            created_at_us: v.created_at_us.map(|n| n.to_string()),
            observed_at_us: v.observed_at_us.map(|n| n.to_string()),
            time_quality: v.time_quality,
            updated_at_us: v.updated_at_us.map(|n| n.to_string()),
            is_deleted: v.is_deleted,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RankingDuplicateEvidence {
    pub policy: DuplicateHeat,
    pub metadata: RankingObservation,
    pub heat: Vec<RankingObservation>,
    pub post_count: u64,
    pub omitted_posts: u64,
    pub partial_counts: bool,
    pub counts_clamped: bool,
}
impl From<domain::RankingDuplicateEvidence> for RankingDuplicateEvidence {
    fn from(v: domain::RankingDuplicateEvidence) -> Self {
        Self {
            policy: v.policy.into(),
            metadata: v.metadata.into(),
            heat: v.heat.into_iter().map(Into::into).collect(),
            post_count: v.post_count,
            omitted_posts: v.omitted_posts,
            partial_counts: v.partial_counts,
            counts_clamped: v.counts_clamped,
        }
    }
}
