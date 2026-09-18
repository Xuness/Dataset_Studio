use super::*;
use studio_domain::{AssetKey, aesthetic::AestheticCandidate};

struct Fixture {
    candidates: Vec<AestheticCandidate>,
    observations: Vec<AestheticReplayObservation>,
}
impl AestheticReplaySource for Fixture {
    fn replay_candidates(&self, _: &str, after: Option<u64>) -> Result<Vec<AestheticCandidate>> {
        Ok(self
            .candidates
            .iter()
            .filter(|r| after.is_none_or(|a| r.ordinal > a))
            .take(64)
            .cloned()
            .collect())
    }
    fn replay_observations(
        &self,
        input: &AestheticAnalysisInput,
        after: u64,
    ) -> Result<Vec<AestheticReplayObservation>> {
        Ok(self
            .observations
            .iter()
            .filter(|r| r.batch > after && r.batch <= input.evidence_watermark)
            .take(64)
            .cloned()
            .collect())
    }
}
fn fixture() -> Fixture {
    let candidates = (0..8)
        .map(|ordinal| AestheticCandidate {
            ordinal,
            key: AssetKey {
                source_id: "fixture".into(),
                asset_id: ordinal.to_string(),
            },
            rating: if ordinal < 6 { "g" } else { "s" }.into(),
            year: Some(2000 + ordinal as i32),
            basis: "fixture".into(),
            content_version: "frozen".into(),
            bytes: 0,
            exposures: 999,
            protected: true,
        })
        .collect();
    let mut observations = Vec::new();
    for _ in 0..30 {
        for (rating, tiers, elite, unjudgeable) in [
            ("g", vec![vec![0, 1], vec![2]], vec![2], vec![5]),
            ("g", vec![vec![3], vec![4]], vec![], vec![]),
            ("s", vec![vec![7], vec![6]], vec![], vec![]),
        ] {
            observations.push(AestheticReplayObservation {
                batch: observations.len() as u64 + 1,
                rating: rating.into(),
                tiers,
                elite,
                unjudgeable,
            });
        }
    }
    Fixture {
        candidates,
        observations,
    }
}
fn config(kind: &str) -> AestheticFit {
    AestheticFit {
        stage_id: studio_domain::new_id(),
        estimator: AestheticEstimator {
            kind: kind.into(),
            iterations: 128,
            regularization: 0.05,
            tie_strength: 1.0,
        },
        stability_seed: Some(17),
    }
}
fn run(f: &Fixture, config: &AestheticFit) -> (AestheticFitSummary, Vec<AestheticRankingRow>) {
    let mut rows = Vec::new();
    let input = AestheticAnalysisInput {
        stage_id: config.stage_id.clone(),
        stage_config_hash: "fixture".into(),
        candidates: f.candidates.len() as u64,
        evidence_watermark: f.observations.len() as u64,
        observations: f.observations.len() as u64,
        review_watermark: 0,
    };
    let summary = replay(
        f,
        &input,
        config,
        &|| Ok(()),
        &mut |_, _, _| Ok(()),
        &mut |page| {
            rows.extend(page);
            Ok(())
        },
    )
    .unwrap();
    rows.sort_by_key(|r| r.ordinal);
    (summary, rows)
}
#[test]
fn ties_components_rating_isolation_and_missing_evidence_have_explicit_meaning() {
    let (summary, rows) = run(&fixture(), &config("davidson_v1"));
    assert!(summary.converged, "{summary:?}");
    assert!((rows[0].score.unwrap() - rows[1].score.unwrap()).abs() < 1e-9);
    assert!(rows[1].score > rows[2].score);
    assert_eq!((rows[0].rank_min, rows[0].rank_max), (Some(1), Some(2)));
    assert!(rows[..6].iter().all(|r| r.rating_rank_min.is_none()));
    assert!(rows[5].score.is_none());
    assert_eq!(rows[5].unjudgeable, 30);
    assert_eq!(rows[5].exposures, 0);
    assert_eq!(rows[7].rating_rank_min, Some(1));
    assert_eq!(rows[6].rating_rank_min, Some(2));
    assert!(rows[2].protected);
    assert!(!rows[0].protected);
    assert_eq!(rows[0].exposures, 30);
    assert_eq!(rows[0].split_percentile_delta, Some(0.0));
    assert!(rows[0].cross_year_exposures > 0);
    let filter = AestheticRankingFilter {
        component: Some(0),
        top_percent: Some(1.0),
        ..Default::default()
    };
    assert!(matches_filter(&rows[0], &filter, false));
    assert!(matches_filter(&rows[1], &filter, false));
    assert!(!matches_filter(&rows[2], &filter, false));
    let filter = AestheticRankingFilter {
        ratings: vec!["g".into()],
        top_percent: Some(20.0),
        ..Default::default()
    };
    assert!(!rows.iter().any(|r| matches_filter(r, &filter, false)));
}
#[test]
fn elite_nominations_do_not_change_scores_and_borda_is_available_for_ablation() {
    let mut f = fixture();
    let c = config("davidson_v1");
    let (_, a) = run(&f, &c);
    for o in &mut f.observations {
        o.elite.clear();
    }
    let (_, b) = run(&f, &c);
    assert_eq!(
        a.iter().map(|r| r.score).collect::<Vec<_>>(),
        b.iter().map(|r| r.score).collect::<Vec<_>>()
    );
    assert!(b.iter().all(|r| !r.protected));
    let (_, borda) = run(&f, &config("borda_v1"));
    assert!(borda[0].score > borda[2].score);
    assert!(borda.iter().all(|r| r.disagreement.is_none()));
}
#[test]
fn a_single_batch_never_reports_split_stability_and_cancellation_stops_replay() {
    let mut f = fixture();
    f.observations.truncate(1);
    let (_, rows) = run(&f, &config("davidson_v1"));
    assert!(rows.iter().all(|r| r.split_percentile_delta.is_none()));
    let input = AestheticAnalysisInput {
        stage_id: studio_domain::new_id(),
        stage_config_hash: "fixture".into(),
        candidates: 8,
        evidence_watermark: 1,
        observations: 1,
        review_watermark: 0,
    };
    let error = replay(
        &f,
        &input,
        &config("davidson_v1"),
        &|| Err(Error::new("CANCELLED", "test")),
        &mut |_, _, _| Ok(()),
        &mut |_| panic!("must not publish"),
    )
    .unwrap_err();
    assert_eq!(error.code, "CANCELLED");
}
#[test]
fn davidson_gradient_matches_three_outcome_log_likelihood() {
    let eps = 1e-6;
    for y in [0.0, 0.5, 1.0] {
        for d in [-5.0, -0.3, 0.0, 1.7, 8.0] {
            for nu in [0.1, 1.0, 9.0] {
                let logp = |d: f64| {
                    let z = d.exp() + 1.0 + nu * (d * 0.5).exp();
                    let numerator = if y == 1.0 {
                        d.exp()
                    } else if y == 0.0 {
                        1.0
                    } else {
                        nu * (d * 0.5).exp()
                    };
                    (numerator / z).ln()
                };
                let numerical = (logp(d + eps) - logp(d - eps)) / (2.0 * eps);
                assert!((numerical - (y - expected(d, nu))).abs() < 1e-8);
            }
        }
    }
}
#[test]
fn comparisons_require_identical_frozen_population_and_connected_rating() {
    let (summary, rows) = run(&fixture(), &config("davidson_v1"));
    let mut comparison = crate::aesthetic_analysis::comparison::Comparison::new(&summary, &summary);
    for row in &rows {
        comparison.push(row, Some(row));
    }
    let groups = comparison.summary();
    let s = groups.iter().find(|g| g.rating == "s").unwrap();
    assert!(s.comparable);
    assert_eq!(s.rank_correlation, Some(1.0));
    assert_eq!(s.mean_absolute_percentile_delta, Some(0.0));
    assert!(!groups.iter().find(|g| g.rating == "g").unwrap().comparable);
    let mut comparison = crate::aesthetic_analysis::comparison::Comparison::new(&summary, &summary);
    for row in &rows {
        let mut other = row.clone();
        other.content_version = "changed".into();
        comparison.push(row, Some(&other));
    }
    assert!(
        comparison
            .summary()
            .iter()
            .all(|g| !g.comparable && g.rank_correlation.is_none())
    );
}
