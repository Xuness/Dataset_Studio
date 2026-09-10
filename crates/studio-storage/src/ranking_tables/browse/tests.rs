use super::*;
use sha2::{Digest, Sha256};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering as AtomicOrdering},
};

#[test]
fn bidirectional_scans_keep_saved_membership_and_match_independent_sql() {
    let temp = tempfile::tempdir().unwrap();
    let mut table = RankingResultTable::create(&temp.path().join("scores.sqlite")).unwrap();
    let ratings = [
        Some("e"),
        Some("g"),
        Some("q"),
        Some("s"),
        None,
        Some("z"),
        Some("zz"),
    ];
    let rows = (0..512)
        .map(|ordinal| RankingScores {
            ordinal,
            rating: ratings[ordinal as usize % ratings.len()].map(str::to_owned),
            main_rank: (ordinal % 11 != 0).then_some(1 + ordinal % 31),
            rescue_rank: (ordinal % 13 != 0).then_some(1 + (ordinal * 17) % 37),
            selected_route: if ordinal % 3 == 0 {
                RankingRoute::Main
            } else {
                RankingRoute::BudgetRejected
            },
            missing_flags: if ordinal % 5 == 0 {
                vec!["unknown".into()]
            } else {
                vec![]
            },
            ..Default::default()
        })
        .collect::<Vec<_>>();
    table.append(&rows).unwrap();
    table
        .finish(&RankingSummary {
            input_count: 512,
            ..Default::default()
        })
        .unwrap();
    let cases = [
        (RankingFilter::default(), "1=1"),
        (
            RankingFilter {
                top: Some(5),
                ..Default::default()
            },
            "rating IN ('e','g','q','s') AND main_rank<=5",
        ),
        (
            RankingFilter {
                top: Some(4),
                order: RankingOrder::Rescue,
                ..Default::default()
            },
            "rating IN ('e','g','q','s') AND rescue_rank<=4",
        ),
        (
            RankingFilter {
                selected_only: true,
                missing_only: true,
                ..Default::default()
            },
            "selected_route IN ('main','rescue','audit') AND missing_flags!='[]'",
        ),
        (
            RankingFilter {
                rating: Some("g".into()),
                route: Some(RankingRoute::Main),
                ..Default::default()
            },
            "rating='g' AND selected_route='main'",
        ),
    ];
    for (filter, predicate) in cases {
        for order in [
            RankingOrder::Main,
            RankingOrder::Rescue,
            RankingOrder::Input,
        ] {
            for descending in [false, true] {
                let position = match order {
                    RankingOrder::Main => "coalesce(main_rank,9223372036854775807)",
                    RankingOrder::Rescue => "coalesce(rescue_rank,9223372036854775807)",
                    RankingOrder::Input => "ordinal",
                };
                let direction = if descending { "DESC" } else { "ASC" };
                let expected = table.db.prepare(&format!("SELECT ordinal FROM scores WHERE {predicate} ORDER BY coalesce(rating,'z') {direction},{position} {direction},ordinal {direction}")).unwrap()
                    .query_map([], |r| unsigned(r,0)).unwrap().collect::<std::result::Result<Vec<_>,_>>().unwrap();
                let mut actual = Vec::new();
                let mut after = None;
                let mut visited = 0;
                loop {
                    let page = table
                        .browse_scan(&filter, order, descending, after.as_ref(), 7)
                        .unwrap();
                    assert!(page.rows.len() <= 7);
                    for (row, keep) in page.rows {
                        visited += 1;
                        after = Some(RankingPosition::for_scores(&row, order));
                        if keep {
                            actual.push(row.ordinal);
                        }
                    }
                    assert!(visited <= 512, "raw cursors must advance without repeats");
                    if !page.more {
                        break;
                    }
                }
                assert_eq!(
                    actual, expected,
                    "filter={filter:?} view={order:?} desc={descending}"
                );
            }
        }
    }
}

#[test]
fn legacy_post_lookup_is_bounded_and_preserves_input_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("input.sqlite");
    let mut input = RankingInputTable::create(&path).unwrap();
    for first in (0..2048).step_by(512) {
        input
            .append(
                &(first..first + 512)
                    .map(|ordinal| RankingInput {
                        ordinal,
                        source_id: "source".into(),
                        asset_id: format!("{ordinal:064x}"),
                        post_id: Some(if matches!(ordinal, 259 | 1300) {
                            9999
                        } else {
                            10000 + ordinal as i64
                        }),
                        rating: Some("g".into()),
                        ..Default::default()
                    })
                    .collect::<Vec<_>>(),
            )
            .unwrap();
    }
    input.flush().unwrap();
    let before = Sha256::digest(fs::read(&path).unwrap());
    let reader = RankingInputTable::open(&path).unwrap();
    for post_id in [9999, 9999999] {
        let mut next = 0;
        let mut matches = Vec::new();
        while next < 2048 {
            let page = reader.post_id_scan(post_id, next, 2048, 257).unwrap();
            assert!(page.next_ordinal > next && page.next_ordinal - next <= 257);
            assert!(page.ordinals.len() <= 512);
            next = page.next_ordinal;
            matches.extend(page.ordinals);
        }
        assert_eq!(
            matches,
            if post_id == 9999 {
                vec![259, 1300]
            } else {
                vec![]
            }
        );
    }
    assert!(reader.post_id_scan(0, 0, 2048, 257).is_err());
    assert!(reader.post_id_scan(1, 2049, 2048, 257).is_err());
    drop(reader);
    assert_eq!(Sha256::digest(fs::read(&path).unwrap()), before);
    // New materials build the optional index before their immutable publication.
    input.finalize(&["g".into()]).unwrap();
    drop(input);
    let reader = RankingInputTable::open(&path).unwrap();
    let page = reader.post_id_scan(9999, 0, 2048, 1).unwrap();
    assert_eq!(page.ordinals, vec![259, 1300]);
    assert_eq!(page.next_ordinal, 2048);
    let plan: String = reader.db.query_row("EXPLAIN QUERY PLAN SELECT ordinal FROM input_rows WHERE post_id=9999 AND ordinal>=0 ORDER BY ordinal LIMIT 513", [], |r| r.get(3)).unwrap();
    assert!(
        plan.contains("input_post") && plan.contains("SEARCH"),
        "{plan}"
    );
}

#[test]
fn sparse_membership_uses_bounded_raw_work_in_the_opposite_order() {
    let temp = tempfile::tempdir().unwrap();
    let mut table = RankingResultTable::create(&temp.path().join("large.sqlite")).unwrap();
    let total = 100_352;
    for first in (0..total).step_by(512) {
        table
            .append(
                &(first..first + 512)
                    .map(|ordinal| RankingScores {
                        ordinal,
                        rating: Some("g".into()),
                        main_rank: Some(ordinal + 1),
                        rescue_rank: Some(total - ordinal),
                        ..Default::default()
                    })
                    .collect::<Vec<_>>(),
            )
            .unwrap();
    }
    table
        .finish(&RankingSummary {
            input_count: total,
            ..Default::default()
        })
        .unwrap();
    let steps = Arc::new(AtomicUsize::new(0));
    let captured = steps.clone();
    table
        .db
        .progress_handler(
            100,
            Some(move || {
                captured.fetch_add(1, AtomicOrdering::Relaxed);
                false
            }),
        )
        .unwrap();
    let page = table
        .browse_scan(
            &RankingFilter {
                top: Some(1),
                ..Default::default()
            },
            RankingOrder::Rescue,
            false,
            None,
            16,
        )
        .unwrap();
    table.db.progress_handler(0, None::<fn() -> bool>).unwrap();
    assert_eq!(page.rows.len(), 16);
    assert!(page.rows.iter().all(|(_, keep)| !keep));
    assert!(page.more);
    assert!(
        steps.load(AtomicOrdering::Relaxed) < 700,
        "one page must not scan the entire sparse population"
    );
}
