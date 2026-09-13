//! Metadata-only v2 profiles. Visual bridge evidence is deliberately not implied.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const RANKING_V2_OPERATOR: &str = "danbooru.metarecall_v2";
pub fn is_ranking_operator(id: &str) -> bool {
    matches!(id, crate::RANKING_OPERATOR | RANKING_V2_OPERATOR)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RatingProfile {
    pub time_up: f64,
    pub time_down: f64,
    pub vote_weight: f64,
    pub era_weight: f64,
}
impl Default for RatingProfile {
    fn default() -> Self {
        Self {
            time_up: 0.30,
            time_down: 0.0,
            vote_weight: 0.08,
            era_weight: 0.30,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EraPreference {
    pub from_year: i32,
    pub through_year: i32,
    pub bonus: f64,
    /// Thousandths of the final per-rating retained count. Null means soft preference only.
    pub target_share: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
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
impl Default for RankingV2Parameters {
    fn default() -> Self {
        Self {
            profiles: ["g", "s", "q", "e"]
                .map(|r| (r.into(), RatingProfile::default()))
                .into(),
            feather_days: 90,
            minimum_effective: 5_000,
            comic_penalty: 0.0,
            keep_per_mille: 333,
            direct_rescue: 50,
            era_rescue: 50,
            audit: 30,
            eras: vec![],
            strict_era_targets: false,
        }
    }
}
impl RankingV2Parameters {
    pub fn normalize(mut self) -> Result<Self> {
        if self
            .profiles
            .keys()
            .any(|r| !matches!(r.as_str(), "g" | "s" | "q" | "e"))
        {
            return Err(Error::invalid("v2 分级配置只能包含 G、S、Q、E"));
        }
        for r in ["g", "s", "q", "e"] {
            self.profiles.entry(r.into()).or_default();
        }
        for p in self.profiles.values() {
            if [p.time_up, p.time_down, p.vote_weight, p.era_weight]
                .iter()
                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            {
                return Err(Error::invalid("v2 评分权重必须在 0–1 之间"));
            }
        }
        if self.feather_days > 180
            || !(1..=10_000_000).contains(&self.minimum_effective)
            || !self.comic_penalty.is_finite()
            || !(0.0..=10.0).contains(&self.comic_penalty)
        {
            return Err(Error::invalid(
                "羽化半宽应为 0–180 天；有效样本下限或类型扣分超出范围",
            ));
        }
        if self.keep_per_mille > 1000
            || self.direct_rescue > 1000
            || self.era_rescue > 1000
            || self.audit > 1000
            || self.direct_rescue + self.era_rescue + self.audit > 1000
        {
            return Err(Error::invalid(
                "v2 保留比例及预算内补救、审计比例必须在 0–100% 内",
            ));
        }
        if self.eras.len() > 64 {
            return Err(Error::invalid("最多配置 64 个年代区间"));
        }
        self.eras.sort_by_key(|e| (e.from_year, e.through_year));
        let mut end = None;
        let mut total = 0u32;
        for e in &self.eras {
            if !(1900..=2200).contains(&e.from_year)
                || !(e.from_year..=2200).contains(&e.through_year)
                || end.is_some_and(|n| e.from_year <= n)
                || !e.bonus.is_finite()
                || !(-20.0..=20.0).contains(&e.bonus)
                || e.target_share.is_some_and(|v| v > 1000)
            {
                return Err(Error::invalid(
                    "年代区间须为 1900–2200 年且互不重叠；偏好为 -20 至 20 分",
                ));
            }
            total += e.target_share.unwrap_or(0);
            end = Some(e.through_year);
        }
        if total > 1000 {
            return Err(Error::invalid("年代目标比例合计不能超过保留预算的 100%"));
        }
        Ok(self)
    }
}

/// Compact per-image values; strings and complete tags remain on disk.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RankingYearSummary {
    pub year: Option<i32>,
    pub count: u64,
    pub selected: u64,
    pub top_count: u64,
    pub fallback: u64,
    pub protected: u64,
    pub penalized: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
