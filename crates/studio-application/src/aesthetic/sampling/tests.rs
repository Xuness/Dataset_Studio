use super::*;
use studio_domain::AssetKey;

fn data(n: usize, ratings: bool) -> Replay {
    Replay {
        candidates: (0..n)
            .map(|i| AestheticCandidate {
                ordinal: i as u64,
                key: AssetKey {
                    source_id: "fixture".into(),
                    asset_id: i.to_string(),
                },
                rating: if ratings && i % 2 == 1 { "s" } else { "g" }.into(),
                year: Some(2025),
                basis: "fixture".into(),
                content_version: "1".into(),
                bytes: 0,
                exposures: 0,
                protected: false,
                disposition: Default::default(),
                disposition_reason: None,
            })
            .collect(),
        observations: vec![],
    }
}
fn input(data: &Replay) -> AestheticAnalysisInput {
    AestheticAnalysisInput {
        stage_id: "00000000-0000-4000-8000-000000000017".into(),
        stage_config_hash: "fixture".into(),
        candidates: data.candidates.len() as u64,
        observations: data.observations.len() as u64,
        evidence_watermark: data.observations.len() as u64,
        review_watermark: 0,
    }
}
fn status(mode: &str, cap: u64) -> AestheticSamplingStatus {
    let policy = AestheticSamplingPolicy {
        mode: mode.into(),
        min_exposures: 4,
        max_exposures: 8,
        rank_tolerance: 0.01,
        seed: 17,
    };
    AestheticSamplingStatus {
        plan_id: "fixture".into(),
        previous_plan_id: None,
        version: version(&policy).into(),
        policy,
        call_limit: cap,
        round: 0,
        evidence_watermark: 0,
        state: "ready".into(),
        reason: None,
        eligible: 0,
        covered: 0,
        stable: 0,
        components: 0,
        unresolved: 0,
    }
}
fn run(mode: &str, n: usize, ratings: bool, cap: u64) -> (Replay, Round) {
    let mut data = data(n, ratings);
    let mut state = status(mode, cap);
    let mut previous = vec![];
    let available = (0..n as u64).collect();
    for _ in 0..40 {
        let next = plan(
            &data,
            &input(&data),
            state,
            &previous,
            &available,
            cap - data.observations.len() as u64,
            &|| Ok(()),
        )
        .unwrap();
        if next.batches.is_empty() {
            return (data, next);
        }
        let mut used = BTreeSet::new();
        for batch in &next.batches {
            assert!((2..=16).contains(&batch.len()));
            let rating = data.candidates[batch[0].ordinal as usize].rating.clone();
            let mut ids: Vec<_> = batch.iter().map(|m| m.ordinal as u32).collect();
            for &id in &ids {
                let c = &mut data.candidates[id as usize];
                assert_eq!(c.rating, rating);
                assert!(used.insert(id));
                assert!(c.exposures < 8);
                c.exposures += 1;
            }
            ids.sort_by_key(|i| estimator::mix(u64::from(*i) ^ 711));
            data.observations.push(AestheticReplayObservation {
                batch: data.observations.len() as u64 + 1,
                rating,
                tiers: ids.into_iter().map(|i| vec![i]).collect(),
                elite: vec![],
                unjudgeable: vec![],
            });
        }
        assert!(data.observations.len() as u64 <= cap);
        state = next.status;
        previous = next.diagnostics;
    }
    panic!("refinement did not stop within the exposure budget");
}

#[test]
fn refinement_budget_preserves_coverage_ratings_and_explicit_limits() {
    let (data, round) = run("refine", 160, true, 60);
    assert_eq!(data.observations.len(), 60);
    assert!(
        data.candidates
            .iter()
            .all(|c| c.exposures >= 4 && c.exposures <= 8)
    );
    assert_eq!(round.status.state, "limited");
    assert_eq!(round.status.reason.as_deref(), Some("call_budget"));
    assert_eq!(round.status.stable, 0);
    assert!(
        round
            .diagnostics
            .iter()
            .all(|r| r.rank_sensitivity.is_some_and(|s| (0.0..=1.0).contains(&s)))
    );
}
#[test]
fn uniform_refinement_handles_small_populations_and_tail_batches() {
    for n in [2, 3, 15, 16, 17, 31, 33] {
        let (data, round) = run("refine_balanced", n, false, 100);
        assert_eq!(round.status.state, "satisfied", "n={n}");
        assert!(data.candidates.iter().all(|c| c.exposures == 4));
    }
}
#[test]
fn old_modes_keep_their_version_and_optional_diagnostic_is_backward_readable() {
    let (_, round) = run("balanced", 32, false, 8);
    assert_eq!(round.status.version, VERSION);
    assert!(
        round
            .diagnostics
            .iter()
            .all(|r| r.rank_sensitivity.is_none())
    );
    let mut value = serde_json::to_value(&round.diagnostics[0]).unwrap();
    value.as_object_mut().unwrap().remove("rank_sensitivity");
    let restored: AestheticSamplingDiagnostic = serde_json::from_value(value).unwrap();
    assert!(restored.rank_sensitivity.is_none());
    let mut incompatible = status("refine", 10);
    incompatible.version = VERSION.into();
    assert!(validate_version(&incompatible).is_err());
}
#[test]
fn refinement_rejects_unknown_versions_capacity_and_honors_cancellation() {
    let data = data(32, false);
    let available = (0..32).collect();
    let mut state = status("refine", 12);
    assert!(validate(&state.policy, MAX_CANDIDATES + 1).is_err());
    state.version = "future".into();
    assert_eq!(
        plan(&data, &input(&data), state, &[], &available, 12, &|| Ok(()))
            .err()
            .unwrap()
            .code,
        "EVALUATION_CONFIG_UNSUPPORTED"
    );
    assert_eq!(
        plan(
            &data,
            &input(&data),
            status("refine", 12),
            &[],
            &available,
            12,
            &|| Err(Error::new("CANCELLED", "fixture"))
        )
        .err()
        .unwrap()
        .code,
        "CANCELLED"
    );
}

#[test]
fn all_sampling_modes_accept_ten_million_and_reject_the_next_candidate() {
    assert_eq!(MAX_CANDIDATES, 10_000_000);
    assert_eq!(MAX_OBSERVATIONS, 20_000_000);
    for mode in ["balanced", "adaptive", "refine", "refine_balanced"] {
        let s = status(mode, 1);
        validate(&s.policy, 10_000_000).unwrap();
        assert!(validate(&s.policy, 10_000_001).is_err());
    }
}
