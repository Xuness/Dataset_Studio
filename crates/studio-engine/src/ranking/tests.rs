use super::*;

#[test]
fn compact_phase_readers_match_full_input_semantics() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-runs");
    fs::create_dir_all(&root).unwrap();
    let temporary = tempfile::tempdir_in(root).unwrap();
    let mut input =
        RankingInputTable::create_enriched(&temporary.path().join("input.sqlite"), true).unwrap();
    let rows: Vec<_>=(0u64..128).map(|n| RankingInput {
        ordinal:n,source_id:if n==127 { "second" } else { "source" }.into(),
        asset_id:format!("{:064x}",if n==127 { 126 } else { n }),
        record_id:(n%11!=0).then(||format!("{:064x}",n+128)),
        observation_id:(n%13!=0).then(||format!("{:064x}",n+256)),
        rating:(n%17!=0).then(||["g","s","q","e","invalid"][n as usize%5].into()),
        created_at_us:(n%7!=0).then_some(n as i64*86_400_000_000),
        observed_at_us:(n%9!=0).then_some((n+3) as i64*86_400_000_000),
        time_quality:if n%2==0 { "exact" } else { "date_only" }.into(),
        fav_count:(n%3!=0).then_some(n as i64-20),up_score:Some(n as i64),down_score:Some(-2),score:Some(n as i64-1),
        artists:if n%4==0 { vec![] } else { vec!["a".into(),"b".into()] },
        stored_width:(n%5!=0).then_some(if n%3==0 { 128 } else { 1024 }),stored_height:Some(512),
        dimension_basis:if n%3==0 { "not_requested" } else { "asset_storage_details" }.into(),
        is_banned:Some(n%6==0),tags_known:n%4!=0,rating_conflict:n%8==0,
        source_issues:Some(if n%2==0 { "[]" } else { " [\"source_issue\"] " }.into()),
        tags:Some("comic jpeg_artifacts".into()),
        evidence:(n%10==0).then(||serde_json::from_value(serde_json::json!({
            "policy":"sum","metadata":{"record_id":"x","observation_id":"y","time_quality":"unknown"},
            "heat":[],"post_count":2,"omitted_posts":0,"partial_counts":true,"counts_clamped":true
        })).unwrap()),
        ..Default::default()
    }).collect();
    input.append(&rows).unwrap();
    input
        .finalize(&["g".into(), "s".into(), "q".into(), "e".into()])
        .unwrap();
    let full = input.page(None, None).unwrap();
    let classification = input.classification_page(None).unwrap();
    for minimum in [None, Some(384)] {
        for exclude_banned in [false, true] {
            let p = RankingParameters {
                minimum_stored_side: minimum,
                exclude_banned,
                ratings: vec!["g".into(), "e".into()],
                v2: Some(Default::default()),
                ..Default::default()
            };
            let validation = input.validation_page(None, &p).unwrap();
            for ((a, b), c) in full.iter().zip(&classification).zip(&validation) {
                assert_eq!(a.ordinal, b.ordinal);
                assert_eq!(a.ordinal, c.ordinal);
                assert_eq!(a.duplicate_of, b.duplicate_of);
                assert_eq!(a.duplicate_of, c.duplicate_of);
                assert_eq!(formula::eligibility(a, &p), formula::eligibility(b, &p));
                assert_eq!(formula::eligibility(a, &p), formula::eligibility(c, &p));
                assert_eq!(formula::flags(a), formula::flags(b));
                if a.duplicate_of.is_none()
                    && formula::eligibility(a, &p) == RankingEligibility::Eligible
                {
                    assert_eq!(a.tags, c.tags);
                    assert_eq!(a.created_at_us, c.created_at_us);
                }
            }
            for rating in ["g", "s", "q", "e", "invalid"] {
                let expected: Vec<_> = full
                    .iter()
                    .filter(|r| {
                        r.rating.as_deref() == Some(rating)
                            && r.duplicate_of.is_none()
                            && formula::eligibility(r, &p) == RankingEligibility::Eligible
                    })
                    .map(|r| r.ordinal)
                    .collect();
                let actual: Vec<_> = input
                    .eligible_page(None, rating, &p)
                    .unwrap()
                    .iter()
                    .map(|r| r.ordinal)
                    .collect();
                assert_eq!(actual, expected);
            }
        }
    }
}
