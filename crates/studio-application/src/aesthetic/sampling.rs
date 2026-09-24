//! Round-based scheduling over frozen evidence. Empirical movement is not a CI.
use super::*;
use crate::aesthetic_analysis::{AestheticReplaySource, estimator};
use std::collections::{BTreeMap, BTreeSet};
use studio_domain::aesthetic_analysis::*;

mod refinement;
mod target_components;
use target_components::TargetComponents;
#[cfg(test)]
mod tests;
pub const VERSION: &str = "connected_rounds_v1";
pub const REFINEMENT_VERSION: &str = "neighbor_budget_v2";
pub fn version(policy: &AestheticSamplingPolicy) -> &'static str {
    if refinement::enabled(policy) {
        REFINEMENT_VERSION
    } else {
        VERSION
    }
}
pub fn balanced(policy: &AestheticSamplingPolicy) -> bool {
    matches!(policy.mode.as_str(), "balanced" | "refine_balanced")
}
pub fn validate_version(status: &AestheticSamplingStatus) -> Result<()> {
    if status.version != version(&status.policy) {
        return Err(Error::new(
            "EVALUATION_CONFIG_UNSUPPORTED",
            "采样策略版本不受支持",
        ));
    }
    Ok(())
}
// Candidate admission is shared with paid creation and offline analysis.
// Replay retains a separate finite bound; this does not authorize extra calls.
pub const MAX_CANDIDATES: u64 = AESTHETIC_MAX_CANDIDATES;
pub const MAX_OBSERVATIONS: u64 = 20_000_000;

pub fn validate(policy: &AestheticSamplingPolicy, total: u64) -> Result<()> {
    if !matches!(
        policy.mode.as_str(),
        "balanced" | "adaptive" | "refine" | "refine_balanced"
    ) || !(1..=32).contains(&policy.min_exposures)
        || !(policy.min_exposures..=32).contains(&policy.max_exposures)
        || !policy.rank_tolerance.is_finite()
        || !(0.01..=0.25).contains(&policy.rank_tolerance)
    {
        return Err(Error::invalid(
            "采样模式须为 balanced/adaptive/refine/refine_balanced；曝光 1–32，最大值不小于最低值；位次变化阈值 0.01–0.25",
        ));
    }
    if total > MAX_CANDIDATES {
        return Err(Error::new(
            "EVALUATION_SAMPLING_CAPACITY",
            "按轮次采样最多 10000000 图；尚未派发",
        ));
    }
    Ok(())
}

pub struct Round {
    pub status: AestheticSamplingStatus,
    pub diagnostics: Vec<AestheticSamplingDiagnostic>,
    pub batches: Vec<Vec<AestheticSamplingMemberReason>>,
}

struct Replay {
    candidates: Vec<AestheticCandidate>,
    observations: Vec<AestheticReplayObservation>,
}
impl AestheticReplaySource for Replay {
    fn replay_candidates(&self, _: &str, after: Option<u64>) -> Result<Vec<AestheticCandidate>> {
        let start = after.map_or(0, |n| n as usize + 1);
        Ok(self
            .candidates
            .iter()
            .skip(start)
            .take(64)
            .cloned()
            .collect())
    }
    fn replay_observations(
        &self,
        _: &AestheticAnalysisInput,
        after: u64,
    ) -> Result<Vec<AestheticReplayObservation>> {
        let start = self.observations.partition_point(|r| r.batch <= after);
        Ok(self
            .observations
            .iter()
            .skip(start)
            .take(64)
            .cloned()
            .collect())
    }
}

fn root(parents: &mut [usize], mut i: usize) -> usize {
    while parents[i] != i {
        parents[i] = parents[parents[i]];
        i = parents[i];
    }
    i
}
fn union(parents: &mut [usize], sizes: &mut [usize], a: usize, b: usize) {
    let mut a = root(parents, a);
    let mut b = root(parents, b);
    if a == b {
        return;
    }
    if sizes[a] < sizes[b] {
        std::mem::swap(&mut a, &mut b);
    }
    parents[b] = a;
    sizes[a] += sizes[b];
}

/// All choices use one accepted-evidence watermark. Network completion order
/// cannot change an already-published round. At most one appearance per round.
pub fn plan(
    source: &dyn AestheticReplaySource,
    input: &AestheticAnalysisInput,
    mut status: AestheticSamplingStatus,
    previous: &[AestheticSamplingDiagnostic],
    available: &BTreeSet<u64>,
    remaining_calls: u64,
    check: &dyn Fn() -> Result<()>,
) -> Result<Round> {
    validate_version(&status)?;
    validate(&status.policy, input.candidates)?;
    let refined = refinement::enabled(&status.policy);
    if input.observations > MAX_OBSERVATIONS {
        return Err(Error::new(
            "EVALUATION_SAMPLING_CAPACITY",
            "按轮次采样重放上限为 20000000 批有效证据",
        ));
    }
    let mut replay = Replay {
        candidates: Vec::new(),
        observations: Vec::new(),
    };
    let mut after = None;
    loop {
        check()?;
        let page = source.replay_candidates(&input.stage_id, after)?;
        if page.is_empty() {
            break;
        }
        after = page.last().map(|r| r.ordinal);
        for row in page {
            if row.ordinal != replay.candidates.len() as u64 || row.ordinal >= input.candidates {
                return Err(Error::new("EVIDENCE_INVALID", "采样候选序列不完整"));
            }
            replay.candidates.push(row);
        }
    }
    if replay.candidates.len() as u64 != input.candidates {
        return Err(Error::new("EVIDENCE_INVALID", "采样候选总数不一致"));
    }
    let mut cursor = 0;
    loop {
        check()?;
        let page = source.replay_observations(input, cursor)?;
        if page.is_empty() {
            break;
        }
        for row in page {
            if row.batch <= cursor || replay.observations.len() as u64 >= MAX_OBSERVATIONS {
                return Err(Error::new("EVIDENCE_INVALID", "采样证据游标或数量无效"));
            }
            cursor = row.batch;
            replay.observations.push(row);
        }
    }
    if replay.observations.len() as u64 != input.observations {
        return Err(Error::new(
            "EVIDENCE_INVALID",
            "采样证据数量与冻结水位不一致",
        ));
    }
    let fit = AestheticFit {
        stage_id: input.stage_id.clone(),
        estimator: if refined {
            refinement::estimator(&replay.observations, replay.candidates.len())
        } else {
            AestheticEstimator {
                kind: "davidson_v1".into(),
                iterations: 128,
                regularization: 0.1,
                tie_strength: 1.0,
            }
        },
        stability_seed: None,
    };
    let mut rows = Vec::new();
    let fitted = estimator::replay(
        &replay,
        input,
        &fit,
        check,
        &mut |_, _, _| Ok(()),
        &mut |page| {
            rows.extend(page);
            Ok(())
        },
    )?;
    rows.sort_by_key(|r| r.ordinal);
    let n = rows.len();
    let mut counts = BTreeMap::new();
    for &i in available {
        *counts
            .entry(rows[i as usize].rating.clone())
            .or_insert(0usize) += 1;
    }
    let no_peers: BTreeSet<_> = available
        .iter()
        .copied()
        .filter(|i| counts[&rows[*i as usize].rating] < 2)
        .collect();
    let eligible: BTreeSet<_> = available.difference(&no_peers).copied().collect();
    let available = &eligible;

    let mut opponents = vec![BTreeSet::new(); n];
    let mut last_batch = vec![0; n];
    let mut parents: Vec<_> = (0..n).collect();
    let mut sizes = vec![1; n];
    for batch in &replay.observations {
        check()?;
        let ids: Vec<_> = batch.tiers.iter().flatten().map(|v| *v as usize).collect();
        if ids.len() < 2 {
            continue;
        }
        for &i in &ids {
            last_batch[i] = batch.batch;
            for &j in &ids {
                if i != j && opponents[i].len() < 32 {
                    opponents[i].insert(j);
                }
            }
            // Excluded/blocked candidates do not serve as the sole bridge.
            if available.contains(&(i as u64)) {
                for &j in &ids {
                    if available.contains(&(j as u64)) {
                        union(&mut parents, &mut sizes, i, j);
                    }
                }
            }
        }
    }
    let old: BTreeMap<_, _> = previous.iter().map(|v| (v.ordinal, v)).collect();
    let mut rating_counts = BTreeMap::new();
    let mut component_counts: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    for &i in available {
        *rating_counts
            .entry(rows[i as usize].rating.clone())
            .or_insert(0usize) += 1;
        component_counts
            .entry(rows[i as usize].rating.clone())
            .or_default()
            .insert(root(&mut parents, i as usize));
    }
    let mut diagnostics = Vec::with_capacity(n);
    let mut targets = BTreeSet::new();
    let mut reasons = vec![String::new(); n];
    let next_round = status.round + 1;
    let salt =
        u64::from(status.policy.seed) ^ u64::from(next_round).wrapping_mul(0x9e3779b97f4a7c15);
    let sensitivity = if refined {
        refinement::sensitivity(
            &rows,
            &replay.observations,
            fit.estimator.regularization,
            fit.estimator.tie_strength,
            check,
        )?
    } else {
        vec![]
    };
    let mut covered = 0;
    let mut stable = 0;
    let mut unresolved_active = 0;
    for (i, row) in rows.iter().enumerate() {
        let active = available.contains(&row.ordinal);
        let rating_size = *rating_counts.get(&row.rating).unwrap_or(&0);
        let connected = rating_size > 1
            && component_counts
                .get(&row.rating)
                .is_some_and(|v| v.len() == 1);
        let previous = old.get(&row.ordinal);
        let delta = previous
            .and_then(|p| p.percentile.zip(row.percentile))
            .map(|(a, b)| (a - b).abs());
        let novel = opponents[i].len() >= rating_size.saturating_sub(1).min(24);
        let changed = previous.is_some_and(|p| row.exposures > p.exposures);
        let rounds = if connected
            && fitted.converged
            && novel
            && row.disagreement.is_none_or(|v| v < 0.4)
            && delta.is_some_and(|v| v <= status.policy.rank_tolerance)
        {
            previous.map_or(0, |p| p.stable_rounds + u32::from(changed))
        } else {
            0
        };
        let enough = row.exposures >= status.policy.min_exposures;
        let is_stable = enough && rounds >= 2 && !refined;
        if active && enough {
            covered += 1;
        }
        if active && is_stable {
            stable += 1;
        }
        let reason = if no_peers.contains(&row.ordinal) {
            "no_comparison_peer"
        } else if !active {
            "unavailable"
        } else if !enough || replay.candidates[i].disposition == AestheticDisposition::Rejudge {
            "coverage"
        } else if !connected {
            "bridge"
        } else if balanced(&status.policy) {
            "covered"
        } else if !novel {
            "opponent_diversity"
        } else if !is_stable {
            "uncertainty"
        } else {
            "stable"
        };
        let needs = matches!(
            reason,
            "coverage" | "bridge" | "opponent_diversity" | "uncertainty"
        );
        if active && needs {
            unresolved_active += 1;
        }
        let reason = if needs && row.exposures >= status.policy.max_exposures {
            "exposure_limit"
        } else {
            reason
        };
        if needs && row.exposures < status.policy.max_exposures {
            targets.insert(i);
        }
        reasons[i] = reason.into();
        diagnostics.push(AestheticSamplingDiagnostic {
            ordinal: row.ordinal,
            exposures: row.exposures,
            distinct_opponents: opponents[i].len() as u32,
            component: row.component,
            component_size: row.component_size,
            percentile: row.percentile,
            rank_delta: delta,
            rank_sensitivity: refined.then(|| sensitivity[i]),
            stable_rounds: rounds,
            reason: reason.into(),
        });
    }
    if refined
        && !balanced(&status.policy)
        && available
            .iter()
            .all(|i| rows[*i as usize].exposures >= status.policy.min_exposures.max(4))
        && component_counts.values().all(|c| c.len() == 1)
    {
        refinement::prioritize(&mut targets, &rows, &sensitivity, salt);
    }
    // When coverage is already sufficient, bridge with rotating representatives
    // from each disconnected component instead of repeating the whole population.
    let mut bridges: BTreeMap<(String, usize), Vec<usize>> = BTreeMap::new();
    for i in 0..n {
        if reasons[i] == "bridge" && targets.remove(&i) {
            bridges
                .entry((rows[i].rating.clone(), root(&mut parents, i)))
                .or_default()
                .push(i);
        }
    }
    for mut members in bridges.into_values() {
        members.sort_by(|a, b| {
            rows[*a]
                .component_percentile
                .unwrap_or(0.5)
                .total_cmp(&rows[*b].component_percentile.unwrap_or(0.5))
                .then_with(|| {
                    estimator::mix(*a as u64 ^ salt).cmp(&estimator::mix(*b as u64 ^ salt))
                })
        });
        for index in [0, members.len() / 2, members.len().saturating_sub(1)] {
            if let Some(i) = members.get(index) {
                targets.insert(*i);
            }
        }
    }
    let connected = component_counts
        .iter()
        .all(|(r, c)| c.len() == 1 && rating_counts[r] > 1);
    let unavailable = replay
        .candidates
        .iter()
        .filter(|c| {
            c.disposition != AestheticDisposition::Excluded && !available.contains(&c.ordinal)
        })
        .count() as u64;
    let unresolved = unresolved_active + unavailable;
    status.evidence_watermark = input.evidence_watermark;
    status.eligible = available.len() as u64;
    status.covered = covered;
    status.stable = stable;
    status.components = component_counts.values().map(|v| v.len() as u64).sum();
    status.unresolved = unresolved;
    status.state = "planning".into();
    status.reason = None;
    if connected && unresolved == 0 {
        status.state = "satisfied".into();
        status.reason = Some(
            if available.is_empty() {
                "no_remaining_candidates"
            } else if balanced(&status.policy) {
                "coverage_connected"
            } else {
                "empirical_stability"
            }
            .into(),
        );
        return Ok(Round {
            status,
            diagnostics,
            batches: vec![],
        });
    }
    if !balanced(&status.policy)
        && connected
        && covered == available.len() as u64
        && available
            .iter()
            .any(|i| rows[*i as usize].percentile.is_none())
    {
        status.state = "limited".into();
        status.reason = Some("ranking_scope_incomplete".into());
        return Ok(Round {
            status,
            diagnostics,
            batches: vec![],
        });
    }
    let evidence_slots = MAX_OBSERVATIONS.saturating_sub(input.observations);
    let remaining_calls = remaining_calls.min(evidence_slots);
    if remaining_calls == 0 || targets.is_empty() {
        status.state = "limited".into();
        status.reason = Some(
            if evidence_slots == 0 {
                "evidence_limit"
            } else if remaining_calls == 0 {
                "call_budget"
            } else if unavailable > 0 {
                "unresolved_candidates"
            } else {
                "exposure_limit"
            }
            .into(),
        );
        return Ok(Round {
            status,
            diagnostics,
            batches: vec![],
        });
    }
    // Stable candidates remain eligible for rotating audits and as connecting
    // peers; quality itself is never a reason to delete a candidate or raise priority.
    if status.policy.mode == "adaptive" {
        for &i in available {
            let i = i as usize;
            if reasons[i] == "stable"
                && rows[i].exposures < status.policy.max_exposures
                && estimator::mix(i as u64 ^ salt).is_multiple_of(8)
            {
                targets.insert(i);
                reasons[i] = "exploration".into();
            }
        }
    }
    if refined
        && next_round > 2
        && connected
        && available
            .iter()
            .all(|i| rows[*i as usize].percentile.is_some())
    {
        let batches = refinement::batches(
            &rows,
            &targets,
            available,
            &reasons,
            salt,
            (remaining_calls, status.policy.max_exposures),
            check,
        )?;
        if batches.is_empty() {
            status.state = "limited".into();
            status.reason = Some("no_available_peer".into());
        } else {
            status.round = next_round;
            status.state = "dispatching".into();
        }
        return Ok(Round {
            status,
            diagnostics,
            batches,
        });
    }
    let mut batches = Vec::new();
    let mut used = BTreeSet::new();
    for rating in ["g", "s", "q", "e"] {
        let mut pending: Vec<_> = targets
            .iter()
            .copied()
            .filter(|i| rows[*i].rating == rating)
            .collect();
        pending.sort_by_key(|i| (rows[*i].exposures, estimator::mix(*i as u64 ^ salt)));
        let order = pending;
        let mut pending: BTreeSet<usize> = (0..order.len()).collect();
        let mut components = TargetComponents::new(&order, &mut parents, &sizes);
        let mut anchors: Vec<_> = available
            .iter()
            .map(|i| *i as usize)
            .filter(|i| {
                rows[*i].rating == rating
                    && !targets.contains(i)
                    && rows[*i].exposures < status.policy.max_exposures
            })
            .collect();
        anchors.sort_by_key(|i| estimator::mix(*i as u64 ^ salt));
        let mut anchors: std::collections::VecDeque<_> = anchors.into();
        while !pending.is_empty() && (batches.len() as u64) < remaining_calls {
            check()?;
            // Seed from the largest planned connected component that still has
            // unused targets. This grows bridges through different representatives.
            let largest = *components
                .sizes
                .last_key_value()
                .expect("remaining component")
                .0;
            let first = *pending
                .iter()
                .find(|&&p| sizes[root(&mut parents, order[p])] == largest)
                .expect("largest seed");
            pending.remove(&first);
            components.take(order[first], &mut parents, &sizes);
            let mut picked = vec![order[first]];
            while picked.len() < 16 && !pending.is_empty() {
                let seed = picked[0];
                let main = root(&mut parents, seed);
                let mut best = None;
                // Bounded candidate lookahead. All targets eventually receive a slot.
                for &p in pending.iter().take(256) {
                    let i = order[p];
                    let separate = root(&mut parents, i) != main;
                    let repeat = picked.iter().filter(|j| opponents[i].contains(j)).count();
                    let cohort = picked
                        .iter()
                        .filter(|&&j| last_batch[i] != 0 && last_batch[i] == last_batch[j])
                        .count();
                    let distance = rows[i]
                        .percentile
                        .zip(rows[seed].percentile)
                        .map_or(0, |(a, b)| ((a - b).abs() * 10000.0) as u32);
                    let key = (
                        !separate,
                        cohort,
                        repeat,
                        distance,
                        estimator::mix(i as u64 ^ salt),
                    );
                    if best.as_ref().is_none_or(|(_, old)| key < *old) {
                        best = Some((p, key));
                    }
                }
                let p = best.expect("nonempty").0;
                pending.remove(&p);
                let i = order[p];
                components.take(i, &mut parents, &sizes);
                components.merge(picked[0], i, &mut parents, &mut sizes);
                picked.push(i);
            }
            // Use rotating peers when an adaptive wave has spare slots. Never
            // exceed the per-image ceiling, duplicate an image, or cross Rating.
            while picked.len() < 16 && !anchors.is_empty() {
                let i = anchors.pop_front().expect("nonempty anchors");
                if used.contains(&i) {
                    continue;
                }
                reasons[i] = "anchor".into();
                picked.push(i);
            }
            if picked.len() < 8 {
                // Balance the tail without extra exposure or a tiny isolated batch.
                if let Some(last) =
                    batches
                        .last_mut()
                        .filter(|b: &&mut Vec<AestheticSamplingMemberReason>| {
                            b.len() > 2 && rows[b[0].ordinal as usize].rating == rating
                        })
                {
                    while last.len() > picked.len() + 1 {
                        picked.push(last.pop().expect("nonempty").ordinal as usize);
                    }
                }
            }
            if picked.len() < 2 {
                break;
            }
            picked.sort_by_key(|i| estimator::mix(*i as u64 ^ salt ^ 0x81ae));
            for i in &picked {
                used.insert(*i);
            }
            batches.push(
                picked
                    .into_iter()
                    .map(|i| AestheticSamplingMemberReason {
                        ordinal: i as u64,
                        reason: reasons[i].clone(),
                    })
                    .collect(),
            );
        }
    }
    if batches.is_empty() {
        status.state = "limited".into();
        status.reason = Some("no_available_peer".into());
    } else {
        status.round = next_round;
        status.state = "dispatching".into();
    }
    Ok(Round {
        status,
        diagnostics,
        batches,
    })
}
