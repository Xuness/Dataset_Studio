use super::*;

#[test]
fn feathered_weighted_ranks_match_a_two_image_analytic_oracle() {
    let mut input = rows(2);
    let boundary = start(2024).unwrap();
    for (i, row) in input.iter_mut().enumerate() {
        let created = boundary + if i == 0 { -15 * DAY } else { 15 * DAY };
        row.created_at_us = Some(created);
        row.observed_at_us = Some(created + 400 * DAY);
        row.fav_count = Some(if i == 0 { 1 } else { 4 });
        row.up_score = Some(8);
        row.down_score = Some(0);
        row.score = Some(8);
    }
    let mut p = parameters();
    let cfg = p.v2.as_mut().unwrap();
    cfg.feather_days = 30;
    cfg.minimum_effective = 1;
    let (_, _, v) = run(&input, &p);
    // Period memberships are 27/32 and 5/32. Both periods have n_eff=512/377,
    // positive-support median 8, and K=2. Values below are independently
    // calculated from weighted midranks (27,5,59,37)/64 and one-sided shrinkage.
    let expected = [
        (
            1.3580901856763925,
            0.4998940431032656,
            0.4994278327576342,
            0.4998211977367607,
        ),
        (
            1.3580901856763925,
            0.5004577337938927,
            0.5000847655173876,
            0.5001430418105915,
        ),
    ];
    for (got, (n, left, right, mix)) in v.iter().zip(expected) {
        assert!((got.effective_count - n).abs() < 1e-12);
        assert!((got.old_percentile - left).abs() < 1e-12);
        assert!((got.new_percentile - right).abs() < 1e-12);
        assert!((got.year_percentile - mix).abs() < 1e-12);
        assert!(!got.era_fallback);
    }
}

fn rows(n: usize) -> Vec<RankingInput> {
    (0..n)
        .map(|i| {
            let created = start(2023 + (i % 3) as i32).unwrap() + (i % 120) as i64 * DAY;
            let heat = ((i * 19) % 71) as i64;
            RankingInput {
                ordinal: i as u64,
                source_id: "source".into(),
                asset_id: format!("{i:064x}"),
                record_id: Some(format!("{i:064x}")),
                observation_id: Some(format!("{i:064x}")),
                post_id: Some(i as i64 + 1),
                rating: Some("g".into()),
                created_at_us: Some(created),
                observed_at_us: Some(created + 400 * DAY),
                time_quality: "exact".into(),
                fav_count: Some(heat),
                up_score: Some(heat),
                down_score: Some(0),
                score: Some(heat),
                tags: Some(
                    if i % 11 == 0 {
                        "comic full_page_comic"
                    } else if i % 13 == 0 {
                        "comic full_page_comic 4koma cut-in"
                    } else {
                        "1girl"
                    }
                    .into(),
                ),
                tags_known: true,
                ..Default::default()
            }
        })
        .collect()
}
fn run(
    inputs: &[RankingInput],
    p: &RankingParameters,
) -> (RankingRatingSummary, Vec<Sample>, Vec<RankingV2Scores>) {
    let mut samples: Vec<_> = inputs.iter().map(|r| Sample::new(r, p).unwrap()).collect();
    let hints: Vec<_> = inputs
        .iter()
        .map(|r| type_hints(r.tags.as_deref()))
        .collect();
    let (summary, values) =
        compute("g", &mut samples, &hints, p, &mut [], &mut |_, _, _| Ok(())).unwrap();
    (summary, samples, values)
}
fn parameters() -> RankingParameters {
    let v2 = RankingV2Parameters {
        minimum_effective: 2,
        comic_penalty: 2.0,
        ..Default::default()
    };
    RankingParameters {
        v2: Some(v2),
        cohort_minimum: 2,
        ..Default::default()
    }
}
#[test]
fn feather_is_normalized_continuous_and_calendar_aware() {
    let b = start(2024).unwrap();
    for (d, w) in [
        (-90, 0.0),
        (-45, 0.15625),
        (0, 0.5),
        (45, 0.84375),
        (90, 1.0),
    ] {
        let f = feather(b + d * DAY, 90).unwrap();
        // At the right edge the pure-year representation is equivalent.
        let actual = if f.1 == 2024 && f.2 == 2024 { 1.0 } else { f.3 };
        assert!((actual - w).abs() < 1e-10);
    }
    let a = feather(b - 1_000_000, 90).unwrap();
    let z = feather(b + 1_000_000, 90).unwrap();
    assert_eq!((a.1, a.2), (2023, 2024));
    assert_eq!((z.1, z.2), (2023, 2024));
    assert!((a.3 - z.3).abs() < 1e-6);
    assert_eq!(feather(b, 0), Some((2024, 2024, 2024, 0.0)));
    let feb = NaiveDate::from_ymd_opt(2024, 2, 29)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_micros();
    assert_eq!(feather(feb, 180).unwrap().0, 2024);
}
#[test]
fn metadata_comic_hints_protect_short_storyboards_and_views() {
    assert_eq!(type_hints(Some("not_comic not_multiple_views")), 0);
    assert_eq!(type_hints(Some("comic 1koma monochrome speech_bubble")), 1);
    let mut input = rows(24);
    for r in &mut input {
        r.tags = Some("comic full_page_comic 2koma".into());
    }
    let (_, _, v) = run(&input, &parameters());
    assert!(
        v.iter()
            .all(|v| v.layout_protected && v.type_penalty == 0.0)
    );
    input[0].tags = Some("comic full_page_comic vertical_scroll_comic".into());
    let (_, _, v) = run(&input, &parameters());
    assert_eq!(v[0].type_penalty, 2.0); // One family, one bounded penalty.
}
#[test]
fn fusion_endpoints_and_composition_are_verifiable() {
    let input = rows(180);
    for lambda in [0.0, 1.0] {
        let mut p = parameters();
        let v2 = p.v2.as_mut().unwrap();
        v2.comic_penalty = 0.0;
        v2.profiles.get_mut("g").unwrap().era_weight = lambda;
        let (summary, samples, values) = run(&input, &p);
        assert!(summary.v2.unwrap().metadata_only);
        for ((i, s), v) in input.iter().zip(&samples).zip(&values) {
            let mut scores = s.scores("g");
            scores.v2 = Some(*v);
            validate_scores(i, &scores, &p).unwrap();
            let expected = if lambda == 0.0 {
                v.direct_rank
            } else {
                s.rescue_rank
            };
            assert_eq!(s.main_rank, expected);
        }
    }
}
#[test]
fn selection_respects_one_budget_and_strict_year_targets() {
    let input = rows(300);
    let mut p = parameters();
    p.mode = RankingMode::Select;
    let cfg = p.v2.as_mut().unwrap();
    cfg.keep_per_mille = 500;
    cfg.eras = vec![EraPreference {
        from_year: 2023,
        through_year: 2023,
        bonus: 3.0,
        target_share: Some(400),
    }];
    cfg.strict_era_targets = true;
    let (summary, samples, v) = run(&input, &p);
    let selected = |s: &Sample| {
        matches!(
            s.route,
            RankingRoute::Main | RankingRoute::Rescue | RankingRoute::Audit
        )
    };
    assert_eq!(samples.iter().filter(|s| selected(s)).count(), 150);
    assert_eq!(
        samples
            .iter()
            .zip(&v)
            .filter(|(s, v)| selected(s) && v.created_year == Some(2023))
            .count(),
        60
    );
    assert_eq!(summary.selected.iter().sum::<u64>(), 150);
    assert_eq!(summary.v2.unwrap().era_target_shortfall, 0);
    let cfg = p.v2.as_mut().unwrap();
    cfg.eras[0].target_share = Some(1000);
    let mut samples: Vec<_> = input.iter().map(|r| Sample::new(r, &p).unwrap()).collect();
    let hints: Vec<_> = input
        .iter()
        .map(|r| type_hints(r.tags.as_deref()))
        .collect();
    assert!(
        compute(
            "g",
            &mut samples,
            &hints,
            &p,
            &mut [],
            &mut |_, _, _| Ok(())
        )
        .is_err()
    );
    p.v2.as_mut().unwrap().strict_era_targets = false;
    let (summary, samples, _) = run(&input, &p);
    assert_eq!(samples.iter().filter(|s| selected(s)).count(), 150);
    assert_eq!(summary.v2.unwrap().era_target_shortfall, 50);
}
#[test]
fn unknown_time_and_cancelled_work_never_become_visual_evidence() {
    let mut input = rows(24);
    for r in &mut input {
        r.observed_at_us = None;
        r.time_quality = "unknown".into();
    }
    let p = parameters();
    let (summary, _, v) = run(&input, &p);
    assert_eq!(summary.v2.unwrap().fallback_count, 24);
    assert!(v.iter().all(|v| v.era_fallback && v.effective_count == 0.0));
    let mut samples: Vec<_> = input.iter().map(|r| Sample::new(r, &p).unwrap()).collect();
    let err = compute(
        "g",
        &mut samples,
        &[0; 24],
        &p,
        &mut [],
        &mut |phase, _, _| {
            if phase == "v2_periods" {
                Err(Error::new("CANCELLED", "stop"))
            } else {
                Ok(())
            }
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "CANCELLED");
}
#[test]
fn parameters_reject_overlaps_and_invalid_budgets() {
    let mut p = RankingV2Parameters {
        feather_days: 181,
        ..Default::default()
    };
    assert!(p.clone().normalize().is_err());
    p.feather_days = 90;
    p.eras = vec![
        EraPreference {
            from_year: 2020,
            through_year: 2025,
            bonus: 1.0,
            target_share: None,
        },
        EraPreference {
            from_year: 2025,
            through_year: 2026,
            bonus: 1.0,
            target_share: None,
        },
    ];
    assert!(p.clone().normalize().is_err());
    p.eras.clear();
    p.direct_rescue = 900;
    p.era_rescue = 900;
    assert!(p.normalize().is_err());
}
