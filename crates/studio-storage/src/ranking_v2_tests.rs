use super::*;
#[test]
fn v2_tags_and_all_orderings_roundtrip_without_rewriting_v1() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-runs");
    fs::create_dir_all(&root).unwrap();
    let dir = tempfile::Builder::new()
        .prefix("ranking-v2-material-")
        .tempdir_in(root)
        .unwrap();
    let input_path = dir.path().join("input.sqlite");
    let output_path = dir.path().join("scores.sqlite");
    let mut input = RankingInputTable::create_v2(&input_path).unwrap();
    let mut rows = Vec::new();
    for i in 0..3u64 {
        rows.push(RankingInput {
            ordinal: i,
            source_id: "source".into(),
            asset_id: format!("{i:064x}"),
            record_id: Some(format!("{i:064x}")),
            observation_id: Some(format!("{i:064x}")),
            rating: Some("g".into()),
            tags: Some("comic 4koma cut-in 中文线索".into()),
            ..Default::default()
        });
    }
    input.append(&rows).unwrap();
    input.finalize(&["g".into()]).unwrap();
    drop(input);
    let input = RankingInputTable::open(&input_path).unwrap();
    assert!(input.is_v2().unwrap());
    assert_eq!(input.row(1).unwrap().tags, rows[1].tags);
    assert_eq!(input.page(None, None).unwrap().len(), 3);
    let mut output = RankingResultTable::create_v2(&output_path).unwrap();
    for i in 0..3u64 {
        output
            .append(&[RankingScores {
                ordinal: i,
                rating: Some("g".into()),
                eligibility: RankingEligibility::Eligible,
                main_rank: Some(i + 1),
                rescue_rank: Some(3 - i),
                selected_route: RankingRoute::Ranked,
                v2: Some(RankingV2Scores {
                    direct_rank: [3, 1, 2][i as usize],
                    fused_rank: [2, 3, 1][i as usize],
                    created_year: Some(2025),
                    year_rank: i + 1,
                    ..Default::default()
                }),
                ..Default::default()
            }])
            .unwrap();
    }
    output
        .finish(&RankingSummary {
            schema_version: 2,
            input_count: 3,
            eligible_count: 3,
            parameters: RankingParameters {
                v2: Some(RankingV2Parameters::default()),
                ..Default::default()
            },
            ratings: vec![RankingRatingSummary {
                rating: "g".into(),
                eligible: 3,
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap();
    drop(output);
    let output = RankingResultTable::open(&output_path).unwrap();
    assert!(output.is_v2().unwrap());
    for (order, expected) in [
        (RankingOrder::Direct, vec![1, 2, 0]),
        (RankingOrder::Fused, vec![2, 0, 1]),
    ] {
        let filter = RankingFilter {
            rating: Some("g".into()),
            order,
            ..Default::default()
        };
        let page = output.browse_scan(&filter, order, false, None, 8).unwrap();
        assert_eq!(
            page.rows
                .into_iter()
                .map(|(r, _)| r.ordinal)
                .collect::<Vec<_>>(),
            expected
        );
        let filter = RankingFilter {
            top: Some(1),
            ..filter
        };
        assert_eq!(output.filtered_count(&filter).unwrap(), 1);
        assert_eq!(
            output.filtered_page(&filter, None, 8).unwrap().0[0].ordinal,
            expected[0]
        );
    }
    let old_path = dir.path().join("old.sqlite");
    let mut old = RankingResultTable::create(&old_path).unwrap();
    old.append(&[RankingScores {
        ordinal: 0,
        rating: Some("g".into()),
        main_rank: Some(1),
        ..Default::default()
    }])
    .unwrap();
    old.finish(&RankingSummary::default()).unwrap();
    drop(old);
    let before = fs::read(&old_path).unwrap();
    let old = RankingResultTable::open(&old_path).unwrap();
    assert!(!old.is_v2().unwrap());
    assert!(old.row(0).unwrap().v2.is_none());
    let filter = RankingFilter {
        rating: Some("g".into()),
        order: RankingOrder::Direct,
        top: Some(1),
        ..Default::default()
    };
    assert_eq!(old.filtered_count(&filter).unwrap(), 1);
    assert_eq!(
        old.browse_scan(&filter, RankingOrder::Direct, false, None, 8)
            .unwrap()
            .rows
            .len(),
        1
    );
    drop(old);
    assert_eq!(before, fs::read(&old_path).unwrap());
}
