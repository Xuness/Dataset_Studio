use super::*;

fn date(value: &str) -> i64 {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .timestamp_micros()
}
fn input(id: u64, created: i64, age: i64) -> RankingInput {
    RankingInput {
        ordinal: id,
        source_id: "source".into(),
        asset_id: format!("{id:064x}"),
        record_id: Some(format!("record-{id}")),
        observation_id: Some(format!("observation-{id}")),
        post_id: Some(id as i64 + 1),
        rating: Some("g".into()),
        created_at_us: Some(created),
        observed_at_us: Some(created + age),
        time_quality: "exact".into(),
        fav_count: Some((id % 23) as i64),
        up_score: Some((id % 19) as i64),
        down_score: Some(-((id % 5) as i64)),
        stored_width: Some(1024),
        stored_height: Some(768),
        tags_known: true,
        ..Default::default()
    }
}
fn run(
    rows: &[RankingInput],
    p: &RankingParameters,
    artists: &mut [ArtistWork],
) -> (Vec<Sample>, RankingRatingSummary) {
    let mut samples = rows
        .iter()
        .map(|v| Sample::new(v, p).unwrap())
        .collect::<Vec<_>>();
    let summary = compute_rating("g", &mut samples, p, artists, &mut |_, _, _| Ok(())).unwrap();
    (samples, summary)
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-10, "{a} != {b}");
}

/// Independent quadratic reference: no tree, histogram, sweep, or optimized rank calls.
fn local_oracle(
    rows: &[RankingInput],
    g: f64,
    i: usize,
    p: &RankingParameters,
) -> (f64, u64, Option<u32>, Option<f64>) {
    let row = &rows[i];
    let Some(key) = heat_key(row.fav_count, row.up_score) else {
        return (g, 0, None, None);
    };
    let Some(age) = age_interval(row.created_at_us, row.observed_at_us, &row.time_quality) else {
        return (g, 0, None, None);
    };
    for (level, (months, edges)) in [(6, BASE), (12, BASE), (24, BASE), (24, COARSE), (24, BROAD)]
        .into_iter()
        .enumerate()
    {
        let Some(b) = edges
            .windows(2)
            .position(|v| age.0 >= v[0].saturating_mul(DAY) && age.1 < v[1].saturating_mul(DAY))
        else {
            continue;
        };
        let (lo, hi) = month_window(row.created_at_us.unwrap(), months).unwrap();
        let mut heat = Vec::new();
        let mut positive = Vec::new();
        for candidate in rows {
            if candidate.created_at_us.is_none_or(|t| t < lo || t > hi) {
                continue;
            }
            let Some((a, z)) = age_interval(
                candidate.created_at_us,
                candidate.observed_at_us,
                &candidate.time_quality,
            ) else {
                continue;
            };
            if a < edges[b].saturating_mul(DAY) || z >= edges[b + 1].saturating_mul(DAY) {
                continue;
            }
            if let Some(h) = heat_key(candidate.fav_count, candidate.up_score) {
                heat.push(h);
                let m = valid_count(candidate.fav_count)
                    .unwrap_or(0)
                    .max(valid_count(candidate.up_score).unwrap_or(0));
                if m > 0 {
                    positive.push(m);
                }
            }
        }
        if heat.len() < p.cohort_minimum as usize {
            continue;
        }
        let local = (heat.iter().filter(|&&h| h < key).count() as f64
            + 0.5 * heat.iter().filter(|&&h| h == key).count() as f64)
            / heat.len() as f64;
        positive.sort_unstable();
        let k = if positive.is_empty() {
            1.0
        } else {
            let n = positive.len();
            let median = if n % 2 == 1 {
                positive[n / 2] as f64
            } else {
                0.5 * (positive[n / 2 - 1] as f64 + positive[n / 2] as f64)
            };
            (0.25 * median).clamp(1.0, 10.0)
        };
        let q = 0.5 + heat.len() as f64 / (heat.len() as f64 + 1000.0) * (local - 0.5);
        let m = valid_count(row.fav_count)
            .unwrap_or(0)
            .max(valid_count(row.up_score).unwrap_or(0)) as f64;
        return (
            if q <= 0.5 {
                q
            } else {
                0.5 + m / (m + k) * (q - 0.5)
            },
            heat.len() as u64,
            Some(level as u32),
            Some(k),
        );
    }
    (g, 0, None, None)
}

#[test]
fn exact_heat_ties_and_unknowns_do_not_turn_into_zero_votes() {
    assert_eq!(heat_key(Some(1), Some(8)), heat_key(Some(2), Some(5)));
    assert_eq!(heat_key(Some(2), None), heat_key(Some(2), Some(2)));
    assert_eq!(heat_key(None, Some(-1)), None);
    assert!(heat_key(Some(i64::MAX), Some(i64::MAX)).is_some());
    let p = RankingParameters::default();
    let mut rows = (0..4)
        .map(|i| input(i, date("2026-01-01T00:00:00Z"), DAY))
        .collect::<Vec<_>>();
    for row in &mut rows {
        row.fav_count = Some(0);
        row.up_score = Some(0);
    }
    rows[3].fav_count = None;
    rows[3].up_score = None;
    let (samples, summary) = run(&rows, &p, &mut []);
    assert_eq!(summary.valid_heat, 3);
    for s in samples {
        near(s.g, 0.5);
        near(s.c, 0.5);
        assert!(s.main_score.is_finite());
    }
}

#[test]
fn dates_are_intervals_and_month_end_windows_can_move_backwards() {
    let c = date("2026-05-18T12:00:00Z");
    let o = date("2026-05-18T00:00:00Z");
    assert_eq!(
        age_interval(Some(c), Some(o), "date_only"),
        Some((0, DAY / 2 - 1))
    );
    assert!(age_interval(Some(c), Some(o), "exact").is_none());
    assert!(age_interval(Some(c), Some(o - DAY), "date_only").is_none());
    let a = month_window(date("2026-08-30T23:00:00Z"), 6).unwrap();
    let b = month_window(date("2026-08-31T00:00:00Z"), 6).unwrap();
    assert!(b.0 < a.0 && b.1 < a.1);
}

#[test]
fn optimized_cohorts_match_independent_population_oracle() {
    let p = RankingParameters {
        cohort_minimum: 11,
        ..Default::default()
    };
    let start = date("2023-01-01T00:00:00Z");
    let ages = [
        DAY,
        5 * DAY,
        12 * DAY,
        25 * DAY,
        60 * DAY,
        300 * DAY,
        800 * DAY,
        1500 * DAY,
        3000 * DAY,
    ];
    let mut rows = (0..640)
        .map(|i| {
            input(
                i,
                start + ((i * 37) % 1400) as i64 * DAY,
                ages[i as usize % ages.len()],
            )
        })
        .collect::<Vec<_>>();
    for (i, row) in rows.iter_mut().enumerate() {
        if i % 11 == 0 {
            row.time_quality = "date_only".into();
            row.observed_at_us = Some(row.created_at_us.unwrap() + 3 * DAY - DAY / 2);
        }
        if i % 17 == 0 {
            row.observed_at_us = None;
            row.time_quality = "unknown".into();
        }
        if i % 23 == 0 {
            row.fav_count = None;
        }
    }
    // Non-monotonic month-end limits with close observations near the boundary.
    for s in [
        "2026-08-30T23:00:00Z",
        "2026-08-31T00:00:00Z",
        "2026-02-28T12:00:00Z",
        "2027-02-28T12:00:00Z",
    ] {
        for _ in 0..12 {
            rows.push(input(rows.len() as u64, date(s), 5 * DAY));
        }
    }
    let (samples, _) = run(&rows, &p, &mut []);
    let valid = rows
        .iter()
        .filter_map(|r| heat_key(r.fav_count, r.up_score))
        .collect::<Vec<_>>();
    for (i, s) in samples.iter().enumerate() {
        let expected_g = if let Some(k) = s.heat {
            (valid.iter().filter(|&&v| v < k).count() as f64
                + 0.5 * valid.iter().filter(|&&v| v == k).count() as f64)
                / valid.len() as f64
        } else {
            0.5
        };
        near(s.g, expected_g);
        let (c, n, level, k) = local_oracle(&rows, s.g, i, &p);
        near(s.c, c);
        assert_eq!(s.local_n, n, "item {i}");
        assert_eq!(s.level, level, "item {i}");
        assert_eq!(s.k, k, "item {i}");
    }
    assert!(samples.iter().any(|s| s.level.is_some_and(|v| v >= 3)));
}

#[test]
fn clipped_positive_median_handles_sparse_extreme_votes() {
    let p = RankingParameters {
        cohort_minimum: 4,
        ..Default::default()
    };
    let mut rows = (0..8)
        .map(|i| input(i, date("2026-01-01T00:00:00Z"), DAY))
        .collect::<Vec<_>>();
    for r in &mut rows {
        r.fav_count = Some(0);
        r.up_score = Some(0);
    }
    rows[0].fav_count = Some(1);
    rows[1].fav_count = Some(i64::MAX);
    let (samples, _) = run(&rows, &p, &mut []);
    for s in samples {
        near(s.k.unwrap(), 10.0);
    }
}

#[test]
fn quotas_and_routes_are_exact_deterministic_and_nonoverlapping() {
    for n in 0..10_000 {
        let q = quotas(n, [280, 43, 10]);
        assert_eq!(q.iter().sum::<u64>(), n * 333 / 1000);
    }
    assert_eq!(quotas(1, [500, 500, 0]), [1, 0, 0]);
    let p = RankingParameters {
        mode: RankingMode::Select,
        ..Default::default()
    };
    let rows = (0..101)
        .map(|i| input(i, date("2026-01-01T00:00:00Z"), DAY))
        .collect::<Vec<_>>();
    let (samples, summary) = run(&rows, &p, &mut []);
    assert_eq!(summary.selected, summary.quotas);
    let mut reverse = rows.clone();
    reverse.reverse();
    let (mut other, _) = run(&reverse, &p, &mut []);
    other.sort_by_key(|s| s.ordinal);
    for (a, b) in samples.iter().zip(other) {
        assert_eq!(a.route, b.route);
        assert_eq!(a.main_rank, b.main_rank);
    }
    assert!(digest(&p.seed, "audit", &[0; 32]) != digest(&p.seed, "score-tie", &[0; 32]));
}

#[test]
fn artists_exclude_self_and_limit_related_variants() {
    let p = RankingParameters {
        artist_enabled: true,
        time_enabled: false,
        ..Default::default()
    };
    let mut rows = (0..60)
        .map(|i| input(i, date("2026-01-01T00:00:00Z"), DAY))
        .collect::<Vec<_>>();
    for (i, r) in rows.iter_mut().enumerate() {
        r.fav_count = Some(i as i64);
        r.up_score = Some(i as i64);
    }
    let mut works = (40..60)
        .map(|item| ArtistWork { item, artist: 0 })
        .collect::<Vec<_>>();
    let (plain, _) = run(&rows, &p, &mut works);
    assert!(plain[55].a > 0.0);
    assert_eq!(plain[55].artist_support, 19);
    let mu = plain.iter().map(|s| s.c).sum::<f64>() / plain.len() as f64;
    let expected =
        ((plain[40..60].iter().map(|s| s.c).sum::<f64>() - plain[55].c + 50.0 * mu) / 69.0 - mu)
            / 0.2;
    near(plain[55].a, expected.clamp(0.0, 1.0));
    for r in &mut rows[40..60] {
        r.parent_id = Some(9_999);
    }
    let (grouped, _) = run(&rows, &p, &mut works);
    for s in grouped {
        near(s.a, 0.0);
    }
}

#[test]
fn eligibility_is_separate_from_missing_and_from_selection() {
    let p = RankingParameters {
        minimum_stored_side: Some(768),
        ..Default::default()
    };
    let mut row = input(0, date("2026-01-01T00:00:00Z"), DAY);
    assert_eq!(eligibility(&row, &p), RankingEligibility::Eligible);
    row.stored_width = None;
    assert_eq!(eligibility(&row, &p), RankingEligibility::DimensionsUnknown);
    row.stored_width = Some(512);
    assert_eq!(
        eligibility(&row, &p),
        RankingEligibility::DimensionsExcluded
    );
    row.stored_width = Some(1024);
    row.observed_at_us = None;
    assert_eq!(eligibility(&row, &p), RankingEligibility::Eligible);
    assert!(flag_names(flags(&row)).contains(&"observation_time_unknown".into()));
}
