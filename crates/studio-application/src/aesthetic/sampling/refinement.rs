//! Versioned neighbor refinement and budget allocation. Sensitivity is not a CI.
use super::*;
pub(super) fn enabled(policy: &AestheticSamplingPolicy) -> bool {
    matches!(policy.mode.as_str(), "refine" | "refine_balanced")
}
pub(super) fn estimator(
    observations: &[AestheticReplayObservation],
    count: usize,
) -> AestheticEstimator {
    let exposures: usize = observations
        .iter()
        .map(|b| b.tiers.iter().map(Vec::len).sum::<usize>())
        .filter(|n| *n >= 2)
        .sum();
    let mean = exposures as f64 / count.max(1) as f64;
    AestheticEstimator {
        kind: "davidson_v2".into(),
        iterations: 128,
        regularization: (0.02 / 4f64.powf((mean - 2.0).max(0.0))).max(0.001),
        tie_strength: 0.1,
    }
}
pub(super) fn prioritize(
    targets: &mut BTreeSet<usize>,
    rows: &[AestheticRankingRow],
    sensitivity: &[f64],
    salt: u64,
) {
    let mut priority: Vec<_> = targets.iter().copied().collect();
    priority.sort_by(|a, b| {
        let value = |i: usize| sensitivity[i] / (f64::from(rows[i].exposures) + 1.0).sqrt();
        value(*b)
            .total_cmp(&value(*a))
            .then(estimator::mix(*a as u64 ^ salt).cmp(&estimator::mix(*b as u64 ^ salt)))
    });
    let take = priority.len() * 3 / 4;
    *targets = priority
        .into_iter()
        .enumerate()
        .filter(|(p, i)| *p < take || estimator::mix(*i as u64 ^ salt ^ 0x39).is_multiple_of(8))
        .map(|(_, i)| i)
        .collect();
}
pub(super) fn batches(
    rows: &[AestheticRankingRow],
    targets: &BTreeSet<usize>,
    available: &BTreeSet<u64>,
    reasons: &[String],
    salt: u64,
    limits: (u64, u32),
    check: &dyn Fn() -> Result<()>,
) -> Result<Vec<Vec<AestheticSamplingMemberReason>>> {
    let (cap, max) = limits;
    let locals: usize = 16;
    let fraction: f64 = 0.002;
    assert!((4..=16).contains(&locals));
    let mut batches: Vec<Vec<AestheticSamplingMemberReason>> = vec![];
    for rating in ["g", "s", "q", "e"] {
        check()?;
        let mut ids: Vec<_> = targets
            .iter()
            .copied()
            .filter(|i| rows[*i].rating == rating)
            .collect();
        let mut anchors: Vec<_> = available
            .iter()
            .map(|i| *i as usize)
            .filter(|i| {
                rows[*i].rating == rating && !targets.contains(i) && rows[*i].exposures < max
            })
            .collect();
        anchors.sort_by_key(|i| estimator::mix(*i as u64 ^ salt));
        let extra = (16 - ids.len() % 16) % 16;
        ids.extend(anchors.into_iter().take(extra));
        ids.sort_by_key(|i| estimator::mix(*i as u64 ^ salt ^ 0x3372));
        let global_count = ids.len() * (16 - locals) / 16;
        let global: Vec<_> = ids.drain(..global_count).collect();
        let window = ((ids.len() as f64 * fraction) as usize).max(16);
        ids.sort_by(|a, b| {
            rows[*a]
                .percentile
                .unwrap()
                .total_cmp(&rows[*b].percentile.unwrap())
                .then(a.cmp(b))
        });
        let offset = estimator::mix(salt ^ 0x77) as usize % window;
        ids.sort_by_key(|i| {
            let bucket = (rows[*i].percentile.unwrap() / fraction + offset as f64 / window as f64)
                .floor() as u64;
            (bucket, estimator::mix(*i as u64 ^ salt ^ 0x2121))
        });
        let mut global = global.into_iter();
        for chunk in ids.chunks(locals) {
            let mut picked = chunk.to_vec();
            picked.extend(global.by_ref().take(16 - picked.len()));
            if picked.len() < 8
                && let Some(last) = batches.last_mut()
                && rows[last[0].ordinal as usize].rating == rating
            {
                while last.len() > picked.len() + 1 && last.len() > 2 {
                    picked.push(last.pop().unwrap().ordinal as usize);
                }
            }
            if picked.len() < 2 {
                continue;
            }
            picked.sort_by_key(|i| estimator::mix(*i as u64 ^ salt ^ 0x81ae));
            batches.push(
                picked
                    .into_iter()
                    .map(|i| AestheticSamplingMemberReason {
                        ordinal: i as u64,
                        reason: if targets.contains(&i) {
                            reasons[i].clone()
                        } else {
                            "anchor".into()
                        },
                    })
                    .collect(),
            );
        }
        assert!(global.next().is_none());
    }
    batches.sort_by_key(|b| {
        (
            b.iter()
                .map(|m| rows[m.ordinal as usize].exposures)
                .min()
                .unwrap(),
            estimator::mix(b[0].ordinal ^ salt ^ 0x139),
        )
    });
    batches.truncate(cap as usize);
    Ok(batches)
}
// Whole-batch, diagonal leave-one-batch influence proxy. This ignores cross-item
// covariance and is NOT a confidence interval. Calibration is measured separately.
pub(super) fn sensitivity(
    rows: &[AestheticRankingRow],
    observations: &[AestheticReplayObservation],
    reg: f64,
    ties: f64,
    check: &dyn Fn() -> Result<()>,
) -> Result<Vec<f64>> {
    let n = rows.len();
    let mut indexed = vec![None; n];
    for row in rows {
        indexed[row.ordinal as usize] = Some(row);
    }
    let mut count = vec![0u32; n];
    let mut sum = vec![0.0; n];
    let mut square = vec![0.0; n];
    let mut curvature = vec![reg; n];
    for batch in observations {
        check()?;
        let ids: Vec<_> = batch
            .tiers
            .iter()
            .enumerate()
            .flat_map(|(tier, ids)| ids.iter().map(move |i| (*i as usize, tier)))
            .collect();
        if ids.len() < 2 {
            continue;
        }
        let w = 2.0 / (ids.len() * (ids.len() - 1)) as f64;
        let mut g = [0.0; 16];
        for a in 0..ids.len() {
            for b in a + 1..ids.len() {
                let (i, ti) = ids[a];
                let (j, tj) = ids[b];
                let y = if ti == tj { 0.5 } else { 1.0 };
                let d = (indexed[i].unwrap().score.unwrap_or(0.0)
                    - indexed[j].unwrap().score.unwrap_or(0.0))
                    * 0.5;
                let mx = d.abs().max(ties.ln());
                let win = (d - mx).exp();
                let loss = (-d - mx).exp();
                let draw = (ties.ln() - mx).exp();
                let z = win + loss + draw;
                let mean = (win + 0.5 * draw) / z;
                let variance = ((win + 0.25 * draw) / z - mean * mean).max(0.0);
                g[a] += w * (y - mean);
                g[b] -= w * (y - mean);
                curvature[i] += w * variance;
                curvature[j] += w * variance;
            }
        }
        for (a, (i, _)) in ids.iter().enumerate() {
            count[*i] += 1;
            sum[*i] += g[a];
            square[*i] += g[a] * g[a];
        }
    }
    let mut groups: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    for row in rows {
        if let Some(score) = row.score {
            groups.entry(&row.rating).or_default().push(score);
        }
    }
    for scores in groups.values_mut() {
        scores.sort_by(f64::total_cmp);
    }
    let mut result = vec![1.0; n];
    for row in rows {
        let i = row.ordinal as usize;
        if count[i] < 4 || row.percentile.is_none() {
            continue;
        }
        let m = f64::from(count[i]);
        let score = row.score.unwrap();
        let variance = (m / (m - 1.0) * (square[i] - sum[i] * sum[i] / m)).max(0.0);
        let sensitivity = (2.0 * variance.sqrt() + reg * score.abs()) / curvature[i];
        let scores = &groups[row.rating.as_str()];
        let denominator = scores.len().saturating_sub(1).max(1) as f64;
        let lo = scores.partition_point(|s| *s < score - sensitivity) as f64 / denominator;
        let hi = scores
            .partition_point(|s| *s <= score + sensitivity)
            .saturating_sub(1) as f64
            / denominator;
        let center = 1.0 - row.percentile.unwrap();
        result[i] = (center - lo).abs().max((hi - center).abs()).min(1.0);
    }
    Ok(result)
}
