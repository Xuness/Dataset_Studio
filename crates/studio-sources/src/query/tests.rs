use super::*;
use std::{collections::BTreeSet, fs, path::Path};

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
    let dll = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/duckdb/duckdb.dll");
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
