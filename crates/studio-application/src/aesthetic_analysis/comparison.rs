use super::*;
use std::collections::BTreeMap;

#[derive(Default)]
struct Accumulator {
    matched: u64,
    n: u64,
    mx: f64,
    my: f64,
    xx: f64,
    yy: f64,
    xy: f64,
    delta: f64,
    middle_delta: f64,
    middle: u64,
    intersection: u64,
    union: u64,
    elite: u64,
}
pub struct Comparison {
    left: BTreeMap<String, AestheticRatingSummary>,
    right: BTreeMap<String, AestheticRatingSummary>,
    totals: BTreeMap<String, Accumulator>,
}
impl Comparison {
    pub fn new(left: &AestheticFitSummary, right: &AestheticFitSummary) -> Self {
        Self {
            left: left
                .groups
                .iter()
                .map(|g| (g.rating.clone(), g.clone()))
                .collect(),
            right: right
                .groups
                .iter()
                .map(|g| (g.rating.clone(), g.clone()))
                .collect(),
            totals: Default::default(),
        }
    }
    pub fn push(&mut self, left: &AestheticRankingRow, right: Option<&AestheticRankingRow>) {
        let s = self.totals.entry(left.rating.clone()).or_default();
        let Some(right) =
            right.filter(|r| r.rating == left.rating && r.content_version == left.content_version)
        else {
            return;
        };
        s.matched += 1;
        s.elite += u64::from(left.protected != right.protected);
        if let Some((x, y)) = left.percentile.zip(right.percentile) {
            s.n += 1;
            let dx = x - s.mx;
            let dy = y - s.my;
            let n = s.n as f64;
            s.mx += dx / n;
            s.my += dy / n;
            s.xx += dx * (x - s.mx);
            s.yy += dy * (y - s.my);
            s.xy += dx * (y - s.my);
            s.delta += (x - y).abs();
            if (0.2..=0.8).contains(&x) {
                s.middle += 1;
                s.middle_delta += (x - y).abs();
            }
            let a = left
                .rank_min
                .is_some_and(|r| r <= ((left.component_size as f64 * 0.2).ceil() as u64).max(1));
            let b = right
                .rank_min
                .is_some_and(|r| r <= ((right.component_size as f64 * 0.2).ceil() as u64).max(1));
            s.intersection += u64::from(a && b);
            s.union += u64::from(a || b);
        }
    }
    pub fn summary(&self) -> Vec<AestheticComparisonGroup> {
        let mut ratings = std::collections::BTreeSet::new();
        ratings.extend(self.left.keys());
        ratings.extend(self.right.keys());
        ratings
            .into_iter()
            .map(|rating| {
                let default = Accumulator::default();
                let s = self.totals.get(rating).unwrap_or(&default);
                let a = self.left.get(rating);
                let b = self.right.get(rating);
                let same = a.zip(b).is_some_and(|(a, b)| {
                    a.candidates == b.candidates && a.candidates == s.matched
                });
                let connected = a
                    .zip(b)
                    .is_some_and(|(a, b)| a.fully_connected && b.fully_connected);
                let comparable = same && connected && s.n > 1;
                AestheticComparisonGroup {
                    rating: rating.clone(),
                    matched: s.matched,
                    comparable,
                    rank_correlation: if comparable && s.xx > 1e-15 && s.yy > 1e-15 {
                        Some((s.xy / (s.xx * s.yy).sqrt()).clamp(-1.0, 1.0))
                    } else {
                        None
                    },
                    mean_absolute_percentile_delta: comparable.then(|| s.delta / s.n as f64),
                    middle_mean_absolute_percentile_delta: (comparable && s.middle > 0)
                        .then(|| s.middle_delta / s.middle as f64),
                    top20_jaccard: (comparable && s.union > 0)
                        .then(|| s.intersection as f64 / s.union as f64),
                    elite_disagreements: s.elite,
                    reason: if !same {
                        Some("候选身份、Rating 或内容版本集合不同".into())
                    } else if !connected {
                        Some("Rating 比较图未覆盖全部候选且连通".into())
                    } else if s.n < 2 {
                        Some("可比较候选不足".into())
                    } else {
                        None
                    },
                }
            })
            .collect()
    }
}
