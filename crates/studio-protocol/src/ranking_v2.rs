use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use studio_domain as domain;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RatingProfile {
    pub time_up: f64,
    pub time_down: f64,
    pub vote_weight: f64,
    pub era_weight: f64,
}
impl From<domain::RatingProfile> for RatingProfile {
    fn from(v: domain::RatingProfile) -> Self {
        Self {
            time_up: v.time_up,
            time_down: v.time_down,
            vote_weight: v.vote_weight,
            era_weight: v.era_weight,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EraPreference {
    pub from_year: i32,
    pub through_year: i32,
    pub bonus: f64,
    /// Thousandths of the final per-rating retained count. Null means soft preference only.
    pub target_share: Option<u32>,
}
impl From<domain::EraPreference> for EraPreference {
    fn from(v: domain::EraPreference) -> Self {
        Self {
            from_year: v.from_year,
            through_year: v.through_year,
            bonus: v.bonus,
            target_share: v.target_share,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RankingV2Parameters {
    pub profiles: BTreeMap<String, RatingProfile>,
    pub feather_days: u32,
    pub minimum_effective: u32,
    pub comic_penalty: f64,
    pub keep_per_mille: u32,
    pub direct_rescue: u32,
    pub era_rescue: u32,
    pub audit: u32,
    pub eras: Vec<EraPreference>,
    pub strict_era_targets: bool,
}
impl From<domain::RankingV2Parameters> for RankingV2Parameters {
    fn from(v: domain::RankingV2Parameters) -> Self {
        Self {
            profiles: v.profiles.into_iter().map(|(k, p)| (k, p.into())).collect(),
            feather_days: v.feather_days,
            minimum_effective: v.minimum_effective,
            comic_penalty: v.comic_penalty,
            keep_per_mille: v.keep_per_mille,
            direct_rescue: v.direct_rescue,
            era_rescue: v.era_rescue,
            audit: v.audit,
            eras: v.eras.into_iter().map(Into::into).collect(),
            strict_era_targets: v.strict_era_targets,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RankingV2Scores {
    pub created_year: Option<i32>,
    pub old_period: Option<i32>,
    pub new_period: Option<i32>,
    pub new_weight: f64,
    pub old_percentile: f64,
    pub new_percentile: f64,
    pub year_percentile: f64,
    pub effective_count: f64,
    pub era_fallback: bool,
    pub type_hints: u32,
    pub layout_protected: bool,
    pub direct_raw: f64,
    pub era_raw: f64,
    pub direct_percentile: f64,
    pub era_percentile: f64,
    pub fused_score: f64,
    pub type_penalty: f64,
    pub era_bonus: f64,
    pub direct_rank: u64,
    pub fused_rank: u64,
    pub year_rank: u64,
    /// 0 none/main, 1 direct-only rescue, 2 era-only rescue, 3 random audit.
    pub selection_reason: u8,
}
impl From<domain::RankingV2Scores> for RankingV2Scores {
    fn from(v: domain::RankingV2Scores) -> Self {
        Self {
            created_year: v.created_year,
            old_period: v.old_period,
            new_period: v.new_period,
            new_weight: v.new_weight,
            old_percentile: v.old_percentile,
            new_percentile: v.new_percentile,
            year_percentile: v.year_percentile,
            effective_count: v.effective_count,
            era_fallback: v.era_fallback,
            type_hints: v.type_hints,
            layout_protected: v.layout_protected,
            direct_raw: v.direct_raw,
            era_raw: v.era_raw,
            direct_percentile: v.direct_percentile,
            era_percentile: v.era_percentile,
            fused_score: v.fused_score,
            type_penalty: v.type_penalty,
            era_bonus: v.era_bonus,
            direct_rank: v.direct_rank,
            fused_rank: v.fused_rank,
            year_rank: v.year_rank,
            selection_reason: v.selection_reason,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RankingYearSummary {
    pub year: Option<i32>,
    pub count: u64,
    pub selected: u64,
    pub top_count: u64,
    pub fallback: u64,
    pub protected: u64,
    pub penalized: u64,
}
impl From<domain::RankingYearSummary> for RankingYearSummary {
    fn from(v: domain::RankingYearSummary) -> Self {
        Self {
            year: v.year,
            count: v.count,
            selected: v.selected,
            top_count: v.top_count,
            fallback: v.fallback,
            protected: v.protected,
            penalized: v.penalized,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RankingV2Summary {
    /// This release has no visual bridge calibration or inferred aesthetic probabilities.
    pub metadata_only: bool,
    pub years: Vec<RankingYearSummary>,
    pub direct_rescued: u64,
    pub era_rescued: u64,
    pub audit_selected: u64,
    pub protection_shortfall: [u64; 2],
    pub era_target_shortfall: u64,
    pub fallback_count: u64,
    pub protected_count: u64,
    pub type_penalized: u64,
}
impl From<domain::RankingV2Summary> for RankingV2Summary {
    fn from(v: domain::RankingV2Summary) -> Self {
        Self {
            metadata_only: v.metadata_only,
            years: v.years.into_iter().map(Into::into).collect(),
            direct_rescued: v.direct_rescued,
            era_rescued: v.era_rescued,
            audit_selected: v.audit_selected,
            protection_shortfall: v.protection_shortfall,
            era_target_shortfall: v.era_target_shortfall,
            fallback_count: v.fallback_count,
            protected_count: v.protected_count,
            type_penalized: v.type_penalized,
        }
    }
}
