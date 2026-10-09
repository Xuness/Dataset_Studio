use super::*;
use crate::ranking_tables::{RankingInputTable, RankingResultTable};
use std::sync::atomic::AtomicUsize;

fn fixture(count: u64) -> (tempfile::TempDir, RankingProjection, Vec<RankingScores>) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-runs");
    std::fs::create_dir_all(&root).unwrap();
    let temp = tempfile::Builder::new()
        .prefix("ranking-projection-")
        .tempdir_in(root)
        .unwrap();
    std::fs::create_dir(temp.path().join("artifacts")).unwrap();
    let source = new_id();
    let mut input =
        RankingInputTable::create_v2(&temp.path().join("artifacts/input.sqlite")).unwrap();
    let mut scores =
        RankingResultTable::create_v2(&temp.path().join("artifacts/scores.sqlite")).unwrap();
    let mut all = Vec::new();
    for chunk in (0..count).collect::<Vec<_>>().chunks(512) {
        let entries = chunk
            .iter()
            .map(|ordinal| {
                let rating = match ordinal % 17 {
                    0 => None,
                    1 => Some(""),
                    2 => Some("z"),
                    3..=6 => Some("e"),
                    7..=10 => Some("g"),
                    11..=13 => Some("q"),
                    _ => Some("s"),
                }
                .map(str::to_string);
                RankingInput {
                    ordinal: *ordinal,
                    source_id: source.clone(),
                    asset_id: format!("{:064x}", count - ordinal),
                    post_id: Some((ordinal / 3) as i64 + 1),
                    rating,
                    ..Default::default()
                }
            })
            .collect::<Vec<_>>();
        let rows = entries
            .iter()
            .map(|input| {
                let eligible = input.ordinal % 11 == 0
                    && input
                        .rating
                        .as_deref()
                        .is_some_and(|r| matches!(r, "e" | "g" | "q" | "s"));
                RankingScores {
                    ordinal: input.ordinal,
                    rating: input.rating.clone(),
                    eligibility: if eligible {
                        RankingEligibility::Eligible
                    } else {
                        RankingEligibility::DimensionsUnknown
                    },
                    missing_flags: if input.ordinal % 23 == 0 {
                        vec!["fixture_missing".into()]
                    } else {
                        Vec::new()
                    },
                    main_rank: eligible.then_some(input.ordinal + 1),
                    rescue_rank: eligible.then_some(count - input.ordinal),
                    selected_route: if input.ordinal % 97 == 0 {
                        RankingRoute::Main
                    } else if input.ordinal % 89 == 0 {
                        RankingRoute::Rescue
                    } else {
                        RankingRoute::BudgetRejected
                    },
                    v2: eligible.then_some(RankingV2Scores {
                        direct_rank: count - input.ordinal,
                        fused_rank: input.ordinal + 1,
                        ..Default::default()
                    }),
                    ..Default::default()
                }
            })
            .collect::<Vec<_>>();
        input.append(&entries).unwrap();
        scores.append(&rows).unwrap();
        all.extend(rows);
    }
    input
        .finalize(&["e".into(), "g".into(), "q".into(), "s".into()])
        .unwrap();
    scores
        .finish(&RankingSummary {
            input_count: count,
            ..Default::default()
        })
        .unwrap();
    let recipe = RankingProjection {
        edits: Vec::new(),
        member_result: None,
        version: 1,
        artifact_id: new_id(),
        input_file: "artifacts/input.sqlite".into(),
        score_file: "artifacts/scores.sqlite".into(),
        source_ids: vec![source],
        filter: RankingFilter::default(),
        ratings: None,
        count,
        workset_id: new_id(),
    };
    (temp, recipe, all)
}

fn rank(row: &RankingScores, order: RankingOrder) -> Option<u64> {
    match order {
        RankingOrder::Main => row.main_rank,
        RankingOrder::Rescue => row.rescue_rank,
        RankingOrder::Input => Some(row.ordinal),
        RankingOrder::Direct => row.v2.map(|v| v.direct_rank),
        RankingOrder::Fused => row.v2.map(|v| v.fused_rank),
    }
}
fn matches(row: &RankingScores, recipe: &RankingProjection) -> bool {
    let f = &recipe.filter;
    f.rating
        .as_ref()
        .is_none_or(|v| row.rating.as_ref() == Some(v))
        && recipe
            .ratings
            .as_ref()
            .is_none_or(|values| row.rating.as_ref().is_some_and(|v| values.contains(v)))
        && f.route.is_none_or(|v| v == row.selected_route)
        && f.eligibility.is_none_or(|v| v == row.eligibility)
        && (!f.missing_only || !row.missing_flags.is_empty())
        && (!f.selected_only
            || matches!(
                row.selected_route,
                RankingRoute::Main | RankingRoute::Rescue | RankingRoute::Audit
            ))
        && f.top.is_none_or(|top| {
            rank(
                row,
                if f.order == RankingOrder::Input {
                    RankingOrder::Main
                } else {
                    f.order
                },
            )
            .is_some_and(|n| n <= top)
        })
}

#[test]
fn shared_material_pages_match_independent_membership_in_every_order_and_direction() {
    let (temp, base, rows) = fixture(2048);
    let mut filters = vec![RankingFilter::default()];
    filters.extend([
        RankingFilter {
            rating: Some("g".into()),
            ..Default::default()
        },
        RankingFilter {
            eligibility: Some(RankingEligibility::DimensionsUnknown),
            ..Default::default()
        },
        RankingFilter {
            route: Some(RankingRoute::Main),
            ..Default::default()
        },
        RankingFilter {
            selected_only: true,
            ..Default::default()
        },
        RankingFilter {
            missing_only: true,
            ..Default::default()
        },
        RankingFilter {
            top: Some(48),
            ..Default::default()
        },
        RankingFilter {
            top: Some(48),
            order: RankingOrder::Rescue,
            ..Default::default()
        },
    ]);
    for (case, filter) in filters.into_iter().enumerate() {
        let mut recipe = base.clone();
        recipe.filter = filter;
        if case == 1 {
            recipe.ratings = Some(vec!["g".into(), "s".into()]);
        }
        let kept = rows
            .iter()
            .filter(|row| matches(row, &recipe))
            .collect::<Vec<_>>();
        recipe.count = kept.len() as u64;
        let reader =
            RankingProjectionReader::open(temp.path(), &recipe, Arc::new(AtomicBool::new(false)))
                .unwrap();
        assert_eq!(reader.count().unwrap(), recipe.count, "count case {case}");
        for order in [
            RankingOrder::Main,
            RankingOrder::Rescue,
            RankingOrder::Input,
            RankingOrder::Direct,
            RankingOrder::Fused,
        ] {
            for descending in [false, true] {
                let mut expected = kept.clone();
                expected.sort_by_key(|row| {
                    (
                        row.rating.clone().unwrap_or_else(|| "z".into()),
                        rank(row, order).unwrap_or(i64::MAX as u64),
                        row.ordinal,
                    )
                });
                if descending {
                    expected.reverse();
                }
                // Stable partition after applying direction: every ranked
                // image precedes every missing rank across Rating boundaries.
                expected.sort_by_key(|row| rank(row, order).is_none());
                let ranked_count = expected
                    .iter()
                    .filter(|row| rank(row, order).is_some())
                    .count() as u64;
                let expected = expected.iter().map(|r| r.ordinal).collect::<Vec<_>>();
                let mut after = None;
                let mut actual = Vec::new();
                loop {
                    let page = reader.page(order, descending, after.as_ref(), 29).unwrap();
                    if page.is_empty() {
                        break;
                    }
                    after = page.last().cloned();
                    actual.extend(page.iter().map(|r| r.ordinal));
                    assert!(
                        actual.len() <= expected.len(),
                        "page repeated case {case} {order:?} {descending}"
                    );
                }
                assert_eq!(actual, expected, "case {case} {order:?} {descending}");
                for position in [1, 17, 129, ranked_count, ranked_count + 1, recipe.count]
                    .into_iter()
                    .filter(|n| *n > 0 && *n <= recipe.count)
                {
                    let anchor = reader
                        .locate_position(position, order, descending)
                        .unwrap()
                        .unwrap();
                    assert_eq!(
                        anchor.ordinal,
                        expected[position as usize - 1],
                        "position {position}, case {case} {order:?} {descending}"
                    );
                }
                for post in [1, 43, 682] {
                    let found = reader
                        .locate(post, order, descending)
                        .unwrap()
                        .map(|r| r.ordinal);
                    assert_eq!(
                        found,
                        expected.iter().copied().find(|n| n / 3 + 1 == post as u64)
                    );
                }
            }
        }
    }
}

#[test]
fn late_null_rank_pages_seek_the_tie_instead_of_rescanning_it() {
    let (temp, recipe, _) = fixture(65_536);
    let reader =
        RankingProjectionReader::open(temp.path(), &recipe, Arc::new(AtomicBool::new(false)))
            .unwrap();
    let ticks = Arc::new(AtomicUsize::new(0));
    let counter = ticks.clone();
    reader
        .db
        .progress_handler(
            100,
            Some(move || counter.fetch_add(1, Ordering::Relaxed) > 400),
        )
        .unwrap();
    let after = ranking_tables::RankingPosition {
        group: "g".into(),
        position: i64::MAX,
        ordinal: 60_000,
    };
    let page = reader
        .page(RankingOrder::Main, false, Some(&after), 48)
        .unwrap();
    assert_eq!(page.len(), 48);
    assert!(page.iter().all(|p| p.ordinal > 60_000 && p.group == "g"));
    assert!(ticks.load(Ordering::Relaxed) <= 400);
}

#[test]
fn projections_remain_readonly_and_reject_escape_paths_or_cancelled_reads() {
    let (temp, mut recipe, _) = fixture(256);
    recipe.input_file = "../foreign.sqlite".into();
    assert!(recipe.files(temp.path()).is_err());
    recipe.input_file = "artifacts/input.sqlite".into();
    let cancelled = Arc::new(AtomicBool::new(true));
    let reader = RankingProjectionReader::open(temp.path(), &recipe, cancelled);
    match reader {
        Ok(reader) => assert!(
            reader
                .count_filter(&RankingFilter {
                    missing_only: true,
                    ..Default::default()
                })
                .is_err()
        ),
        Err(error) => assert!(matches!(error.code, "DATABASE_ERROR" | "CANCELLED")),
    }
    let reader =
        RankingProjectionReader::open(temp.path(), &recipe, Arc::new(AtomicBool::new(false)))
            .unwrap();
    assert!(reader.db.execute("DELETE FROM scores", []).is_err());
    assert!(
        reader
            .db
            .execute("DELETE FROM fixed_input.input_rows", [])
            .is_err()
    );
}
