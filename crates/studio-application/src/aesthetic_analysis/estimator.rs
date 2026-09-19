//! Two experimental estimators. Whole batches are the resampling unit.
//! Davidson uses a regularized, batch-normalized composite objective, not a
//! listwise likelihood. No pairwise Fisher standard errors are reported.
use super::*;
use std::collections::BTreeMap;

const TOLERANCE: f64 = 1e-5;
const SCORE_GRID: f64 = 1e-6;
pub const MAX_CANDIDATES: u64 = studio_domain::aesthetic::AESTHETIC_MAX_CANDIDATES;
pub fn working_bytes(n: u64) -> u64 {
    n.saturating_mul(384).saturating_add(32 << 20)
}

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Default)]
struct Meta {
    rating: u8,
    year: Option<i32>,
}
#[derive(Clone, Default)]
struct Signal {
    exposures: u32,
    unjudgeable: u32,
    protected: bool,
    opponents: u64,
    cross_year: u32,
    batches: u32,
    borda: f64,
    residual: f64,
}
struct FitState {
    scores: Vec<f64>,
    parent: Vec<u32>,
    size: Vec<u32>,
    signals: Vec<Signal>,
    mass: Vec<f64>,
    groups: BTreeMap<u8, AestheticRatingSummary>,
    iterations: u32,
    max_update: f64,
    converged: bool,
}
#[derive(Clone, Copy, Default)]
struct Rank {
    lo: u32,
    hi: u32,
    percentile: f64,
    position: u32,
}
fn rating(value: &str) -> u8 {
    match value {
        "g" => 1,
        "s" => 2,
        "q" => 3,
        "e" => 4,
        _ => 0,
    }
}
fn rating_name(value: u8) -> &'static str {
    match value {
        1 => "g",
        2 => "s",
        3 => "q",
        4 => "e",
        _ => "unknown",
    }
}
fn root(parent: &mut [u32], mut x: usize) -> usize {
    while parent[x] as usize != x {
        parent[x] = parent[parent[x] as usize];
        x = parent[x] as usize;
    }
    x
}
pub fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e3779b97f4a7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}
/// Stable three-outcome log probability, mean and variance of win + half tie.
fn terms(difference: f64, tie_strength: f64, y: f64) -> (f64, f64, f64) {
    let d = difference * 0.5;
    let tie = tie_strength.ln();
    let max = d.abs().max(tie);
    let win = (d - max).exp();
    let loss = (-d - max).exp();
    let draw = (tie - max).exp();
    let z = win + loss + draw;
    let mean = (win + 0.5 * draw) / z;
    let variance = ((win + 0.25 * draw) / z - mean * mean).max(0.0);
    let logp = if y == 1.0 {
        d
    } else if y == 0.0 {
        -d
    } else {
        tie
    } - max
        - z.ln();
    (mean, variance, logp)
}
fn expected(difference: f64, tie_strength: f64) -> f64 {
    terms(difference, tie_strength, 0.5).0
}
fn visit(
    source: &dyn AestheticReplaySource,
    input: &AestheticAnalysisInput,
    split: Option<(u32, u64)>,
    check: &dyn Fn() -> Result<()>,
    f: &mut dyn FnMut(&AestheticReplayObservation) -> Result<()>,
) -> Result<()> {
    let mut after = 0;
    loop {
        check()?;
        let rows = source.replay_observations(input, after)?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            if row.batch <= after {
                return Err(Error::new("EVIDENCE_INVALID", "评审重放游标不递增"));
            }
            after = row.batch;
            if split.is_none_or(|(seed, half)| mix(row.batch ^ u64::from(seed)) & 1 == half) {
                f(&row)?;
            }
        }
    }
    Ok(())
}
fn pairs(row: &AestheticReplayObservation, mut f: impl FnMut(usize, usize, f64, f64)) {
    let members: Vec<_> = row
        .tiers
        .iter()
        .enumerate()
        .flat_map(|(t, ids)| ids.iter().map(move |id| (*id as usize, t)))
        .collect();
    let n = members.len();
    if n < 2 {
        return;
    }
    let weight = 2.0 / (n * (n - 1)) as f64;
    for a in 0..n {
        for b in a + 1..n {
            f(
                members[a].0,
                members[b].0,
                if members[a].1 == members[b].1 {
                    0.5
                } else {
                    1.0
                },
                weight,
            );
        }
    }
}
fn fit(
    source: &dyn AestheticReplaySource,
    input: &AestheticAnalysisInput,
    config: &AestheticEstimator,
    meta: &[Meta],
    split: Option<(u32, u64)>,
    check: &dyn Fn() -> Result<()>,
    progress: &mut dyn FnMut(&str, u64, u64) -> Result<()>,
) -> Result<FitState> {
    let n = meta.len();
    let mut state = FitState {
        scores: vec![0.0; n],
        parent: (0..n as u32).collect(),
        size: vec![0; n],
        signals: vec![Signal::default(); n],
        mass: vec![0.0; n],
        groups: BTreeMap::new(),
        iterations: 0,
        max_update: 0.0,
        converged: false,
    };
    for m in meta {
        let group = state.groups.entry(m.rating).or_default();
        group.rating = rating_name(m.rating).into();
        group.candidates += 1;
    }
    visit(source, input, split, check, &mut |row| {
        let mut ids = Vec::new();
        for tier in &row.tiers {
            ids.extend(tier.iter().copied());
        }
        let mut all = ids.clone();
        all.extend(&row.unjudgeable);
        all.sort_unstable();
        if all.len() > 16
            || all.windows(2).any(|v| v[0] == v[1])
            || all.iter().any(|id| {
                *id as usize >= n
                    || meta[*id as usize].rating != rating(&row.rating)
                    || rating(&row.rating) == 0
            })
            || row.elite.iter().any(|id| !ids.contains(id))
        {
            return Err(Error::new(
                "EVIDENCE_INVALID",
                "重放证据的成员或 Rating 不一致",
            ));
        }
        let cross = ids
            .iter()
            .filter_map(|id| meta[*id as usize].year)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            > 1;
        if cross {
            state
                .groups
                .get_mut(&rating(&row.rating))
                .expect("group")
                .cross_year_batches += 1;
        }
        for &id in &ids {
            let s = &mut state.signals[id as usize];
            s.exposures += 1;
            s.cross_year += u32::from(cross);
            s.protected |= row.elite.contains(&id);
            if ids.len() > 1 {
                s.batches += 1;
            }
        }
        for &id in &row.unjudgeable {
            state.signals[id as usize].unjudgeable += 1;
        }
        if let Some(&first) = ids.first().filter(|_| ids.len() > 1) {
            for &id in &ids {
                let a = root(&mut state.parent, first as usize);
                let b = root(&mut state.parent, id as usize);
                state.parent[a.max(b)] = a.min(b) as u32;
            }
        }
        pairs(row, |i, j, y, w| {
            state.mass[i] += w;
            state.mass[j] += w;
            state.signals[i].borda += y / (ids.len() - 1) as f64;
            state.signals[j].borda += (1.0 - y) / (ids.len() - 1) as f64;
            state.signals[i].opponents |= 1 << (mix(j as u64) & 63);
            state.signals[j].opponents |= 1 << (mix(i as u64) & 63);
        });
        Ok(())
    })?;
    for i in 0..n {
        let r = root(&mut state.parent, i);
        state.parent[i] = r as u32;
        if state.mass[i] > 0.0 {
            state.size[r] += 1;
        }
    }
    for (i, m) in meta.iter().enumerate() {
        let group = state.groups.get_mut(&m.rating).expect("group");
        group.judged += u64::from(state.signals[i].exposures > 0);
        group.compared += u64::from(state.mass[i] > 0.0);
        group.components += u64::from(state.parent[i] as usize == i && state.size[i] > 1);
        group.protected += u64::from(state.signals[i].protected);
    }
    for g in state.groups.values_mut() {
        g.fully_connected = g.compared == g.candidates && g.components == 1;
    }
    if config.kind == "borda_v1" {
        for i in 0..n {
            if state.signals[i].batches > 0 {
                state.scores[i] = state.signals[i].borda / f64::from(state.signals[i].batches);
            }
        }
        state.iterations = 1;
        state.converged = true;
        return Ok(state);
    }
    let mut gradient = vec![0.0; n];
    let mut curvature = vec![0.0; n];
    for iteration in 0..config.iterations {
        check()?;
        progress(
            "fitting",
            u64::from(iteration),
            u64::from(config.iterations),
        )?;
        for (g, s) in gradient.iter_mut().zip(&state.scores) {
            *g = -config.regularization * s;
        }
        curvature.fill(config.regularization);
        let mut objective =
            -0.5 * config.regularization * state.scores.iter().map(|s| s * s).sum::<f64>();
        visit(source, input, split, check, &mut |row| {
            pairs(row, |i, j, y, w| {
                let (mean, variance, logp) =
                    terms(state.scores[i] - state.scores[j], config.tie_strength, y);
                objective += w * logp;
                let delta = w * (y - mean);
                gradient[i] += delta;
                gradient[j] -= delta;
                curvature[i] += 2.0 * w * variance;
                curvature[j] += 2.0 * w * variance;
            });
            Ok(())
        })?;
        let mut directional = 0.0;
        for (g, c) in gradient.iter_mut().zip(&curvature) {
            let direction = 0.9 * *g / c;
            directional += *g * direction;
            *g = direction;
        }
        // Diagonal Newton preconditioning with an Armijo check on the entire
        // composite objective. Each trial streams evidence with no open SQL transaction.
        let mut step = 1.0;
        let mut accepted = false;
        for _ in 0..12 {
            let mut proposed = -0.5
                * config.regularization
                * state
                    .scores
                    .iter()
                    .zip(&gradient)
                    .map(|(s, d)| (s + step * d).powi(2))
                    .sum::<f64>();
            visit(source, input, split, check, &mut |row| {
                pairs(row, |i, j, y, w| {
                    proposed += w * terms(
                        state.scores[i] + step * gradient[i] - state.scores[j] - step * gradient[j],
                        config.tie_strength,
                        y,
                    )
                    .2;
                });
                Ok(())
            })?;
            if proposed >= objective + 1e-4 * step * directional - 1e-10 * (1.0 + objective.abs()) {
                accepted = true;
                break;
            }
            step *= 0.5;
        }
        if !accepted {
            return Err(Error::new("ESTIMATOR_FAILED", "估计器线搜索未收敛"));
        }
        let mut update: f64 = 0.0;
        for (s, d) in state.scores.iter_mut().zip(&gradient) {
            let delta = step * d;
            *s += delta;
        }
        // Component translations do not affect pair probabilities. Their optimum
        // under the zero-centered quadratic penalty is exactly zero mean; remove
        // this otherwise very slow common mode after every accepted step.
        curvature.fill(0.0);
        for (i, s) in state.scores.iter().enumerate() {
            curvature[state.parent[i] as usize] += s;
        }
        for (i, s) in state.scores.iter_mut().enumerate() {
            let root = state.parent[i] as usize;
            let mean = if state.size[root] > 0 {
                curvature[root] / f64::from(state.size[root])
            } else {
                0.0
            };
            *s -= mean;
            update = update.max((step * gradient[i] - mean).abs());
        }
        state.iterations = iteration + 1;
        state.max_update = update;
        if update < TOLERANCE {
            state.converged = true;
            break;
        }
    }
    if state.scores.iter().any(|v| !v.is_finite()) {
        return Err(Error::new("ESTIMATOR_FAILED", "估计器产生非有限值"));
    }
    visit(source, input, split, check, &mut |row| {
        let n = row.tiers.iter().map(Vec::len).sum::<usize>();
        pairs(row, |i, j, y, _| {
            let residual = (y - expected(state.scores[i] - state.scores[j], config.tie_strength))
                .abs()
                / (n - 1) as f64;
            state.signals[i].residual += residual;
            state.signals[j].residual += residual;
        });
        Ok(())
    })?;
    Ok(state)
}
fn ranks(state: &FitState, meta: &[Meta]) -> Vec<Rank> {
    let mut order: Vec<u32> = (0..meta.len() as u32).collect();
    let quantized = |i: usize| (state.scores[i] / SCORE_GRID).round() as i64;
    order.sort_unstable_by_key(|i| {
        let i = *i as usize;
        (meta[i].rating, state.parent[i], -quantized(i), i)
    });
    let mut result = vec![Rank::default(); meta.len()];
    let mut start = 0;
    while start < order.len() {
        let id = order[start] as usize;
        let component = state.parent[id];
        let mut end = start + 1;
        while end < order.len()
            && meta[order[end] as usize].rating == meta[id].rating
            && state.parent[order[end] as usize] == component
        {
            end += 1;
        }
        let mut cursor = start;
        while cursor < end {
            let mut next = cursor + 1;
            while next < end && quantized(order[next] as usize) == quantized(order[cursor] as usize)
            {
                next += 1;
            }
            for position in cursor..next {
                result[order[position] as usize] = Rank {
                    lo: (cursor - start + 1) as u32,
                    hi: (next - start) as u32,
                    percentile: if end - start > 1 {
                        ((cursor - start) as f64 + (next - start - 1) as f64)
                            / 2.0
                            / (end - start - 1) as f64
                    } else {
                        0.5
                    },
                    position: position as u32 + 1,
                };
            }
            cursor = next;
        }
        start = end;
    }
    result
}

pub fn replay(
    source: &dyn AestheticReplaySource,
    input: &AestheticAnalysisInput,
    config: &AestheticFit,
    check: &dyn Fn() -> Result<()>,
    progress: &mut dyn FnMut(&str, u64, u64) -> Result<()>,
    emit: &mut dyn FnMut(Vec<AestheticRankingRow>) -> Result<()>,
) -> Result<AestheticFitSummary> {
    validate_fit(config)?;
    if input.candidates == 0 || input.candidates > MAX_CANDIDATES {
        return Err(Error::invalid("单次离线估计支持 1–1000000 个冻结候选"));
    }
    let mut meta = Vec::with_capacity(input.candidates as usize);
    let mut after = None;
    loop {
        check()?;
        let page = source.replay_candidates(&input.stage_id, after)?;
        if page.is_empty() {
            break;
        }
        for row in page {
            if row.ordinal != meta.len() as u64 || row.ordinal >= input.candidates {
                return Err(Error::new("EVIDENCE_INVALID", "冻结候选序列不完整"));
            }
            after = Some(row.ordinal);
            meta.push(Meta {
                rating: rating(&row.rating),
                year: row.year,
            });
        }
        progress("loading", meta.len() as u64, input.candidates)?;
    }
    if meta.len() as u64 != input.candidates {
        return Err(Error::new("EVIDENCE_INVALID", "冻结候选数不一致"));
    }
    let mut state = fit(
        source,
        input,
        &config.estimator,
        &meta,
        None,
        check,
        progress,
    )?;
    let rank = ranks(&state, &meta);
    let mut split_delta = vec![None; meta.len()];
    if let Some(seed) = config.stability_seed {
        let mut first = vec![None; meta.len()];
        for half in 0..2 {
            progress("stability", half, 2)?;
            let split = fit(
                source,
                input,
                &config.estimator,
                &meta,
                Some((seed, half)),
                check,
                progress,
            )?;
            let sr = ranks(&split, &meta);
            for i in 0..meta.len() {
                let root = state.parent[i] as usize;
                let connected = state.converged
                    && split.converged
                    && state.size[root] > 1
                    && split.parent[i] == state.parent[i]
                    && split.size[root] == state.size[root];
                if half == 0 && connected {
                    first[i] = Some(sr[i].percentile);
                }
                if half == 1 && connected {
                    split_delta[i] = first[i].map(|p: f64| (p - sr[i].percentile).abs());
                }
            }
        }
    }
    after = None;
    loop {
        check()?;
        let page = source.replay_candidates(&input.stage_id, after)?;
        if page.is_empty() {
            break;
        }
        let mut rows = Vec::with_capacity(page.len());
        for c in page {
            let i = c.ordinal as usize;
            let s = &state.signals[i];
            let r = rank[i];
            let root = state.parent[i] as usize;
            let size = state.size[root];
            let compared = state.mass[i] > 0.0;
            let group = state.groups.get_mut(&meta[i].rating).expect("group");
            group.split_comparable += u64::from(split_delta[i].is_some());
            let bins = s.opponents.count_ones();
            let disagreement = (config.estimator.kind == "davidson_v1" && s.batches > 0)
                .then(|| s.residual / f64::from(s.batches));
            rows.push(AestheticRankingRow {
                position: u64::from(r.position),
                ordinal: c.ordinal,
                key: c.key,
                rating: c.rating,
                year: c.year,
                content_version: c.content_version,
                score: compared.then_some(state.scores[i]),
                component: compared.then_some(root as u64),
                component_size: u64::from(size),
                rank_min: compared.then_some(u64::from(r.lo)),
                rank_max: compared.then_some(u64::from(r.hi)),
                rating_rank_min: group.fully_connected.then_some(u64::from(r.lo)),
                rating_rank_max: group.fully_connected.then_some(u64::from(r.hi)),
                percentile: group.fully_connected.then_some(r.percentile),
                component_percentile: compared.then_some(r.percentile),
                exposures: s.exposures,
                unjudgeable: s.unjudgeable,
                protected: s.protected,
                opponent_bins: bins,
                opponent_diversity_estimate: if bins < 64 {
                    Some(-64.0 * (1.0 - f64::from(bins) / 64.0).ln())
                } else {
                    None
                },
                cross_year_exposures: s.cross_year,
                split_percentile_delta: split_delta[i],
                disagreement,
                needs_review: s.exposures < 4
                    || !group.fully_connected
                    || bins < 8
                    || split_delta[i].is_some_and(|v| v >= 0.2)
                    || disagreement.is_some_and(|v| v >= 0.4),
            });
            after = Some(c.ordinal);
        }
        progress("publishing", after.unwrap_or(0) + 1, input.candidates)?;
        emit(rows)?;
    }
    Ok(AestheticFitSummary {
        estimator_version: config.estimator.kind.clone(),
        groups: state.groups.into_values().collect(),
        iterations_completed: state.iterations,
        converged: state.converged,
        max_update: state.max_update,
        working_bytes_estimate: working_bytes(input.candidates),
        stability_method: if config.stability_seed.is_some() {
            "whole_batch_disjoint_halves_v1"
        } else {
            "disabled"
        }
        .into(),
    })
}
