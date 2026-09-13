//! Pure, deterministic population computation over compact frozen metadata.
use chrono::{DateTime, Months, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use studio_application::Operator;
use studio_domain::*;

pub struct MetaRecall;
impl Operator for MetaRecall {
    fn population(&self) -> bool {
        true
    }
    fn descriptor(&self) -> OperatorDescriptor {
        OperatorDescriptor {
            id: RANKING_OPERATOR.into(),
            name: "Danbooru 元数据排名".into(),
            version: 1,
            parameters_version: 1,
            parameters: vec![ParameterDescriptor {
                id: "ranking".into(),
                name: "MetaRecall v1 参数".into(),
                value_type: "ranking_parameters".into(),
                default_value: json!(RankingParameters::default()),
                required: true,
            }],
            input_scopes: ["source", "query_result", "workset", "selection"]
                .map(String::from)
                .into(),
            outputs: vec![OutputDescriptor {
                id: "data".into(),
                name: "元数据排名".into(),
                kind: RANKING_KIND.into(),
                schema_version: 1,
                subject: "asset".into(),
            }],
            capabilities: OperatorCapabilities {
                cancel: true,
                checkpoint: true,
                retry: true,
                deterministic: true,
                item_failures: true,
            },
            resources: ResourceRequirements {
                cpu_slots: 1,
                memory_bytes: 4 << 30,
                media_reads: false,
                gpu: false,
            },
        }
    }
    fn normalize(&self, parameters: Value) -> Result<Value> {
        let parameters: RankingParameters = serde_json::from_value(parameters)
            .map_err(|e| Error::invalid(format!("排名参数无效：{e}")))?;
        if parameters.v2.is_some() {
            return Err(Error::invalid("v2 参数请使用 MetaRecall v2 方案"));
        }
        serde_json::to_value(parameters.normalize()?).map_err(Error::io)
    }
    fn required_fields(&self, _: &Value) -> Result<Vec<ScalarInput>> {
        Ok(vec![])
    }
    fn row(&self, _: &FrozenInput, _: u64, _: &Value) -> Result<Value> {
        Err(Error::new(
            "POPULATION_REQUIRED",
            "元数据排名需要固定的总体输入表",
        ))
    }
}

pub const DAY: i64 = 86_400_000_000;
const BASE: &[i64] = &[0, 3, 7, 14, 30, 90, 365, 1095, 2555, i64::MAX];
const COARSE: &[i64] = &[0, 7, 30, 365, 2555, i64::MAX];
const BROAD: &[i64] = &[0, 30, 365, 2555, i64::MAX];
const FLAG_NAMES: [&str; 16] = [
    "fav_unknown_or_invalid",
    "up_unknown_or_invalid",
    "down_unknown_or_invalid",
    "created_time_unknown",
    "observation_time_unknown",
    "observation_date_only",
    "observation_time_invalid",
    "artist_unknown",
    "score_inconsistent",
    "stored_dimensions_unknown",
    "tags_unknown",
    "rating_conflict",
    "source_issues",
    "cohort_insufficient",
    "duplicate_heat_partial",
    "duplicate_heat_clamped",
];
pub fn flag_names(bits: u64) -> Vec<String> {
    FLAG_NAMES
        .iter()
        .enumerate()
        .filter(|(i, _)| bits & (1 << i) != 0)
        .map(|(_, name)| (*name).into())
        .collect()
}
fn valid_count(value: Option<i64>) -> Option<u64> {
    value.and_then(|v| u64::try_from(v).ok())
}
fn down(value: Option<i64>) -> Option<u64> {
    value.and_then(i64::checked_abs).map(|v| v as u64)
}
pub fn heat_key(fav: Option<i64>, up: Option<i64>) -> Option<u128> {
    match (valid_count(fav), valid_count(up)) {
        (Some(f), Some(u)) => Some((u128::from(f) + 1) * (u128::from(u) + 1)),
        (Some(v), None) | (None, Some(v)) => Some((u128::from(v) + 1).pow(2)),
        _ => None,
    }
}
pub fn eligibility(input: &RankingInput, p: &RankingParameters) -> RankingEligibility {
    if input.record_id.is_none() || input.observation_id.is_none() {
        return RankingEligibility::MetadataUnavailable;
    }
    let Some(rating) = input
        .rating
        .as_deref()
        .filter(|r| matches!(*r, "g" | "s" | "q" | "e"))
    else {
        return RankingEligibility::RatingUnknown;
    };
    if !p.ratings.iter().any(|r| r == rating) {
        return RankingEligibility::RatingExcluded;
    }
    if p.exclude_banned && input.is_banned == Some(true) {
        return RankingEligibility::PolicyExcluded;
    }
    if let Some(minimum) = p.minimum_stored_side {
        match (
            input.stored_width.filter(|v| *v > 0),
            input.stored_height.filter(|v| *v > 0),
        ) {
            (Some(w), Some(h)) if w.min(h) < minimum => {
                return RankingEligibility::DimensionsExcluded;
            }
            (None, _) | (_, None) => return RankingEligibility::DimensionsUnknown,
            _ => (),
        }
    }
    RankingEligibility::Eligible
}
pub fn flags(input: &RankingInput) -> u64 {
    let age = age_interval(
        input.created_at_us,
        input.observed_at_us,
        &input.time_quality,
    );
    let values = [
        valid_count(input.fav_count).is_none(),
        valid_count(input.up_score).is_none(),
        down(input.down_score).is_none(),
        input.created_at_us.is_none(),
        input.observed_at_us.is_none()
            || !matches!(input.time_quality.as_str(), "exact" | "date_only"),
        input.time_quality == "date_only",
        age.is_none()
            && input.observed_at_us.is_some()
            && input.created_at_us.is_some()
            && matches!(input.time_quality.as_str(), "exact" | "date_only"),
        input.artists.is_empty(),
        matches!((input.score, valid_count(input.up_score), down(input.down_score)), (Some(s),Some(u),Some(d)) if i128::from(s) != i128::from(u)-i128::from(d)),
        input.dimension_basis != "not_requested"
            && (input.stored_width.is_none_or(|v| v == 0)
                || input.stored_height.is_none_or(|v| v == 0)),
        !input.tags_known,
        input.rating_conflict,
        input
            .source_issues
            .as_deref()
            .is_some_and(|s| !matches!(s.trim(), "" | "[]" | "{}" | "null")),
        false,
        input.evidence.as_ref().is_some_and(|e| e.partial_counts),
        input.evidence.as_ref().is_some_and(|e| e.counts_clamped),
    ];
    values
        .iter()
        .enumerate()
        .fold(0, |bits, (i, set)| bits | (u64::from(*set) << i))
}
pub fn age_interval(
    created: Option<i64>,
    observed: Option<i64>,
    quality: &str,
) -> Option<(i64, i64)> {
    let (created, observed) = (created?, observed?);
    DateTime::<Utc>::from_timestamp_micros(created)?;
    let end = match quality {
        "exact" => observed,
        "date_only" => observed.checked_add(DAY - 1)?,
        _ => return None,
    };
    let lower = observed.checked_sub(created)?.max(0);
    let upper = end.checked_sub(created)?;
    (upper >= 0).then_some((lower, upper))
}
pub fn month_window(created: i64, months: u32) -> Option<(i64, i64)> {
    let date = DateTime::<Utc>::from_timestamp_micros(created)?;
    Some((
        date.checked_sub_months(Months::new(months))?
            .timestamp_micros(),
        date.checked_add_months(Months::new(months))?
            .timestamp_micros(),
    ))
}
fn bucket(age: (i64, i64), edges: &[i64]) -> Option<usize> {
    edges
        .windows(2)
        .position(|b| age.0 >= b[0].saturating_mul(DAY) && age.1 < b[1].saturating_mul(DAY))
}
fn digest(seed: &str, domain: &str, asset: &[u8; 32]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"metarecall-v1\0");
    hash.update(domain.as_bytes());
    hash.update([0]);
    hash.update((seed.len() as u64).to_be_bytes());
    hash.update(seed.as_bytes());
    hash.update(asset);
    hash.finalize().into()
}

/// Strings and complete provenance stay on disk. One rating's numeric working set is admitted at a time.
#[derive(Debug, Clone)]
pub struct Sample {
    pub ordinal: u64,
    pub hash: [u8; 32],
    pub heat: Option<u128>,
    pub fav: Option<u64>,
    pub votes: Option<(u64, u64)>,
    pub support: u64,
    pub created: Option<i64>,
    pub age: Option<(i64, i64)>,
    pub post_id: Option<i64>,
    pub parent_id: Option<i64>,
    pub flags: u64,
    pub g: f64,
    pub c: f64,
    pub a: f64,
    pub v: f64,
    pub t: f64,
    pub local_p: Option<f64>,
    pub local_n: u64,
    pub k: Option<f64>,
    pub level: Option<u32>,
    pub artist_support: u64,
    pub main_score: f64,
    pub rescue_score: f64,
    pub main_rank: u64,
    pub rescue_rank: u64,
    pub route: RankingRoute,
    /// 0 disabled, 1 unknown time, 2 invalid time, 3 unknown heat, 4 insufficient, 5 used.
    pub time_reason: u8,
}
impl Sample {
    pub fn new(input: &RankingInput, p: &RankingParameters) -> Result<Self> {
        let mut hash = [0; 32];
        hex::decode_to_slice(&input.asset_id, &mut hash)
            .map_err(|_| Error::new("INPUT_INVALID", "排名输入缺少有效的图片字节哈希"))?;
        Ok(Self {
            ordinal: input.ordinal,
            hash,
            heat: heat_key(input.fav_count, input.up_score),
            fav: valid_count(input.fav_count),
            votes: valid_count(input.up_score).zip(down(input.down_score)),
            support: valid_count(input.fav_count)
                .unwrap_or(0)
                .max(valid_count(input.up_score).unwrap_or(0)),
            created: input.created_at_us,
            age: age_interval(
                input.created_at_us,
                input.observed_at_us,
                &input.time_quality,
            ),
            post_id: input.post_id,
            parent_id: input.parent_id,
            flags: flags(input),
            g: 0.5,
            c: 0.5,
            a: 0.0,
            v: 0.0,
            t: if p.damage_enabled {
                (f64::from((input.damage_classes & 3).count_ones()) * p.damage_weight).min(0.08)
            } else {
                0.0
            },
            local_p: None,
            local_n: 0,
            k: None,
            level: None,
            artist_support: 0,
            main_score: 0.0,
            rescue_score: 0.0,
            main_rank: 0,
            rescue_rank: 0,
            route: RankingRoute::Ranked,
            time_reason: 0,
        })
    }
    pub fn scores(&self, rating: &str) -> RankingScores {
        RankingScores {
            v2: None,
            ordinal: self.ordinal,
            rating: Some(rating.into()),
            eligibility: RankingEligibility::Eligible,
            missing_flags: flag_names(self.flags),
            g: Some(self.g),
            c: Some(self.c),
            a: Some(self.a),
            v: Some(self.v),
            t: Some(self.t),
            local_percentile: self.local_p,
            local_count: self.local_n,
            support_k: self.k,
            cohort_level: self.level,
            artist_support: self.artist_support,
            main_score: Some(self.main_score),
            rescue_score: Some(self.rescue_score),
            main_rank: Some(self.main_rank),
            rescue_rank: Some(self.rescue_rank),
            selected_route: self.route,
            time_reason: [
                "disabled",
                "time_unknown",
                "time_invalid",
                "heat_unknown",
                "cohort_insufficient",
                "used",
            ][usize::from(self.time_reason)]
            .into(),
            duplicate_of: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ArtistWork {
    pub item: usize,
    pub artist: u32,
}
pub type Progress<'a> = dyn FnMut(&str, u64, u64) -> Result<()> + 'a;

/// Largest remainder apportionment, entirely in integer arithmetic.
pub fn quotas(n: u64, rates: [u32; 3]) -> [u64; 3] {
    let numerators = rates.map(|p| u128::from(n) * u128::from(p));
    let mut result = numerators.map(|v| (v / 1_000) as u64);
    let total = (numerators.iter().sum::<u128>() / 1_000) as u64;
    let mut order = [0, 1, 2];
    order.sort_by_key(|&i| (std::cmp::Reverse(numerators[i] % 1_000), i));
    for &i in order
        .iter()
        .take((total - result.iter().sum::<u64>()) as usize)
    {
        result[i] += 1;
    }
    result
}

struct Fenwick {
    values: Vec<u32>,
}
impl Fenwick {
    fn new(n: usize) -> Self {
        Self {
            values: vec![0; n + 1],
        }
    }
    fn add(&mut self, index: usize, add: bool) {
        let mut i = index + 1;
        while i < self.values.len() {
            if add {
                self.values[i] += 1;
            } else {
                self.values[i] -= 1;
            }
            i += i & i.wrapping_neg();
        }
    }
    fn prefix(&self, mut end: usize) -> u64 {
        let mut sum = 0;
        while end > 0 {
            sum += u64::from(self.values[end]);
            end &= end - 1;
        }
        sum
    }
    fn select(&self, ordinal: u64) -> usize {
        let (mut index, mut seen) = (0usize, 0u64);
        let mut step = self.values.len().next_power_of_two();
        while step != 0 {
            let next = index + step;
            if next < self.values.len() && seen + u64::from(self.values[next]) < ordinal {
                index = next;
                seen += u64::from(self.values[next]);
            }
            step >>= 1;
        }
        index
    }
}

pub fn compute_rating(
    rating: &str,
    samples: &mut [Sample],
    p: &RankingParameters,
    artists: &mut [ArtistWork],
    progress: &mut Progress<'_>,
) -> Result<RankingRatingSummary> {
    let n = samples.len() as u64;
    let mut summary = RankingRatingSummary {
        rating: rating.into(),
        eligible: n,
        ..Default::default()
    };
    if n == 0 {
        return Ok(summary);
    }
    for s in samples.iter_mut() {
        s.g = 0.5;
        s.c = 0.5;
        s.a = 0.0;
        s.v = 0.0;
        s.local_p = None;
        s.local_n = 0;
        s.k = None;
        s.level = None;
        s.artist_support = 0;
        s.flags &= !(1 << 13);
        s.route = RankingRoute::Ranked;
    }
    progress("heat", 0, n)?;
    let mut ordered: Vec<usize> = (0..samples.len())
        .filter(|&i| samples[i].heat.is_some())
        .collect();
    ordered.sort_unstable_by_key(|&i| samples[i].heat);
    summary.valid_heat = ordered.len() as u64;
    let mut start = 0;
    while start < ordered.len() {
        let mut end = start + 1;
        while end < ordered.len() && samples[ordered[end]].heat == samples[ordered[start]].heat {
            end += 1;
        }
        let g = (start as f64 + 0.5 * (end - start) as f64) / ordered.len() as f64;
        for &i in &ordered[start..end] {
            samples[i].g = g;
        }
        start = end;
    }
    drop(ordered);
    let (mut total_u, mut total_d) = (0u128, 0u128);
    for s in samples.iter() {
        if let Some((u, d)) = s.votes {
            total_u += u128::from(u);
            total_d += u128::from(d);
        }
    }
    summary.q0 = if total_u + total_d > 0 {
        total_d as f64 / (total_u + total_d) as f64
    } else {
        0.0
    };
    for s in samples.iter_mut() {
        s.c = s.g;
        s.time_reason = if !p.time_enabled {
            0
        } else if s.heat.is_none() {
            3
        } else if s.flags & (1 << 6) != 0 {
            2
        } else if s.age.is_none() {
            1
        } else {
            4
        };
        if p.votes_enabled
            && let Some((u, d)) = s.votes
            && d >= 3
        {
            let smooth = (d as f64 + 20.0 * summary.q0) / (u as f64 + d as f64 + 20.0);
            s.v = ((smooth - summary.q0 - 0.05) / 0.25).clamp(0.0, 1.0);
        }
    }
    if p.time_enabled && summary.valid_heat >= u64::from(p.cohort_minimum) {
        let stages = [(6, BASE), (12, BASE), (24, BASE), (24, COARSE), (24, BROAD)];
        for (level, (months, edges)) in stages.into_iter().enumerate() {
            if !samples.iter().any(|s| s.time_reason == 4) {
                break;
            }
            progress("time", level as u64, 5)?;
            for b in 0..edges.len() - 1 {
                let members: Vec<usize> = (0..samples.len())
                    .filter(|&i| {
                        samples[i].heat.is_some()
                            && samples[i].created.is_some()
                            && samples[i].age.and_then(|age| bucket(age, edges)) == Some(b)
                    })
                    .collect();
                if members.len() < p.cohort_minimum as usize
                    || !members.iter().any(|&i| samples[i].time_reason == 4)
                {
                    continue;
                }
                local_cohort(
                    samples,
                    members,
                    months,
                    level as u32,
                    p.cohort_minimum,
                    progress,
                )?;
            }
        }
    }
    for s in samples.iter_mut() {
        if s.time_reason == 5 {
            summary.time_used += 1;
            summary.cohort_counts[s.level.expect("used level") as usize] += 1;
        } else {
            summary.time_fallback += 1;
            if s.time_reason == 4 {
                s.flags |= 1 << 13;
            }
        }
    }
    if p.artist_enabled && !artists.is_empty() {
        artist_priors(samples, artists, progress)?;
    }
    progress("scores", 0, n)?;
    for s in samples.iter_mut() {
        if s.a > 0.0 {
            summary.artist_used += 1;
        }
        s.main_score = 100.0
            * (s.g + p.time_weight * (s.c - s.g).max(0.0) - p.vote_weight * s.v - s.t)
                .clamp(0.0, 1.0);
        s.rescue_score = 100.0
            * (s.c + p.artist_weight * s.a * (1.0 - s.c) - p.vote_weight * s.v - s.t)
                .clamp(0.0, 1.0);
    }
    let tie_keys: Vec<[u8; 32]> = samples
        .iter()
        .map(|s| digest(&p.seed, "score-tie", &s.hash))
        .collect();
    let mut order: Vec<usize> = (0..samples.len()).collect();
    order.sort_unstable_by(|&a, &b| {
        samples[b]
            .main_score
            .total_cmp(&samples[a].main_score)
            .then_with(|| tie_keys[a].cmp(&tie_keys[b]))
            .then_with(|| samples[a].hash.cmp(&samples[b].hash))
    });
    for (rank, &i) in order.iter().enumerate() {
        samples[i].main_rank = rank as u64 + 1;
    }
    summary.quotas = if p.mode == RankingMode::Select {
        quotas(n, p.quotas)
    } else {
        [0; 3]
    };
    if p.mode == RankingMode::Select {
        for &i in order.iter().take(summary.quotas[0] as usize) {
            samples[i].route = RankingRoute::Main;
        }
    }
    progress("ranks", 1, 3)?;
    order.sort_unstable_by(|&a, &b| {
        samples[b]
            .rescue_score
            .total_cmp(&samples[a].rescue_score)
            .then_with(|| tie_keys[a].cmp(&tie_keys[b]))
            .then_with(|| samples[a].hash.cmp(&samples[b].hash))
    });
    for (rank, &i) in order.iter().enumerate() {
        samples[i].rescue_rank = rank as u64 + 1;
    }
    if p.mode == RankingMode::Select {
        let mut remaining = summary.quotas[1];
        for &i in &order {
            if remaining > 0 && samples[i].route != RankingRoute::Main {
                samples[i].route = RankingRoute::Rescue;
                remaining -= 1;
            }
        }
        let audit_keys: Vec<[u8; 32]> = samples
            .iter()
            .map(|s| digest(&p.seed, "audit", &s.hash))
            .collect();
        order.sort_unstable_by(|&a, &b| {
            audit_keys[a]
                .cmp(&audit_keys[b])
                .then_with(|| samples[a].hash.cmp(&samples[b].hash))
        });
        remaining = summary.quotas[2];
        for &i in &order {
            if samples[i].route == RankingRoute::Ranked {
                samples[i].route = if remaining > 0 {
                    remaining -= 1;
                    RankingRoute::Audit
                } else {
                    RankingRoute::BudgetRejected
                };
            }
        }
        order.sort_unstable_by(|&a, &b| {
            samples[b]
                .fav
                .cmp(&samples[a].fav)
                .then_with(|| tie_keys[a].cmp(&tie_keys[b]))
        });
        let budget = summary.quotas.iter().sum::<u64>() as usize;
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
        for s in samples.iter() {
            match s.route {
                RankingRoute::Main => summary.selected[0] += 1,
                RankingRoute::Rescue => summary.selected[1] += 1,
                RankingRoute::Audit => summary.selected[2] += 1,
                _ => (),
            }
        }
    }
    progress("ranks", 3, 3)?;
    Ok(summary)
}

fn local_cohort(
    samples: &mut [Sample],
    mut members: Vec<usize>,
    months: u32,
    level: u32,
    minimum: u32,
    progress: &mut Progress<'_>,
) -> Result<()> {
    members.sort_unstable_by_key(|&i| (samples[i].created, samples[i].ordinal));
    let mut keys: Vec<u128> = members
        .iter()
        .map(|&i| samples[i].heat.expect("valid heat"))
        .collect();
    keys.sort_unstable();
    keys.dedup();
    let coordinates: Vec<usize> = members
        .iter()
        .map(|&i| {
            keys.binary_search(&samples[i].heat.expect("valid heat"))
                .expect("heat coordinate")
        })
        .collect();
    let mut heat = Fenwick::new(keys.len());
    // Only the clipped median matters. Any positive value >80 may be 81:
    // even with a lower middle value of 1, its mean already exceeds the K cap.
    let mut support = Fenwick::new(82);
    let (mut left, mut right, mut positive) = (0, 0, 0u64);
    for (position, &item) in members.iter().enumerate() {
        if position.is_multiple_of(8192) {
            progress("time_window", position as u64, members.len() as u64)?;
        }
        if samples[item].time_reason != 4 {
            continue;
        }
        let Some((lower, upper)) = month_window(samples[item].created.expect("created"), months)
        else {
            continue;
        };
        // Calendar-month clamping may move a boundary backwards at month end.
        // Maintain both ends by their actual bounds, rather than assuming monotonic offsets.
        let next_left = members.partition_point(|&i| samples[i].created.expect("created") < lower);
        let next_right =
            members.partition_point(|&i| samples[i].created.expect("created") <= upper);
        while right < next_right {
            heat.add(coordinates[right], true);
            let m = samples[members[right]].support;
            if m > 0 {
                support.add(m.min(81) as usize, true);
                positive += 1;
            }
            right += 1;
        }
        while left > next_left {
            left -= 1;
            heat.add(coordinates[left], true);
            let m = samples[members[left]].support;
            if m > 0 {
                support.add(m.min(81) as usize, true);
                positive += 1;
            }
        }
        while right > next_right {
            right -= 1;
            heat.add(coordinates[right], false);
            let m = samples[members[right]].support;
            if m > 0 {
                support.add(m.min(81) as usize, false);
                positive -= 1;
            }
        }
        while left < next_left {
            heat.add(coordinates[left], false);
            let m = samples[members[left]].support;
            if m > 0 {
                support.add(m.min(81) as usize, false);
                positive -= 1;
            }
            left += 1;
        }
        let count = (right - left) as u64;
        if count < u64::from(minimum) {
            continue;
        }
        let before = heat.prefix(coordinates[position]);
        let through = heat.prefix(coordinates[position] + 1);
        let local = (before as f64 + 0.5 * (through - before) as f64) / count as f64;
        let k = if positive == 0 {
            1.0
        } else {
            let a = support.select(positive.div_ceil(2));
            let b = support.select(positive / 2 + 1);
            (0.125 * (a + b) as f64).clamp(1.0, 10.0)
        };
        let q = 0.5 + (count as f64 / (count as f64 + 1000.0)) * (local - 0.5);
        let m = samples[item].support as f64;
        let c = if q <= 0.5 {
            q
        } else {
            0.5 + m / (m + k) * (q - 0.5)
        };
        let s = &mut samples[item];
        s.c = c;
        s.local_p = Some(local);
        s.local_n = count;
        s.k = Some(k);
        s.level = Some(level);
        s.time_reason = 5;
    }
    Ok(())
}

fn artist_priors(
    samples: &mut [Sample],
    works: &mut [ArtistWork],
    progress: &mut Progress<'_>,
) -> Result<()> {
    let mu = samples.iter().map(|s| s.c).sum::<f64>() / samples.len() as f64;
    let groups = related_groups(samples);
    works.sort_unstable_by_key(|w| (w.artist, samples[w.item].created, samples[w.item].ordinal));
    let mut valid_artists = vec![0u32; samples.len()];
    let mut start = 0;
    while start < works.len() {
        let mut end = start + 1;
        while end < works.len() && works[end].artist == works[start].artist {
            end += 1;
        }
        let members: Vec<usize> = works[start..end]
            .iter()
            .map(|w| w.item)
            .filter(|&i| samples[i].created.is_some())
            .collect();
        let (mut left, mut right) = (0, 0);
        let mut active = HashMap::<usize, (f64, u32)>::new();
        let mut sum_means = 0.0;
        for (at, &item) in members.iter().enumerate() {
            if at.is_multiple_of(8192) {
                progress("artists", start as u64 + at as u64, works.len() as u64)?;
            }
            let Some((lo, hi)) = month_window(samples[item].created.expect("created"), 36) else {
                continue;
            };
            let next_left = members.partition_point(|&i| samples[i].created.expect("created") < lo);
            let next_right =
                members.partition_point(|&i| samples[i].created.expect("created") <= hi);
            while right < next_right {
                let j = members[right];
                let state = active.entry(groups[j]).or_default();
                if state.1 > 0 {
                    sum_means -= state.0 / f64::from(state.1);
                }
                state.0 += samples[j].c;
                state.1 += 1;
                sum_means += state.0 / f64::from(state.1);
                right += 1;
            }
            while left > next_left {
                left -= 1;
                let j = members[left];
                let state = active.entry(groups[j]).or_default();
                if state.1 > 0 {
                    sum_means -= state.0 / f64::from(state.1);
                }
                state.0 += samples[j].c;
                state.1 += 1;
                sum_means += state.0 / f64::from(state.1);
            }
            while right > next_right {
                right -= 1;
                let j = members[right];
                let state = active.get_mut(&groups[j]).expect("active group");
                sum_means -= state.0 / f64::from(state.1);
                state.0 -= samples[j].c;
                state.1 -= 1;
                if state.1 > 0 {
                    sum_means += state.0 / f64::from(state.1);
                } else {
                    active.remove(&groups[j]);
                }
            }
            while left < next_left {
                let j = members[left];
                let state = active.get_mut(&groups[j]).expect("active group");
                sum_means -= state.0 / f64::from(state.1);
                state.0 -= samples[j].c;
                state.1 -= 1;
                if state.1 > 0 {
                    sum_means += state.0 / f64::from(state.1);
                } else {
                    active.remove(&groups[j]);
                }
                left += 1;
            }
            let own = active.get(&groups[item]).expect("current group");
            let mut n = active.len() as u64;
            let mut sum = sum_means - own.0 / f64::from(own.1);
            if own.1 > 1 {
                sum += (own.0 - samples[item].c) / f64::from(own.1 - 1);
            } else {
                n -= 1;
            }
            if n >= 5 {
                let posterior = (sum + 50.0 * mu) / (n as f64 + 50.0);
                samples[item].a += ((posterior - mu) / 0.20).clamp(0.0, 1.0);
                samples[item].artist_support = samples[item].artist_support.max(n);
                valid_artists[item] += 1;
            }
        }
        start = end;
    }
    for (s, n) in samples.iter_mut().zip(valid_artists) {
        if n > 0 {
            s.a /= f64::from(n);
        }
    }
    Ok(())
}

fn related_groups(samples: &[Sample]) -> Vec<usize> {
    fn root(parent: &mut [usize], mut at: usize) -> usize {
        while parent[at] != at {
            parent[at] = parent[parent[at]];
            at = parent[at];
        }
        at
    }
    let mut ids = BTreeMap::new();
    for s in samples {
        for id in [s.post_id, s.parent_id].into_iter().flatten() {
            if id > 0 {
                let next = ids.len();
                ids.entry(id).or_insert(next);
            }
        }
    }
    let mut parent: Vec<usize> = (0..ids.len()).collect();
    for s in samples {
        if let (Some(a), Some(b)) = (
            s.post_id.and_then(|id| ids.get(&id)),
            s.parent_id.and_then(|id| ids.get(&id)),
        ) {
            let (a, b) = (root(&mut parent, *a), root(&mut parent, *b));
            parent[a] = b;
        }
    }
    samples
        .iter()
        .enumerate()
        .map(|(i, s)| {
            s.post_id
                .and_then(|id| ids.get(&id))
                .map(|&at| root(&mut parent, at))
                .unwrap_or(ids.len() + i)
        })
        .collect()
}

#[cfg(test)]
#[path = "ranking_tests.rs"]
mod tests;
