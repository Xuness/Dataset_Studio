//! Versioned metadata ranking values; no transport or storage dependency.
use crate::{AssetKey, Error, QuerySpec, Result};
use serde::{Deserialize, Serialize};

pub const RANKING_OPERATOR: &str = "danbooru.metarecall";
pub const RANKING_KIND: &str = "ranking_table";

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_defaults_and_old_serialized_parameters_keep_distinct_rules() {
        let current = RankingParameters::default();
        assert_eq!(current.duplicate_heat, Some(DuplicateHeat::Highest));
        let mut old = serde_json::to_value(current).unwrap();
        old.as_object_mut().unwrap().remove("duplicate_heat");
        let restored: RankingParameters = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(restored.duplicate_heat, None);
        assert_eq!(serde_json::to_value(restored).unwrap(), old);
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RankingMode {
    #[default]
    Rank,
    Select,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateHeat {
    Highest,
    Sum,
}

/// Complete per-post evidence. Metadata identity and heat evidence may differ.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingObservation {
    pub record_id: String,
    pub observation_id: String,
    pub post_id: Option<i64>,
    pub rating: Option<String>,
    pub fav_count: Option<i64>,
    pub up_score: Option<i64>,
    pub down_score: Option<i64>,
    pub score: Option<i64>,
    pub created_at_us: Option<i64>,
    pub observed_at_us: Option<i64>,
    pub time_quality: String,
    pub updated_at_us: Option<i64>,
    pub is_deleted: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingDuplicateEvidence {
    pub policy: DuplicateHeat,
    pub metadata: RankingObservation,
    pub heat: Vec<RankingObservation>,
    pub post_count: u64,
    pub omitted_posts: u64,
    pub partial_counts: bool,
    pub counts_clamped: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RankingParameters {
    /// None preserves the original complete-representative rule in old jobs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplicate_heat: Option<DuplicateHeat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub v2: Option<crate::RankingV2Parameters>,
    pub ratings: Vec<String>,
    pub mode: RankingMode,
    /// Integer thousandths, e.g. 280 = 28.0 percent.
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
impl Default for RankingParameters {
    fn default() -> Self {
        Self {
            duplicate_heat: Some(DuplicateHeat::Highest),
            v2: None,
            ratings: ["g", "s", "q", "e"].map(String::from).into(),
            mode: RankingMode::Rank,
            quotas: [280, 43, 10],
            seed: "metarecall-v1".into(),
            minimum_stored_side: None,
            exclude_banned: false,
            time_enabled: true,
            artist_enabled: false,
            votes_enabled: true,
            damage_enabled: true,
            cohort_minimum: 5_000,
            time_weight: 0.30,
            artist_weight: 0.25,
            vote_weight: 0.08,
            damage_weight: 0.04,
        }
    }
}
impl RankingParameters {
    pub fn normalize(mut self) -> Result<Self> {
        self.v2 = self
            .v2
            .map(crate::RankingV2Parameters::normalize)
            .transpose()?;
        if self.ratings.is_empty()
            || self.ratings.len() > 4
            || self
                .ratings
                .iter()
                .any(|r| !matches!(r.as_str(), "g" | "s" | "q" | "e"))
        {
            return Err(Error::invalid("请选择 G、S、Q、E 中的有效分级"));
        }
        self.ratings.sort();
        self.ratings.dedup();
        if self.quotas.iter().any(|v| *v > 1_000)
            || self.quotas.iter().map(|v| u64::from(*v)).sum::<u64>() > 1_000
        {
            return Err(Error::invalid("三条通道的总比例不得超过 100%"));
        }
        if self.seed.is_empty() || self.seed.len() > 128 {
            return Err(Error::invalid("抽样种子需要 1–128 字节"));
        }
        if self
            .minimum_stored_side
            .is_some_and(|v| v == 0 || v > 65_535)
            || self.cohort_minimum == 0
            || self.cohort_minimum > 10_000_000
        {
            return Err(Error::invalid("用途尺寸或比较群体最低数量超出范围"));
        }
        for value in [
            self.time_weight,
            self.artist_weight,
            self.vote_weight,
            self.damage_weight,
        ] {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(Error::invalid("评分系数必须是 0–1 的有限数值"));
            }
        }
        Ok(self)
    }
}

/// Frozen metadata and numeric inputs; merged inputs retain separate evidence.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RankingInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<RankingDuplicateEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<String>,
    pub ordinal: u64,
    pub source_id: String,
    pub asset_id: String,
    pub record_id: Option<String>,
    pub observation_id: Option<String>,
    pub post_id: Option<i64>,
    pub rating: Option<String>,
    pub created_at_us: Option<i64>,
    pub observed_at_us: Option<i64>,
    pub updated_at_us: Option<i64>,
    pub time_quality: String,
    pub source_priority: Option<i64>,
    pub fav_count: Option<i64>,
    pub up_score: Option<i64>,
    pub down_score: Option<i64>,
    pub score: Option<i64>,
    pub artists: Vec<String>,
    pub parent_id: Option<i64>,
    pub stored_width: Option<u32>,
    pub stored_height: Option<u32>,
    pub dimension_basis: String,
    pub stored_extension: String,
    pub stored_bytes: u64,
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
impl RankingInput {
    pub fn key(&self) -> AssetKey {
        AssetKey {
            source_id: self.source_id.clone(),
            asset_id: self.asset_id.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingBasis {
    pub index: u32,
    pub result_id: Option<String>,
    pub spec: QuerySpec,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RankingScores {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub v2: Option<crate::RankingV2Scores>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingRow {
    pub input: RankingInput,
    pub scores: RankingScores,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RankingRatingSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub v2: Option<crate::RankingV2Summary>,
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RankingSummary {
    pub schema_version: u32,
    pub input_count: u64,
    pub eligible_count: u64,
    pub parameters: RankingParameters,
    pub ratings: Vec<RankingRatingSummary>,
    pub eligibility_counts: std::collections::BTreeMap<String, u64>,
    pub missing_counts: std::collections::BTreeMap<String, u64>,
    pub input_sha256: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RankingOrder {
    #[default]
    Main,
    Rescue,
    Input,
    Direct,
    Fused,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
impl RankingFilter {
    pub fn validate(&self) -> Result<()> {
        if self
            .rating
            .as_deref()
            .is_some_and(|r| !matches!(r, "g" | "s" | "q" | "e"))
            || self.top.is_some_and(|v| v == 0 || v > i64::MAX as u64)
        {
            return Err(Error::invalid("无效排名过滤条件"));
        }
        Ok(())
    }
}
