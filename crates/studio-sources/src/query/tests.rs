use super::*;
use std::{collections::BTreeSet, fs};

fn hex(n: u64) -> String {
    format!("{n:064x}")
}
struct Fixture {
    _root: tempfile::TempDir,
    source: Source,
    reader: QueryReader,
}
fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let source = Source {
        id: new_id(),
        name: "查询夹具".into(),
        kind: "danbooru".into(),
        index_root: Some(root.path().into()),
        media_root: Some(root.path().into()),
    };
    let generation = root.path().join("indexes/gen-query");
    fs::create_dir_all(&generation).unwrap();
    fs::write(
        root.path().join("CURRENT.json"),
        serde_json::to_vec(
            &serde_json::json!({"library_id":source.id,"index_version":1,"generation":"gen-query"}),
        )
        .unwrap(),
    )
    .unwrap();
    fs::write(root.path().join("library.json"), serde_json::to_vec(&serde_json::json!({"library_id":source.id,"format_version":1,"image_format":"uncompressed-pax-tar"})).unwrap()).unwrap();
    let catalog = rusqlite::Connection::open(generation.join("catalog.sqlite")).unwrap();
    catalog.execute_batch("CREATE TABLE state(key TEXT PRIMARY KEY,value INTEGER); INSERT INTO state VALUES ('seq',1); CREATE TABLE objects(sha256 TEXT PRIMARY KEY,length INTEGER,stored_ext TEXT);").unwrap();
    for n in [1, 2, 3, 4, 6] {
        catalog
            .execute(
                "INSERT INTO objects VALUES (?1,?2,?3)",
                rusqlite::params![
                    hex(n),
                    (n * 100) as i64,
                    if n == 6 { None } else { Some("webp") }
                ],
            )
            .unwrap();
    }
    drop(catalog);
    let dll = crate::duckdb::bundled_library();
    let db = Session::fixture(&dll, &generation.join("analysis.duckdb")).unwrap();
    db.query("CREATE TABLE applied(seq BIGINT); INSERT INTO applied VALUES(1); CREATE TABLE assets(asset_id VARCHAR,observation_id VARCHAR,post_id BIGINT,sha256 VARCHAR); CREATE TABLE observations(observation_id VARCHAR,row_id BIGINT,post_id BIGINT,image_width BIGINT,image_height BIGINT,file_ext VARCHAR,score BIGINT,fav_count BIGINT,rating VARCHAR,tag_string VARCHAR,is_deleted BOOLEAN); CREATE TABLE current_posts(post_id BIGINT,row_id BIGINT,asset_id VARCHAR);").unwrap();
    for (record, obs, post, sha) in [
        (11, 101, Some(10), 1),
        (12, 101, Some(10), 1),
        (21, 201, Some(20), 2),
        (31, 301, Some(30), 3),
        (41, 401, None, 4),
        (51, 501, Some(50), 5),
    ] {
        db.query(&format!(
            "INSERT INTO assets VALUES ('{}','{}',{},'{}')",
            hex(record),
            hex(obs),
            post.map(|n| n.to_string()).unwrap_or("NULL".into()),
            hex(sha)
        ))
        .unwrap();
    }
    for (id, post, width, score, rating, tags, deleted) in [
        (101, "10", 100, "20", "'g'", "'blue sky'", "false"),
        (102, "10", 2000, "0", "'e'", "'red'", "true"),
        (201, "20", 3000, "30", "'g'", "'blue'", "true"),
        (202, "20", 4000, "NULL", "''", "''", "false"),
        (301, "30", 1500, "10", "NULL", "NULL", "NULL"),
        (401, "NULL", 1600, "50", "'g'", "'blue'", "false"),
        (501, "50", 9000, "99", "'g'", "'blue'", "false"),
    ] {
        db.query(&format!("INSERT INTO observations VALUES ('{}',{id},{post},{width},2000,'png',{score},1,{rating},{tags},{deleted})",hex(id))).unwrap();
    }
    for (post, row, asset) in [(10, 102, 11), (20, 202, 21), (30, 301, 31)] {
        db.query(&format!(
            "INSERT INTO current_posts VALUES ({post},{row},'{}')",
            hex(asset)
        ))
        .unwrap();
    }
    drop(db);
    Fixture {
        _root: root,
        source,
        reader: QueryReader::new(dll),
    }
}
fn condition(field: &str, operator: QueryOperator, value: Option<QueryValue>) -> QueryCondition {
    QueryCondition {
        field: field.into(),
        operator,
        value,
    }
}
fn spec(f: &Fixture, conditions: Vec<QueryCondition>, rule: ObservationRule) -> QuerySpec {
    QuerySpec {
        version: 1,
        source_ids: vec![f.source.id.clone()],
        conditions,
        observation_rule: rule,
        order: QueryOrder::AssetKeyAsc,
        input_scope: None,
    }
    .normalize()
    .unwrap()
}
fn run(f: &Fixture, spec: &QuerySpec) -> BTreeSet<String> {
    let version = f.reader.query_version(&f.source, spec).unwrap();
    let mut ids = BTreeSet::new();
    f.reader
        .execute_query(
            &f.source,
            spec,
            &version,
            Arc::new(AtomicBool::new(false)),
            &mut |keys, _| {
                assert!(keys.len() <= 512);
                ids.extend(keys.iter().map(|k| k.asset_id.clone()));
                Ok(())
            },
        )
        .unwrap();
    ids
}

fn enable_commit_history(f: &Fixture) {
    let root = f
        .source
        .index_root
        .as_ref()
        .unwrap()
        .join("indexes/gen-query");
    let dll = crate::duckdb::bundled_library();
    let db = Session::fixture(&dll, &root.join("analysis.duckdb")).unwrap();
    db.query("ALTER TABLE applied ADD COLUMN batch_id VARCHAR DEFAULT 'first'; ALTER TABLE assets ADD COLUMN commit_seq BIGINT DEFAULT 1; ALTER TABLE observations ADD COLUMN commit_seq BIGINT DEFAULT 1; CREATE TABLE objects(sha256 VARCHAR,pack_path VARCHAR,length BIGINT,stored_ext VARCHAR);").unwrap();
    for n in [1, 2, 3, 4, 6] {
        db.query(&format!(
            "INSERT INTO objects VALUES ('{}','segments/first/images.tar',{},'webp')",
            hex(n),
            n * 100
        ))
        .unwrap();
    }
}

#[test]
fn shared_rating_candidates_preserve_same_observation_and_incremental_changes() {
    let f = fixture();
    enable_commit_history(&f);
    let dll = crate::duckdb::bundled_library();
    let generation = f
        .source
        .index_root
        .as_ref()
        .unwrap()
        .join("indexes/gen-query");
    {
        let db = Session::fixture(&dll, &generation.join("analysis.duckdb")).unwrap();
        db.query(&format!("INSERT INTO observations(observation_id,row_id,post_id,rating,tag_string,commit_seq) VALUES ('{}',900,900,'g','only_g literal space namespace:one quote''tag 東京',1); INSERT INTO assets VALUES('{}','{}',900,'{}',1); INSERT INTO current_posts VALUES(900,900,'{}');",hex(900),hex(901),hex(900),hex(1),hex(901))).unwrap();
    }
    let root = tempfile::tempdir().unwrap();
    let cache = Arc::new(crate::RatingCache::new(root.path().join("bases")));
    let cancelled = || Arc::new(AtomicBool::new(false));
    for rating in crate::RATINGS {
        cache
            .ensure(&f.source, rating, 1 << 30, root.path(), cancelled())
            .unwrap();
    }
    assert_eq!(cache.entries().unwrap().len(), 4);
    let mut other_project_source = f.source.clone();
    other_project_source.name = "另一个项目的同一数据湖".into();
    cache
        .ensure(
            &other_project_source,
            "g",
            1 << 30,
            root.path(),
            cancelled(),
        )
        .unwrap();
    assert_eq!(
        cache.entries().unwrap().len(),
        4,
        "A second project shares the existing library basis"
    );
    let reader = QueryReader::new(dll.clone()).with_rating_cache(cache.clone());
    let evaluate = |query: &QuerySpec| {
        let version = reader.query_version(&f.source, query).unwrap();
        let mut ids = BTreeSet::new();
        reader
            .execute_query(&f.source, query, &version, cancelled(), &mut |keys, _| {
                ids.extend(keys.iter().map(|k| k.asset_id.clone()));
                Ok(())
            })
            .unwrap();
        ids
    };
    let query = |rating: &str, tag: &str| {
        spec(
            &f,
            vec![
                condition(
                    "rating",
                    QueryOperator::Eq,
                    Some(QueryValue::Text(rating.into())),
                ),
                condition(
                    "tags",
                    QueryOperator::HasTag,
                    Some(QueryValue::Text(tag.into())),
                ),
            ],
            ObservationRule::CurrentPost,
        )
    };
    assert_eq!(evaluate(&query("g", "only_g")), BTreeSet::from([hex(1)]));
    for rating in ["g", "e"] {
        for (operator, value) in [
            (
                QueryOperator::HasAllTags,
                Some(QueryValue::TextList(vec![
                    "namespace:one".into(),
                    "quote'tag".into(),
                    "東京".into(),
                ])),
            ),
            (
                QueryOperator::HasAnyTags,
                Some(QueryValue::TextList(vec!["only_g".into(), "solo".into()])),
            ),
            (
                QueryOperator::HasNoTags,
                Some(QueryValue::TextList(vec!["only_g".into(), "solo".into()])),
            ),
            (QueryOperator::IsMissing, None),
            (QueryOperator::IsPresent, None),
        ] {
            let combined = QuerySpec {
                version: 2,
                conditions: vec![
                    condition(
                        "rating",
                        QueryOperator::Eq,
                        Some(QueryValue::Text(rating.into())),
                    ),
                    condition("tags", operator, value),
                ],
                ..spec(&f, vec![], ObservationRule::CurrentPost)
            };
            assert_eq!(
                evaluate(&combined),
                run(&f, &combined),
                "Cached Tag operations retain source SQL semantics, including NULLs and token boundaries"
            );
        }
    }
    let pure = spec(
        &f,
        vec![condition(
            "rating",
            QueryOperator::Eq,
            Some(QueryValue::Text("g".into())),
        )],
        ObservationRule::CurrentPost,
    );
    assert_eq!(
        evaluate(&pure),
        run(&f, &pure),
        "Pure ratings use the cached asset identities directly"
    );
    let mut identity = pure.clone();
    identity.conditions.push(condition(
        "asset.id",
        QueryOperator::Eq,
        Some(QueryValue::Text(hex(1))),
    ));
    assert_eq!(
        evaluate(&identity),
        BTreeSet::from([hex(1)]),
        "Binary cached identities retain text identity predicate semantics"
    );
    assert!(
        evaluate(&query("e", "only_g")).is_empty(),
        "Tags on a different post for the same image cannot satisfy the conjunction"
    );
    assert!(
        reader.rating_usage().unwrap().1 > 0,
        "The native query imported the saved candidate rows"
    );
    advance_day(&f);
    let previous = cache
        .entries()
        .unwrap()
        .into_iter()
        .find(|entry| entry.rating == "e")
        .unwrap();
    assert_eq!(
        cache
            .ensure(
                &f.source,
                "e",
                1 << 30,
                root.path(),
                Arc::new(AtomicBool::new(true))
            )
            .unwrap_err()
            .code,
        "CANCELLED"
    );
    let retained = cache
        .entries()
        .unwrap()
        .into_iter()
        .find(|entry| entry.rating == "e")
        .unwrap();
    assert_eq!(
        (retained.sequence, retained.records),
        (previous.sequence, previous.records)
    );
    let refreshed = cache
        .ensure(&f.source, "e", 1 << 30, root.path(), cancelled())
        .unwrap();
    assert!(refreshed.incremental);
    assert_eq!(refreshed.sequence, 2);
    assert_eq!(
        evaluate(&query("e", "daily")),
        run(&f, &query("e", "daily"))
    );
    {
        let db = Session::fixture(&dll, &generation.join("analysis.duckdb")).unwrap();
        db.query("DELETE FROM applied WHERE seq=2; INSERT INTO applied VALUES(3,'third')")
            .unwrap();
        let catalog = rusqlite::Connection::open(generation.join("catalog.sqlite")).unwrap();
        catalog
            .execute("UPDATE state SET value=3 WHERE key='seq'", [])
            .unwrap();
    }
    let rebuilt = cache
        .ensure(&f.source, "e", 1 << 30, root.path(), cancelled())
        .unwrap();
    assert!(
        !rebuilt.incremental,
        "Missing anchor history requires a complete basis rebuild"
    );
    assert_eq!(
        evaluate(&query("e", "daily")),
        run(&f, &query("e", "daily"))
    );
    cache.set_fixed(&f.source.id, "e", true).unwrap();
    assert!(
        !cache.remove(&f.source.id, "e", false).unwrap(),
        "Explicitly fixed bases cannot be reclaimed"
    );
    cache.set_fixed(&f.source.id, "e", false).unwrap();
    let pin = cache.pin(&f.source.id, &["e".into()]).unwrap();
    assert!(!cache.remove(&f.source.id, "e", false).unwrap());
    drop(pin);
    assert!(cache.remove(&f.source.id, "e", false).unwrap());
}
fn advance_day(f: &Fixture) {
    let root = f
        .source
        .index_root
        .as_ref()
        .unwrap()
        .join("indexes/gen-query");
    let dll = crate::duckdb::bundled_library();
    let db = Session::fixture(&dll, &root.join("analysis.duckdb")).unwrap();
    db.query("BEGIN;").unwrap();
    for (row, post, rating) in [
        (103, "10", "g"),
        (203, "20", "e"),
        (503, "5", "g"),
        (402, "NULL", "e"),
    ] {
        db.query(&format!("INSERT INTO observations VALUES ('{}',{row},{post},1000,1000,'png',0,0,'{rating}','daily',false,2)",hex(row))).unwrap();
    }
    for (record, obs, post, sha) in [(22, 203, "20", 7), (15, 503, "5", 1), (42, 402, "NULL", 9)] {
        db.query(&format!(
            "INSERT INTO assets VALUES ('{}','{}',{post},'{}',2)",
            hex(record),
            hex(obs),
            hex(sha)
        ))
        .unwrap();
    }
    db.query("DELETE FROM current_posts WHERE post_id IN (10,20)")
        .unwrap();
    for (post, row, record) in [(10, 103, 11), (20, 203, 22), (5, 503, 15)] {
        db.query(&format!(
            "INSERT INTO current_posts VALUES ({post},{row},'{}')",
            hex(record)
        ))
        .unwrap();
    }
    for n in [7, 8, 9] {
        db.query(&format!(
            "INSERT INTO objects VALUES ('{}','segments/second/images.tar',{},'webp')",
            hex(n),
            n * 100
        ))
        .unwrap();
    }
    db.query("INSERT INTO applied VALUES(2,'second'); COMMIT;")
        .unwrap();
    drop(db);
    let cat = rusqlite::Connection::open(root.join("catalog.sqlite")).unwrap();
    for n in [7, 8, 9] {
        cat.execute(
            "INSERT INTO objects VALUES(?1,?2,'webp')",
            rusqlite::params![hex(n), (n * 100) as i64],
        )
        .unwrap();
    }
    cat.execute("UPDATE state SET value=2 WHERE key='seq'", [])
        .unwrap();
}

#[test]
fn incremental_changes_cover_old_posts_relinked_images_and_orphan_objects() {
    let f = fixture();
    enable_commit_history(&f);
    let root = tempfile::tempdir().unwrap();
    let index = crate::BrowseIndex::new(root.path().join("browse"));
    let cancel = || Arc::new(AtomicBool::new(false));
    let first = index
        .ensure(&f.source, 1 << 30, root.path(), cancel())
        .unwrap();
    assert_eq!(first.count, 5);
    assert!(!first.incremental);
    let old = index.anchor(&f.source.id, 1).unwrap().unwrap();
    let s = spec(
        &f,
        vec![condition(
            "rating",
            QueryOperator::Eq,
            Some(QueryValue::Text("e".into())),
        )],
        ObservationRule::CurrentPost,
    );
    assert_eq!(run(&f, &s), BTreeSet::from([hex(1)]));
    advance_day(&f);
    let second = index
        .ensure(&f.source, 1 << 30, root.path(), cancel())
        .unwrap();
    assert!(second.incremental);
    assert_eq!(second.count, 8);
    assert_eq!(second.refreshed_objects, 5);
    assert_eq!(
        index
            .post_ids(
                &f.source,
                &[AssetKey {
                    source_id: f.source.id.clone(),
                    asset_id: hex(1)
                }]
            )
            .unwrap(),
        vec![Some(5)]
    );
    for (order, expected) in [
        (QueryOrder::PostIdAsc, vec![1, 2, 7, 3, 4, 6, 8, 9]),
        (QueryOrder::PostIdDesc, vec![3, 7, 2, 1, 9, 8, 6, 4]),
    ] {
        let read = index.reader(&f.source).unwrap();
        let mut found = Vec::new();
        let mut after = None;
        loop {
            let rows = read.page(&f.source.id, order, after.as_deref(), 2).unwrap();
            if rows.is_empty() {
                break;
            }
            after = rows.last().map(|(k, _)| k.asset_id.clone());
            found.extend(rows.into_iter().map(|(k, _)| k.asset_id));
        }
        assert_eq!(found, expected.into_iter().map(hex).collect::<Vec<_>>());
    }
    let mut affected = BTreeSet::new();
    let mut matches = BTreeSet::new();
    let version = f.reader.query_version(&f.source, &s).unwrap();
    assert!(
        f.reader
            .execute_delta(
                &f.source,
                &s,
                &version,
                &old,
                cancel(),
                &mut |keys| {
                    affected.extend(keys.iter().map(|k| k.asset_id.clone()));
                    Ok(())
                },
                &mut |keys, _| {
                    matches.extend(keys.iter().map(|k| k.asset_id.clone()));
                    Ok(())
                }
            )
            .unwrap()
    );
    assert_eq!(
        affected,
        BTreeSet::from([hex(1), hex(2), hex(7), hex(8), hex(9)])
    );
    assert_eq!(matches, run(&f, &s));
    assert_eq!(matches, BTreeSet::from([hex(7)]));
}

#[test]
fn incremental_history_requires_continuity_and_the_original_commit_anchor() {
    let f = fixture();
    enable_commit_history(&f);
    advance_day(&f);
    let catalog = Catalog::open(&f.source).unwrap();
    let db = f
        .reader
        .runtime
        .open(&catalog.analysis_path().unwrap())
        .unwrap();
    let good = ChangeAnchor {
        generation: "gen-query".into(),
        sequence: 1,
        batch_id: "first".into(),
    };
    assert!(changes::change_sql(&db, &catalog, &good).unwrap().is_some());
    assert!(
        changes::change_sql(
            &db,
            &catalog,
            &ChangeAnchor {
                batch_id: "replaced".into(),
                ..good.clone()
            }
        )
        .unwrap()
        .is_none()
    );
    assert!(
        changes::change_sql(
            &db,
            &catalog,
            &ChangeAnchor {
                generation: "other-generation".into(),
                ..good.clone()
            }
        )
        .unwrap()
        .is_none()
    );
    drop(db);
    drop(catalog);
    let root = f
        .source
        .index_root
        .as_ref()
        .unwrap()
        .join("indexes/gen-query");
    let dll = crate::duckdb::bundled_library();
    let db = Session::fixture(&dll, &root.join("analysis.duckdb")).unwrap();
    db.query("DELETE FROM applied WHERE seq=2; INSERT INTO applied VALUES(3,'third')")
        .unwrap();
    drop(db);
    let cat = rusqlite::Connection::open(root.join("catalog.sqlite")).unwrap();
    cat.execute("UPDATE state SET value=3 WHERE key='seq'", [])
        .unwrap();
    drop(cat);
    let catalog = Catalog::open(&f.source).unwrap();
    let db = f
        .reader
        .runtime
        .open(&catalog.analysis_path().unwrap())
        .unwrap();
    assert!(changes::change_sql(&db, &catalog, &good).unwrap().is_none());
}

#[test]
fn rating_sets_and_tag_all_any_none_keep_observation_and_null_semantics() {
    let f = fixture();
    let list = |field: &str, operator, values: &[&str]| {
        condition(
            field,
            operator,
            Some(QueryValue::TextList(
                values.iter().map(|s| (*s).to_owned()).collect(),
            )),
        )
    };
    let extended = |conditions, rule| {
        let mut s = spec(&f, vec![], rule);
        s.version = 2;
        s.conditions = conditions;
        s.normalize().unwrap()
    };
    let expected = |numbers: &[u64]| numbers.iter().map(|n| hex(*n)).collect::<BTreeSet<_>>();
    assert_eq!(
        run(
            &f,
            &extended(
                vec![list("rating", QueryOperator::In, &["g", "e", "g"])],
                ObservationRule::CurrentPost
            )
        ),
        expected(&[1])
    );
    assert_eq!(
        run(
            &f,
            &extended(
                vec![list("rating", QueryOperator::In, &["g", "e"])],
                ObservationRule::AnyObservation
            )
        ),
        expected(&[1, 2, 4])
    );
    assert_eq!(
        run(
            &f,
            &extended(
                vec![list("tags", QueryOperator::HasAllTags, &["blue", "sky"])],
                ObservationRule::AnyObservation
            )
        ),
        expected(&[1])
    );
    assert_eq!(
        run(
            &f,
            &extended(
                vec![list("tags", QueryOperator::HasAnyTags, &["red", "sky"])],
                ObservationRule::AnyObservation
            )
        ),
        expected(&[1])
    );
    assert_eq!(
        run(
            &f,
            &extended(
                vec![list("tags", QueryOperator::HasNoTags, &["blue", "red"])],
                ObservationRule::CurrentPost
            )
        ),
        expected(&[2])
    );
    // An e observation and a separate blue/sky observation may not be combined.
    assert!(
        run(
            &f,
            &extended(
                vec![
                    list("rating", QueryOperator::In, &["e"]),
                    list("tags", QueryOperator::HasAllTags, &["blue", "sky"])
                ],
                ObservationRule::AnyObservation
            )
        )
        .is_empty()
    );
    assert_eq!(
        run(
            &f,
            &extended(
                vec![
                    list("rating", QueryOperator::In, &["g", "e"]),
                    list("tags", QueryOperator::HasAllTags, &["blue", "sky"]),
                    list("tags", QueryOperator::HasNoTags, &["red"])
                ],
                ObservationRule::AnyObservation
            )
        ),
        expected(&[1])
    );
    assert!(
        run(
            &f,
            &extended(
                vec![
                    list("tags", QueryOperator::HasAllTags, &["blue"]),
                    list("tags", QueryOperator::HasNoTags, &["blue"])
                ],
                ObservationRule::AnyObservation
            )
        )
        .is_empty()
    );
    assert!(
        run(
            &f,
            &extended(
                vec![list("tags", QueryOperator::HasAnyTags, &["red');--"])],
                ObservationRule::AnyObservation
            )
        )
        .is_empty()
    );
    let mut invalid = extended(
        vec![list("rating", QueryOperator::In, &["e"])],
        ObservationRule::CurrentPost,
    );
    invalid.version = 1;
    assert_eq!(
        invalid.normalize().unwrap_err().code,
        "QUERY_VERSION_UNSUPPORTED"
    );
    let invalid = extended(
        vec![list("tags", QueryOperator::HasAllTags, &["not one tag"])],
        ObservationRule::CurrentPost,
    );
    assert_eq!(
        f.reader
            .fields(&f.source)
            .unwrap()
            .validate(&invalid)
            .unwrap_err()
            .code,
        "INVALID_INPUT"
    );
}

#[test]
fn bounded_project_input_predicates_only_return_requested_assets() {
    let f = fixture();
    let mut s = spec(&f, vec![], ObservationRule::AnyObservation);
    s.version = 2;
    s.conditions = vec![condition(
        "tags",
        QueryOperator::HasAnyTags,
        Some(QueryValue::TextList(vec!["blue".into()])),
    )];
    let version = f.reader.query_version(&f.source, &s).unwrap();
    let keys = vec![
        AssetKey {
            source_id: f.source.id.clone(),
            asset_id: hex(2),
        },
        AssetKey {
            source_id: f.source.id.clone(),
            asset_id: hex(3),
        },
    ];
    let mut actual = BTreeSet::new();
    f.reader
        .execute_query_keys(
            &f.source,
            &s,
            &version,
            Arc::new(AtomicBool::new(false)),
            &keys,
            &mut |keys, _| {
                actual.extend(keys.iter().map(|k| k.asset_id.clone()));
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(actual, BTreeSet::from([hex(2)]));
}
#[test]
fn conjunction_uses_one_observation_and_rules_have_distinct_members() {
    let f = fixture();
    let conditions = vec![
        condition(
            "score",
            QueryOperator::Gte,
            Some(QueryValue::Integer("10".into())),
        ),
        condition(
            "source.width",
            QueryOperator::Gte,
            Some(QueryValue::Integer("1000".into())),
        ),
    ];
    assert_eq!(
        run(
            &f,
            &spec(&f, conditions.clone(), ObservationRule::AnyObservation)
        ),
        [hex(2), hex(3), hex(4)].into()
    );
    assert_eq!(
        run(&f, &spec(&f, conditions, ObservationRule::CurrentPost)),
        [hex(3)].into()
    );
    let tag = spec(
        &f,
        vec![condition(
            "tags",
            QueryOperator::HasTag,
            Some(QueryValue::Text("blue".into())),
        )],
        ObservationRule::AnyObservation,
    );
    assert_eq!(run(&f, &tag), [hex(1), hex(2), hex(4)].into());
}
#[test]
fn null_empty_false_and_unsupported_are_not_conflated() {
    let f = fixture();
    let rule = ObservationRule::CurrentPost;
    for (c, expected) in [
        (
            condition(
                "rating",
                QueryOperator::Eq,
                Some(QueryValue::Text("".into())),
            ),
            vec![hex(2)],
        ),
        (
            condition("tags", QueryOperator::IsPresent, None),
            vec![hex(1), hex(2)],
        ),
        (
            condition("tags", QueryOperator::IsMissing, None),
            vec![hex(3)],
        ),
        (
            condition(
                "is_deleted",
                QueryOperator::Eq,
                Some(QueryValue::Boolean(false)),
            ),
            vec![hex(2)],
        ),
        (
            condition(
                "rating",
                QueryOperator::Ne,
                Some(QueryValue::Text("g".into())),
            ),
            vec![hex(1), hex(2)],
        ),
        (
            condition(
                "rating",
                QueryOperator::Eq,
                Some(QueryValue::Text("' OR true --".into())),
            ),
            vec![],
        ),
        (
            condition("stored.extension", QueryOperator::IsMissing, None),
            vec![hex(6)],
        ),
    ] {
        assert_eq!(
            run(&f, &spec(&f, vec![c], rule)),
            expected.into_iter().collect()
        );
    }
    for c in [
        condition("stored.width", QueryOperator::IsMissing, None),
        condition("unknown", QueryOperator::IsPresent, None),
    ] {
        assert_eq!(
            f.reader
                .query_version(&f.source, &spec(&f, vec![c], rule))
                .unwrap_err()
                .code,
            "QUERY_UNSUPPORTED"
        );
    }
    let bad = spec(
        &f,
        vec![condition(
            "score",
            QueryOperator::Gte,
            Some(QueryValue::Text("1".into())),
        )],
        rule,
    );
    assert_eq!(
        f.reader.query_version(&f.source, &bad).unwrap_err().code,
        "INVALID_INPUT"
    );
}
#[test]
fn changed_versions_and_cancelled_builds_are_rejected() {
    let f = fixture();
    let q = spec(&f, vec![], ObservationRule::AnyObservation);
    let mut version = f.reader.query_version(&f.source, &q).unwrap();
    let mut sink = |_: &[AssetKey], _: u64| Ok(());
    assert_eq!(
        f.reader
            .execute_query(
                &f.source,
                &q,
                &version,
                Arc::new(AtomicBool::new(true)),
                &mut sink
            )
            .unwrap_err()
            .code,
        "CANCELLED"
    );
    version.catalog_revision.push_str("changed");
    assert_eq!(
        f.reader
            .execute_query(
                &f.source,
                &q,
                &version,
                Arc::new(AtomicBool::new(false)),
                &mut sink
            )
            .unwrap_err()
            .code,
        "SOURCE_CHANGED"
    );
}
