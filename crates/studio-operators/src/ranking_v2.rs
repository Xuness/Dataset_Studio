//! Feathered metadata rankings. No visual quality or bridge calibration is inferred.
use crate::ranking::{self, ArtistWork, DAY, Progress, Sample};
use chrono::{Datelike, NaiveDate};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use studio_application::Operator;
use studio_domain::*;

pub struct MetaRecallV2;
impl Operator for MetaRecallV2 {
    fn population(&self) -> bool {
        true
    }
    fn descriptor(&self) -> OperatorDescriptor {
        let mut d = ranking::MetaRecall.descriptor();
        d.id = RANKING_V2_OPERATOR.into();
        d.name = "Danbooru 元数据排名 · v2".into();
        d.outputs[0].schema_version = 2;
        d.parameters[0].name = "MetaRecall v2 元数据参数".into();
        let p = RankingParameters {
            v2: Some(RankingV2Parameters::default()),
            ..Default::default()
        };
        d.parameters[0].default_value = json!(p);
        d
    }
    fn normalize(&self, value: Value) -> Result<Value> {
        let mut p: RankingParameters = serde_json::from_value(value).map_err(Error::io)?;
        p.v2.get_or_insert_with(RankingV2Parameters::default);
        serde_json::to_value(p.normalize()?).map_err(Error::io)
    }
    fn required_fields(&self, _: &Value) -> Result<Vec<ScalarInput>> {
        Ok(vec![])
    }
    fn row(&self, _: &FrozenInput, _: u64, _: &Value) -> Result<Value> {
        Err(Error::new("POPULATION_REQUIRED", "v2 需要固定的总体输入表"))
    }
}

/// Generic comic hints do not trigger a penalty. Protected 2–4-panel and inset
/// layouts override the conservative extended-comic hint, never technical damage.
pub fn type_hints(tags: Option<&str>) -> u32 {
    let mut bits = 0;
    for tag in tags.unwrap_or("").split_whitespace() {
        bits |= match tag {
            "comic" | "1koma" | "sequential" | "borderless_panels" => 1,
            "full_page_comic"
            | "vertical_scroll_comic"
            | "segmented_comic"
            | "5koma"
            | "6koma"
            | "multiple_2koma"
            | "multiple_3koma"
            | "multiple_4koma" => 2,
            "multiple_views" | "cut-in" | "zoom_layer" | "projected_inset" => 4,
            "2koma" | "3koma" | "4koma" => 8,
            "reference_sheet" | "character_sheet" | "turnaround" | "multiple_expressions" => 16,
            "text_focus" | "game_screenshot" | "sprite_sheet" => 32,
            _ => 0,
        }
    }
    bits
}

fn start(year: i32) -> Option<i64> {
    Some(
        NaiveDate::from_ymd_opt(year, 1, 1)?
            .and_hms_opt(0, 0, 0)?
            .and_utc()
            .timestamp_micros(),
    )
}
/// (calendar year, left period, right period, right weight).
pub fn feather(created: i64, days: u32) -> Option<(i32, i32, i32, f64)> {
    let year = chrono::DateTime::from_timestamp_micros(created)?.year();
    if days == 0 {
        return Some((year, year, year, 0.0));
    }
    let h = i64::from(days) * DAY;
    let begin = start(year)?;
    let end = start(year.checked_add(1)?)?;
    let (left, right, boundary) = if created < begin.checked_add(h)? {
        (year.checked_sub(1)?, year, begin)
    } else if created > end.checked_sub(h)? {
        (year, year.checked_add(1)?, end)
    } else {
        return Some((year, year, year, 0.0));
    };
    let u = ((created as f64 - boundary as f64 + h as f64) / (2.0 * h as f64)).clamp(0.0, 1.0);
    Some((year, left, right, u * u * (3.0 - 2.0 * u)))
}
fn age_group(s: &Sample) -> Option<u8> {
    let (lo, hi) = s.age?;
    [0, 30, 365, 2555, i64::MAX]
        .windows(2)
        .position(|w| lo >= w[0].saturating_mul(DAY) && hi < w[1].saturating_mul(DAY))
        .map(|n| n as u8)
}
#[derive(Clone, Copy)]
struct Member {
    item: usize,
    weight: f64,
    right: bool,
}

fn percentile(values: impl Fn(usize) -> f64, out: &mut [f64], order: &mut [usize]) {
    order.sort_unstable_by(|&a, &b| values(a).total_cmp(&values(b)).then(a.cmp(&b)));
    let mut start = 0;
    while start < order.len() {
        let mut end = start + 1;
        while end < order.len() && values(order[start]) == values(order[end]) {
            end += 1;
        }
        let p = (start as f64 + 0.5 * (end - start) as f64) / order.len() as f64;
        for &i in &order[start..end] {
            out[i] = p;
        }
        start = end;
    }
}
fn descending(samples: &[Sample], value: impl Fn(usize) -> f64, order: &mut [usize]) {
    order.sort_unstable_by(|&a, &b| {
        value(b)
            .total_cmp(&value(a))
            .then(samples[a].hash.cmp(&samples[b].hash))
    });
}
fn conditioned(p: f64, n: f64, support: u64, k: f64) -> f64 {
    let q = 0.5 + n / (n + 1000.0) * (p - 0.5);
    if q <= 0.5 {
        q
    } else {
        0.5 + support as f64 / (support as f64 + k) * (q - 0.5)
    }
}
pub fn compute(
    rating: &str,
    samples: &mut [Sample],
    hints: &[u32],
    p: &RankingParameters,
    artists: &mut [ArtistWork],
    progress: &mut Progress<'_>,
) -> Result<(RankingRatingSummary, Vec<RankingV2Scores>)> {
    let cfg =
        p.v2.as_ref()
            .ok_or_else(|| Error::invalid("缺少 v2 配置"))?;
    let profile = &cfg.profiles[rating];
    // v1 remains an unchanged feature/baseline computation; no v1 quotas are applied.
    let base = RankingParameters {
        mode: RankingMode::Rank,
        v2: None,
        ..p.clone()
    };
    let mut summary = ranking::compute_rating(rating, samples, &base, artists, progress)?;
    let n = samples.len();
    if hints.len() != n {
        return Err(Error::invalid("v2 类型特征与输入不一致"));
    }
    let mut values = Vec::with_capacity(n);
    let mut groups = BTreeMap::<(i32, u8), Vec<Member>>::new();
    for (i, s) in samples.iter().enumerate() {
        if i.is_multiple_of(8192) {
            progress("v2_periods", i as u64, n as u64)?;
        }
        let f = s.created.and_then(|t| feather(t, cfg.feather_days));
        let mut v = RankingV2Scores {
            type_hints: hints[i],
            layout_protected: hints[i] & (4 | 8 | 16) != 0,
            old_percentile: s.c,
            new_percentile: s.c,
            year_percentile: s.c,
            era_fallback: true,
            direct_raw: s.g + profile.time_up * (s.c - s.g).max(0.0)
                - profile.time_down * (s.g - s.c).max(0.0)
                - profile.vote_weight * s.v
                - s.t,
            ..Default::default()
        };
        if let Some((year, left, right, w)) = f {
            v.created_year = Some(year);
            v.old_period = Some(left);
            v.new_period = Some(right);
            v.new_weight = w;
            if p.time_enabled
                && s.heat.is_some()
                && let Some(age) = age_group(s)
            {
                if 1.0 - w > 0.0 {
                    groups.entry((left, age)).or_default().push(Member {
                        item: i,
                        weight: 1.0 - w,
                        right: false,
                    });
                }
                if w > 0.0 {
                    groups.entry((right, age)).or_default().push(Member {
                        item: i,
                        weight: w,
                        right: true,
                    });
                }
            }
        }
        values.push(v);
    }
    let mut left_ok = vec![false; n];
    let mut right_ok = vec![false; n];
    let mut left_n = vec![0.0f64; n];
    let mut right_n = vec![0.0f64; n];
    let group_count = groups.len() as u64;
    for (g_index, (_, mut members)) in groups.into_iter().enumerate() {
        progress("v2_feather", g_index as u64, group_count)?;
        let sum = members.iter().map(|m| m.weight).sum::<f64>();
        let sum2 = members.iter().map(|m| m.weight * m.weight).sum::<f64>();
        let effective = sum * sum / sum2;
        if effective < f64::from(cfg.minimum_effective) {
            continue;
        }
        let mut positive: Vec<(u64, f64)> = members
            .iter()
            .filter(|m| samples[m.item].support > 0)
            .map(|m| (samples[m.item].support, m.weight))
            .collect();
        positive.sort_unstable_by_key(|v| v.0);
        let midpoint = positive.iter().map(|v| v.1).sum::<f64>() * 0.5;
        let mut acc = 0.0;
        let mut k = 1.0;
        for (m, w) in positive {
            acc += w;
            if acc >= midpoint {
                k = (0.25 * m as f64).clamp(1.0, 10.0);
                break;
            }
        }
        members.sort_unstable_by_key(|m| (samples[m.item].heat, m.item));
        let mut before = 0.0;
        let mut at = 0;
        while at < members.len() {
            if at.is_multiple_of(8192) {
                progress("v2_weighted_ranks", at as u64, members.len() as u64)?;
            }
            let mut end = at + 1;
            while end < members.len()
                && samples[members[end].item].heat == samples[members[at].item].heat
            {
                end += 1;
            }
            let mass = members[at..end].iter().map(|m| m.weight).sum::<f64>();
            let raw = (before + 0.5 * mass) / sum;
            for m in &members[at..end] {
                let score = conditioned(raw, effective, samples[m.item].support, k);
                if m.right {
                    values[m.item].new_percentile = score;
                    right_ok[m.item] = true;
                    right_n[m.item] = effective;
                } else {
                    values[m.item].old_percentile = score;
                    left_ok[m.item] = true;
                    left_n[m.item] = effective;
                }
            }
            before += mass;
            at = end;
        }
    }
    for (i, (s, v)) in samples.iter().zip(values.iter_mut()).enumerate() {
        if i.is_multiple_of(8192) {
            progress("v2_scores", i as u64, n as u64)?;
        }
        let w = v.new_weight;
        v.era_fallback = (!left_ok[i] && w < 1.0) || (!right_ok[i] && w > 0.0);
        v.effective_count = if w == 0.0 {
            left_n[i]
        } else if w == 1.0 {
            right_n[i]
        } else {
            left_n[i].min(right_n[i])
        };
        v.year_percentile = (1.0 - w) * v.old_percentile + w * v.new_percentile;
        v.era_raw = v.year_percentile + p.artist_weight * s.a * (1.0 - v.year_percentile)
            - profile.vote_weight * s.v
            - s.t;
        v.type_penalty = if v.type_hints & 2 != 0 && !v.layout_protected {
            cfg.comic_penalty
        } else {
            0.0
        };
        let bias = |year: Option<i32>| {
            cfg.eras
                .iter()
                .find(|e| year.is_some_and(|y| (e.from_year..=e.through_year).contains(&y)))
                .map_or(0.0, |e| e.bonus)
        };
        v.era_bonus = (1.0 - w) * bias(v.old_period) + w * bias(v.new_period);
    }
    drop((left_ok, right_ok, left_n, right_n));
    let mut order: Vec<usize> = (0..n).collect();
    let mut percentiles = vec![0.0; n];
    percentile(|i| values[i].direct_raw, &mut percentiles, &mut order);
    for (v, pct) in values.iter_mut().zip(&percentiles) {
        v.direct_percentile = *pct;
    }
    percentile(|i| values[i].era_raw, &mut percentiles, &mut order);
    for (v, pct) in values.iter_mut().zip(&percentiles) {
        v.era_percentile = *pct;
        let lambda = if p.time_enabled {
            profile.era_weight
        } else {
            0.0
        };
        v.fused_score = 100.0 * ((1.0 - lambda) * v.direct_percentile + lambda * v.era_percentile);
    }
    drop(percentiles);
    descending(samples, |i| values[i].direct_raw, &mut order);
    for (rank, &i) in order.iter().enumerate() {
        values[i].direct_rank = rank as u64 + 1;
    }
    descending(samples, |i| values[i].era_raw, &mut order);
    for (rank, &i) in order.iter().enumerate() {
        samples[i].rescue_rank = rank as u64 + 1;
        samples[i].rescue_score = 100.0 * values[i].era_percentile;
    }
    descending(samples, |i| values[i].fused_score, &mut order);
    for (rank, &i) in order.iter().enumerate() {
        values[i].fused_rank = rank as u64 + 1;
    }
    order.sort_unstable_by(|&a, &b| {
        values[a]
            .created_year
            .cmp(&values[b].created_year)
            .then(values[b].era_raw.total_cmp(&values[a].era_raw))
            .then(samples[a].hash.cmp(&samples[b].hash))
    });
    let mut year = None;
    let mut rank = 0u64;
    for &i in &order {
        if rank == 0 || values[i].created_year != year {
            year = values[i].created_year;
            rank = 0;
        }
        rank += 1;
        values[i].year_rank = rank;
    }
    for (s, v) in samples.iter_mut().zip(&values) {
        s.main_score = v.fused_score - v.type_penalty + v.era_bonus;
    }
    descending(samples, |i| samples[i].main_score, &mut order);
    for (rank, &i) in order.iter().enumerate() {
        samples[i].main_rank = rank as u64 + 1;
    }
    let mut extra = RankingV2Summary {
        metadata_only: true,
        ..Default::default()
    };
    if p.mode == RankingMode::Select {
        select(samples, &mut values, cfg, &p.seed, &mut extra, progress)?;
        summary.selected = [0; 3];
        for s in samples.iter() {
            match s.route {
                RankingRoute::Main => summary.selected[0] += 1,
                RankingRoute::Rescue => summary.selected[1] += 1,
                RankingRoute::Audit => summary.selected[2] += 1,
                _ => (),
            }
        }
        // Actual counts include returned, unused protection slots.
        summary.quotas = summary.selected;
        order.sort_unstable_by(|&a, &b| {
            samples[b]
                .fav
                .cmp(&samples[a].fav)
                .then(samples[a].hash.cmp(&samples[b].hash))
        });
        let budget = summary.selected.iter().sum::<u64>() as usize;
        summary.favorite_baseline_overlap = order
            .iter()
            .take(budget)
            .filter(|&&i| {
                matches!(
                    samples[i].route,
                    RankingRoute::Main | RankingRoute::Rescue | RankingRoute::Audit
                )
            })
            .count() as u64;
    }
    let mut years = BTreeMap::<Option<i32>, RankingYearSummary>::new();
    let head = (n as u64).div_ceil(100);
    for (i, (s, v)) in samples.iter().zip(&values).enumerate() {
        if i.is_multiple_of(8192) {
            progress("v2_diagnostics", i as u64, n as u64)?;
        }
        let y = years
            .entry(v.created_year)
            .or_insert_with(|| RankingYearSummary {
                year: v.created_year,
                ..Default::default()
            });
        y.count += 1;
        y.selected += u64::from(matches!(
            s.route,
            RankingRoute::Main | RankingRoute::Rescue | RankingRoute::Audit
        ));
        y.top_count += u64::from(s.main_rank <= head);
        y.fallback += u64::from(v.era_fallback);
        y.protected += u64::from(v.layout_protected);
        y.penalized += u64::from(v.type_penalty > 0.0);
        extra.fallback_count += u64::from(v.era_fallback);
        extra.protected_count += u64::from(v.layout_protected);
        extra.type_penalized += u64::from(v.type_penalty > 0.0);
    }
    extra.years = years.into_values().collect();
    summary.v2 = Some(extra);
    progress("ranks", 3, 3)?;
    Ok((summary, values))
}

fn hash(seed: &str, s: &Sample) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"metarecall-v2-audit\0");
    h.update(seed.as_bytes());
    h.update(s.hash);
    h.finalize().into()
}
/// Exact capacities make year targets and rescue reservations compete inside one budget.
fn select(
    samples: &mut [Sample],
    values: &mut [RankingV2Scores],
    cfg: &RankingV2Parameters,
    seed: &str,
    report: &mut RankingV2Summary,
    progress: &mut Progress<'_>,
) -> Result<()> {
    let n = samples.len();
    let budget = (n as u64 * u64::from(cfg.keep_per_mille) / 1000) as usize;
    let targeted: Vec<_> = cfg
        .eras
        .iter()
        .filter(|e| e.target_share.is_some())
        .collect();
    let bucket: Vec<usize> = values
        .iter()
        .map(|v| {
            targeted
                .iter()
                .position(|e| {
                    v.created_year
                        .is_some_and(|y| (e.from_year..=e.through_year).contains(&y))
                })
                .unwrap_or(targeted.len())
        })
        .collect();
    let mut available = vec![0usize; targeted.len() + 1];
    for &b in &bucket {
        available[b] += 1;
    }
    let mut rates: Vec<u32> = targeted
        .iter()
        .map(|e| e.target_share.unwrap_or(0))
        .collect();
    rates.push(1000 - rates.iter().sum::<u32>());
    let mut capacity: Vec<usize> = rates
        .iter()
        .map(|r| budget * (*r as usize) / 1000)
        .collect();
    let mut remainders: Vec<_> = rates
        .iter()
        .enumerate()
        .map(|(i, r)| (budget * (*r as usize) % 1000, i))
        .collect();
    remainders.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let missing = budget - capacity.iter().sum::<usize>();
    for &(_, i) in remainders.iter().take(missing) {
        capacity[i] += 1;
    }
    let mut deficit = 0;
    for (i, c) in capacity.iter_mut().enumerate() {
        if *c > available[i] {
            deficit += *c - available[i];
            *c = available[i];
        }
    }
    report.era_target_shortfall = deficit as u64;
    if deficit > 0 && cfg.strict_era_targets {
        return Err(Error::invalid(
            "年代硬配额不可行：指定年代候选不足；请调整目标或启用名额回流",
        ));
    }
    // Returned slots go to the strongest remaining candidates, regardless of year.
    let mut free = deficit;
    let mut used = vec![0usize; capacity.len()];
    for s in samples.iter_mut() {
        s.route = RankingRoute::BudgetRejected;
    }
    let mut picked = 0usize;
    let mut order: Vec<usize> = (0..n).collect();
    let direct_slots = budget * cfg.direct_rescue as usize / 1000;
    let era_slots = budget * cfg.era_rescue as usize / 1000;
    let audit_slots = budget * cfg.audit as usize / 1000;
    for (which, wanted) in [(1u8, direct_slots), (2u8, era_slots)] {
        if which == 1 {
            order.sort_unstable_by_key(|&i| values[i].direct_rank);
        } else {
            order.sort_unstable_by_key(|&i| samples[i].rescue_rank);
        }
        let mut got = 0;
        for (at, &i) in order.iter().enumerate() {
            if at.is_multiple_of(8192) {
                progress("v2_selection", at as u64, n as u64)?;
            }
            if got >= wanted {
                break;
            }
            let exclusive = if which == 1 {
                values[i].direct_rank <= budget as u64 && samples[i].rescue_rank > budget as u64
            } else {
                samples[i].rescue_rank <= budget as u64 && values[i].direct_rank > budget as u64
            };
            if !exclusive || samples[i].route != RankingRoute::BudgetRejected {
                continue;
            }
            if admit(bucket[i], &capacity, &mut used, &mut free) {
                samples[i].route = RankingRoute::Rescue;
                values[i].selection_reason = which;
                got += 1;
                picked += 1;
            }
        }
        report.protection_shortfall[(which - 1) as usize] = (wanted - got) as u64;
        if which == 1 {
            report.direct_rescued = got as u64;
        } else {
            report.era_rescued = got as u64;
        }
    }
    order.sort_unstable_by_key(|&i| samples[i].main_rank);
    for (at, &i) in order.iter().enumerate() {
        if at.is_multiple_of(8192) {
            progress("v2_selection", at as u64, n as u64)?;
        }
        if picked >= budget - audit_slots {
            break;
        }
        if samples[i].route == RankingRoute::BudgetRejected
            && admit(bucket[i], &capacity, &mut used, &mut free)
        {
            samples[i].route = RankingRoute::Main;
            picked += 1;
        }
    }
    if picked < budget {
        // The audit is sampled within remaining feasible year capacities, never an extra budget.
        let keys: Vec<_> = samples.iter().map(|s| hash(seed, s)).collect();
        order.sort_unstable_by_key(|&i| keys[i]);
        for (at, &i) in order.iter().enumerate() {
            if at.is_multiple_of(8192) {
                progress("v2_audit", at as u64, n as u64)?;
            }
            if picked >= budget {
                break;
            }
            if samples[i].route == RankingRoute::BudgetRejected
                && admit(bucket[i], &capacity, &mut used, &mut free)
            {
                samples[i].route = RankingRoute::Audit;
                values[i].selection_reason = 3;
                picked += 1;
                report.audit_selected += 1;
            }
        }
    }
    if picked != budget {
        return Err(Error::new("RANKING_INVALID", "v2 实际保留数量与预算不一致"));
    }
    Ok(())
}
fn admit(bucket: usize, capacity: &[usize], used: &mut [usize], free: &mut usize) -> bool {
    if used[bucket] < capacity[bucket] {
        used[bucket] += 1;
        true
    } else if *free > 0 {
        *free -= 1;
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests;

pub fn validate_scores(
    input: &RankingInput,
    scores: &RankingScores,
    p: &RankingParameters,
) -> Result<()> {
    let cfg =
        p.v2.as_ref()
            .ok_or_else(|| Error::new("RANKING_INVALID", "缺少 v2 参数"))?;
    let r = input
        .rating
        .as_deref()
        .ok_or_else(|| Error::new("RANKING_INVALID", "缺少分级"))?;
    let prof = &cfg.profiles[r];
    let v = scores
        .v2
        .ok_or_else(|| Error::new("RANKING_INVALID", "缺少 v2 逐图依据"))?;
    let finite = [
        v.new_weight,
        v.old_percentile,
        v.new_percentile,
        v.year_percentile,
        v.effective_count,
        v.direct_raw,
        v.era_raw,
        v.direct_percentile,
        v.era_percentile,
        v.fused_score,
        v.type_penalty,
        v.era_bonus,
    ];
    if finite.iter().any(|x| !x.is_finite())
        || v.effective_count < 0.0
        || [
            v.new_weight,
            v.old_percentile,
            v.new_percentile,
            v.year_percentile,
            v.direct_percentile,
            v.era_percentile,
        ]
        .iter()
        .any(|x| !(0.0..=1.0).contains(x))
        || v.direct_rank == 0
        || v.fused_rank == 0
        || v.year_rank == 0
    {
        return Err(Error::new("RANKING_INVALID", "v2 特征或名次超出范围"));
    }
    let f = input
        .created_at_us
        .and_then(|t| feather(t, cfg.feather_days));
    let periods = f.map(|(y, l, r, _)| (y, l, r));
    if periods
        != v.created_year
            .zip(v.old_period)
            .zip(v.new_period)
            .map(|((y, l), r)| (y, l, r))
        || (v.new_weight - f.map_or(0.0, |(_, _, _, w)| w)).abs() > 1e-10
    {
        return Err(Error::new("RANKING_INVALID", "v2 羽化权重与上传时间不一致"));
    }
    let (g, c, a, neg, t) = (
        scores.g.unwrap_or(0.0),
        scores.c.unwrap_or(0.0),
        scores.a.unwrap_or(0.0),
        scores.v.unwrap_or(0.0),
        scores.t.unwrap_or(0.0),
    );
    let direct = g + prof.time_up * (c - g).max(0.0)
        - prof.time_down * (g - c).max(0.0)
        - prof.vote_weight * neg
        - t;
    let era = v.year_percentile + p.artist_weight * a * (1.0 - v.year_percentile)
        - prof.vote_weight * neg
        - t;
    let hints = type_hints(input.tags.as_deref());
    let protected = hints & (4 | 8 | 16) != 0;
    let penalty = if hints & 2 != 0 && !protected {
        cfg.comic_penalty
    } else {
        0.0
    };
    let bias = |year: Option<i32>| {
        cfg.eras
            .iter()
            .find(|e| year.is_some_and(|y| (e.from_year..=e.through_year).contains(&y)))
            .map_or(0.0, |e| e.bonus)
    };
    let bonus = (1.0 - v.new_weight) * bias(v.old_period) + v.new_weight * bias(v.new_period);
    let lambda = if p.time_enabled { prof.era_weight } else { 0.0 };
    let fused = 100.0 * ((1.0 - lambda) * v.direct_percentile + lambda * v.era_percentile);
    let checks = [
        (v.direct_raw, direct),
        (v.era_raw, era),
        (v.type_penalty, penalty),
        (v.era_bonus, bonus),
        (v.fused_score, fused),
        (
            v.year_percentile,
            (1.0 - v.new_weight) * v.old_percentile + v.new_weight * v.new_percentile,
        ),
        (
            scores.main_score.unwrap_or(f64::NAN),
            fused - penalty + bonus,
        ),
        (
            scores.rescue_score.unwrap_or(f64::NAN),
            100.0 * v.era_percentile,
        ),
    ];
    if v.type_hints != hints
        || v.layout_protected != protected
        || checks
            .iter()
            .any(|(a, b)| !a.is_finite() || (a - b).abs() > 1e-8)
    {
        return Err(Error::new(
            "RANKING_INVALID",
            "v2 分数组成、类型依据或年代偏好不一致",
        ));
    }
    let reason_ok = matches!(
        (scores.selected_route, v.selection_reason),
        (
            RankingRoute::Ranked | RankingRoute::BudgetRejected | RankingRoute::Main,
            0
        ) | (RankingRoute::Rescue, 1 | 2)
            | (RankingRoute::Audit, 3)
    );
    if !reason_ok {
        return Err(Error::new("RANKING_INVALID", "v2 入选原因与通道不一致"));
    }
    Ok(())
}
