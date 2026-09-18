//! Offline use cases. No provider, image, HTTP or SQLite dependency.
use studio_domain::{Error, Result, aesthetic::AestheticCandidate, aesthetic_analysis::*};
pub mod comparison;
pub mod estimator;

pub trait AestheticReplaySource {
    fn replay_candidates(&self, stage: &str, after: Option<u64>)
    -> Result<Vec<AestheticCandidate>>;
    fn replay_observations(
        &self,
        input: &AestheticAnalysisInput,
        after: u64,
    ) -> Result<Vec<AestheticReplayObservation>>;
}

pub fn validate_fit(fit: &AestheticFit) -> Result<()> {
    studio_domain::validate_id(&fit.stage_id)?;
    let e = &fit.estimator;
    if !matches!(e.kind.as_str(), "davidson_v1" | "borda_v1")
        || !(1..=128).contains(&e.iterations)
        || !e.regularization.is_finite()
        || !(0.001..=10.0).contains(&e.regularization)
        || !e.tie_strength.is_finite()
        || !(0.01..=100.0).contains(&e.tie_strength)
    {
        return Err(Error::invalid(
            "估计器需为 davidson_v1 或 borda_v1；迭代 1–128，正则化 0.001–10，并列强度 0.01–100",
        ));
    }
    Ok(())
}
pub fn validate_filter(f: &AestheticRankingFilter) -> Result<()> {
    if f.ratings.len() > 4
        || f.ratings
            .iter()
            .any(|r| !matches!(r.as_str(), "g" | "s" | "q" | "e"))
        || f.top_percent
            .is_some_and(|v| !v.is_finite() || v <= 0.0 || v > 100.0)
        || f.min_split_delta
            .is_some_and(|v| !v.is_finite() || !(0.0..=1.0).contains(&v))
        || f.rank_from.is_some_and(|v| v == 0 || v > 1_000_000)
        || f.rank_to.is_some_and(|v| v == 0 || v > 1_000_000)
        || f.component.is_some_and(|v| v > 1_000_000)
        || f.rank_from.zip(f.rank_to).is_some_and(|(a, b)| a > b)
        || f.year_from.zip(f.year_to).is_some_and(|(a, b)| a > b)
    {
        return Err(Error::invalid("排名筛选条件无效"));
    }
    Ok(())
}
pub fn validate_create(value: &AestheticAnalysisCreate) -> Result<()> {
    studio_domain::validate_id(&value.idempotency_key)?;
    studio_domain::validate_name(&value.name)?;
    match &value.spec {
        AestheticAnalysisSpec::Fit {
            config,
            experiment_id,
            variant,
        } => {
            validate_fit(config)?;
            if let Some(id) = experiment_id {
                studio_domain::validate_id(id)?;
            }
            if let Some(label) = variant {
                studio_domain::validate_name(label)?;
            }
            if experiment_id.is_some() != variant.is_some() {
                return Err(Error::invalid("实验与变体需要同时提供"));
            }
        }
        AestheticAnalysisSpec::Compare { left, right } => {
            studio_domain::validate_id(left)?;
            studio_domain::validate_id(right)?;
        }
        AestheticAnalysisSpec::Derive {
            snapshot_id,
            filter,
            ..
        } => {
            studio_domain::validate_id(snapshot_id)?;
            validate_filter(filter)?;
        }
    }
    Ok(())
}
pub fn validate_experiment(value: &AestheticExperimentCreate) -> Result<()> {
    studio_domain::validate_id(&value.idempotency_key)?;
    studio_domain::validate_name(&value.name)?;
    if value.description.len() > 8192 || value.variants.is_empty() || value.variants.len() > 12 {
        return Err(Error::invalid("实验需要 1–12 个变体；说明上限 8192 字节"));
    }
    let mut names = std::collections::BTreeSet::new();
    for v in &value.variants {
        studio_domain::validate_name(&v.label)?;
        validate_fit(&v.fit)?;
        if !names.insert(&v.label) {
            return Err(Error::invalid("实验变体名称重复"));
        }
    }
    Ok(())
}
pub fn validate_review(value: &AestheticReviewCreate) -> Result<()> {
    studio_domain::validate_id(&value.idempotency_key)?;
    studio_domain::validate_id(&value.snapshot_id)?;
    studio_domain::validate_name(&value.reviewer)?;
    if value.ordinal > 1_000_000
        || value.reason.trim().is_empty()
        || value.reason.len() > 8192
        || !matches!(
            value.decision.as_str(),
            "protect" | "confirm_elite" | "release" | "defer"
        )
    {
        return Err(Error::invalid("复核需要有效图片、决定和理由"));
    }
    Ok(())
}

/// Rank cutoffs include all members of a computed score tie at the boundary.
/// Percentage denominator is the frozen Rating population, or an explicitly selected component.
pub fn matches_filter(
    row: &AestheticRankingRow,
    f: &AestheticRankingFilter,
    protected: bool,
) -> bool {
    if !f.ratings.is_empty() && !f.ratings.contains(&row.rating)
        || f.year_from.is_some_and(|v| row.year.is_none_or(|y| y < v))
        || f.year_to.is_some_and(|v| row.year.is_none_or(|y| y > v))
    {
        return false;
    }
    if f.include_protected && protected {
        return true;
    }
    if f.component.is_some_and(|c| row.component != Some(c)) {
        return false;
    }
    let (lo, hi, p) = if f.component.is_some() {
        (row.rank_min, row.rank_max, row.component_percentile)
    } else {
        (row.rating_rank_min, row.rating_rank_max, row.percentile)
    };
    // Use the start of a tie, not its midpoint, for Top percentage selection.
    let n = row.component_size;
    if f.top_percent
        .is_some_and(|v| lo.is_none_or(|r| r > ((n as f64 * v / 100.0).ceil() as u64).max(1)))
        || f.rank_from.is_some_and(|v| hi.is_none_or(|r| r < v))
        || f.rank_to.is_some_and(|v| lo.is_none_or(|r| r > v))
        || f.max_exposures.is_some_and(|v| row.exposures > v)
        || f.min_split_delta
            .is_some_and(|v| row.split_percentile_delta.is_none_or(|d| d < v))
        || (f.protected_only && !protected)
        || (f.needs_review && !row.needs_review)
    {
        return false;
    }
    let _ = p;
    true
}
