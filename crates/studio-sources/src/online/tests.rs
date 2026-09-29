use super::*;
use crate::{MetadataReader, QueryReader, SourceRouter};
use std::io::Write;
use studio_application::{MetadataAdapter, QueryAdapter, SourceAdapter};

fn sha(n: u32) -> String {
    format!("{n:064x}")
}
struct Fixture {
    source: Source,
    root: tempfile::TempDir,
    path: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-runs");
        fs::create_dir_all(&base).unwrap();
        let root = tempfile::Builder::new()
            .prefix("online-reader-")
            .tempdir_in(base)
            .unwrap();
        let index = root.path().join("index");
        let media = root.path().join("media");
        fs::create_dir_all(&index).unwrap();
        fs::create_dir_all(&media).unwrap();
        let id = new_id();
        let source = Source {
            id: id.clone(),
            name: "online fixture".into(),
            kind: "danbooru".into(),
            index_root: Some(index.clone()),
            media_root: Some(media.clone()),
        };
        fs::write(media.join("library.json"),serde_json::to_vec(&serde_json::json!({"library_id":id,"format_version":1,"image_format":"uncompressed-pax-tar"})).unwrap()).unwrap();
        fs::write(index.join("ONLINE.json"),serde_json::to_vec(&serde_json::json!({"library_id":id,"schema_version":2,"generation":"online-fixture","file":"online.sqlite","site":"danbooru"})).unwrap()).unwrap();
        let path = index.join("online.sqlite");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        db.execute_batch(SCHEMA).unwrap();
        for (key, value) in [
            ("library_id", id.as_str()),
            ("generation", "online-fixture"),
            ("served_seq", "1"),
            ("min_seq", "1"),
            ("base_seq", "1"),
            ("archive_seq", "1"),
            ("analysis_seq", "1"),
        ] {
            db.execute(
                "INSERT INTO online_state VALUES(?1,?2)",
                params![key, value],
            )
            .unwrap();
        }
        db.execute(
            "INSERT INTO publications VALUES(1,'baseline','now',2,3,3)",
            [],
        )
        .unwrap();
        for n in 1..=2 {
            db.execute(
                "INSERT INTO objects VALUES(?1,?2,'segments/base/images.tar',0,20,'png',1)",
                params![n, sha(n)],
            )
            .unwrap();
            db.execute(
                "INSERT INTO object_versions VALUES(?1,1,NULL,?2)",
                params![sha(n), n],
            )
            .unwrap();
        }
        for (n, object, rating, tag) in [
            (1, 1, "g", "tag_x"),
            (2, 1, "e", "tag_y"),
            (3, 2, "g", "Odd\nTag"),
        ] {
            db.execute("INSERT INTO observations(row_id,observation_id,post_id,rating,score,tag_string,commit_seq,created_at,is_deleted) VALUES(?1,?2,?1,?3,10,?4,1,'2020-01-01 00:00:00+00:00',0)",params![n,sha(n+100),rating,tag]).unwrap();
            db.execute("INSERT INTO assets(asset_row,asset_id,observation_id,post_id,sha256,stored_ext,stored_bytes,commit_seq) VALUES(?1,?2,?3,?1,?4,'png',20,1)",params![n,sha(n+200),sha(n+100),sha(object)]).unwrap();
            db.execute(
                "INSERT INTO post_versions VALUES(?1,1,NULL,?1,?2)",
                params![n, sha(n + 200)],
            )
            .unwrap();
            db.execute("INSERT INTO tags VALUES(?1,?2)", params![n, tag])
                .unwrap();
            db.execute(
                "INSERT INTO tag_index(rowid,tokens) VALUES(?1,?2)",
                params![n, format!("t{n:x}")],
            )
            .unwrap();
        }
        let raw = b"{\"unknown\":123456789012345678901,\"null\":null}";
        let mut compressed =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        compressed.write_all(raw).unwrap();
        use sha2::{Digest, Sha256};
        db.execute("INSERT INTO raw_metadata(observation_id,raw_bytes,raw_sha256,raw_zlib,source_metadata_format) VALUES(?1,?2,?3,?4,'api-json-original-object/v1')",params![sha(101),raw.len() as i64,hex::encode(Sha256::digest(raw)),compressed.finish().unwrap()]).unwrap();
        drop(db);
        Self { source, root, path }
    }
    fn spec(&self, conditions: Vec<QueryCondition>) -> QuerySpec {
        QuerySpec {
            version: 3,
            source_ids: vec![self.source.id.clone()],
            conditions,
            observation_rule: ObservationRule::CurrentPost,
            order: QueryOrder::PostIdAsc,
            input_scope: None,
        }
    }
}
fn tags(values: &[&str]) -> QueryCondition {
    QueryCondition {
        field: "tags".into(),
        operator: QueryOperator::HasAllTags,
        value: Some(QueryValue::TextList(
            values.iter().map(|v| (*v).into()).collect(),
        )),
    }
}
fn read(f: &Fixture, spec: &QuerySpec, version: &QuerySourceVersion) -> SourceQueryPage {
    QueryReader::default()
        .query_page(
            &f.source,
            spec,
            version,
            None,
            20,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap()
}

#[test]
fn lease_write_wait_observes_request_deadline() {
    let f = Fixture::new();
    let version = QueryReader::default()
        .read_version(&f.source, true)
        .unwrap();
    let db = Connection::open(&f.path).unwrap();
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    let started = Instant::now();
    let query = QueryReader::default().with_deadline(Some(started + Duration::from_millis(90)));
    let error = query
        .retain_version(&f.source, &version, "deadline-test", "test", false)
        .unwrap_err();
    assert_eq!(error.code, "SOURCE_TIMEOUT");
    assert!(started.elapsed() < Duration::from_secs(1));
    db.execute_batch("ROLLBACK").unwrap();
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM leases WHERE id='deadline-test'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn lease_write_retries_wal_protocol_contention_but_not_corruption() {
    let f = Fixture::new();
    let cancelled = AtomicBool::new(false);
    for (code, succeeds, expected_attempts) in [
        (rusqlite::ffi::SQLITE_PROTOCOL, true, 2),
        (rusqlite::ffi::SQLITE_CORRUPT, false, 1),
    ] {
        let mut attempts = 0;
        let result = lease_write(
            &f.path,
            &cancelled,
            Instant::now() + Duration::from_secs(1),
            |db| {
                attempts += 1;
                if attempts == 1 {
                    return Err(sql_error(rusqlite::Error::SqliteFailure(
                        rusqlite::ffi::Error::new(code),
                        None,
                    )));
                }
                db.execute("DELETE FROM leases WHERE id='contention-probe'", [])
                    .map_err(sql_error)
            },
        );
        assert_eq!(result.is_ok(), succeeds);
        assert_eq!(attempts, expected_attempts);
        if let Err(error) = result {
            assert_eq!(error.code, "SOURCE_FORMAT_ERROR");
        }
    }
}

#[test]
fn lease_write_wait_observes_cancellation_and_release_uses_its_context() {
    let f = Fixture::new();
    let version = QueryReader::default()
        .read_version(&f.source, true)
        .unwrap();
    let db = Connection::open(&f.path).unwrap();
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    let flag = Arc::new(AtomicBool::new(false));
    let started = Instant::now();
    let thread = {
        let flag = flag.clone();
        let source = f.source.clone();
        std::thread::spawn(move || {
            QueryReader::default()
                .with_cancellation(flag)
                .retain_version(&source, &version, "cancel-test", "test", false)
        })
    };
    std::thread::sleep(Duration::from_millis(60));
    flag.store(true, Ordering::Release);
    assert_eq!(thread.join().unwrap().unwrap_err().code, "CANCELLED");
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(
        QueryReader::default()
            .with_cancellation(flag)
            .release_version(&f.source, "cancel-test")
            .unwrap_err()
            .code,
        "CANCELLED"
    );
    db.execute_batch("ROLLBACK").unwrap();
    QueryReader::default()
        .release_version(&f.source, "cancel-test")
        .unwrap();
}

#[test]
fn indexed_tags_keep_literal_and_same_observation_semantics() {
    let f = Fixture::new();
    let q = QueryReader::default();
    let v = q.read_version(&f.source, true).unwrap();
    assert!(
        read(&f, &f.spec(vec![tags(&["tag_x", "tag_y"])]), &v)
            .hits
            .is_empty()
    );
    assert!(
        read(
            &f,
            &f.spec(vec![
                tags(&["tag_x"]),
                QueryCondition {
                    field: "rating".into(),
                    operator: QueryOperator::Eq,
                    value: Some(QueryValue::Text("e".into()))
                }
            ]),
            &v
        )
        .hits
        .is_empty()
    );
    assert_eq!(
        read(&f, &f.spec(vec![tags(&["Odd\nTag"])]), &v).hits[0]
            .key
            .asset_id,
        sha(2)
    );
    let unknown = read(&f, &f.spec(vec![tags(&["odd\nTag"])]), &v);
    assert!(unknown.hits.is_empty());
    assert!(unknown.next.is_none());
    assert_eq!(unknown.scanned, 0);
    let mut historical = f.spec(vec![tags(&["tag_x", "tag_y"])]);
    historical.observation_rule = ObservationRule::AnyObservation;
    assert!(read(&f, &historical, &v).hits.is_empty());
}

#[test]
fn deep_pages_and_empty_filter_pages_have_bounded_work() {
    use std::sync::atomic::AtomicUsize;
    let f = Fixture::new();
    let mut db = Connection::open(&f.path).unwrap();
    let tx = db.transaction().unwrap();
    tx.execute_batch("WITH RECURSIVE n(x) AS(VALUES(3) UNION ALL SELECT x+1 FROM n WHERE x<100000) INSERT INTO objects SELECT x,printf('%064x',x),'packs/images.tar',0,20,'png',1 FROM n; INSERT INTO object_versions SELECT sha256,1,NULL,object_row FROM objects WHERE object_row>=3; UPDATE publications SET objects_count=100000 WHERE seq=1;").unwrap();
    tx.commit().unwrap();
    db.execute_batch("ANALYZE").unwrap();
    let snapshot = Snapshot::latest(&f.source).unwrap();
    let ticks = Arc::new(AtomicUsize::new(0));
    let counter = ticks.clone();
    snapshot
        .db
        .progress_handler(
            1000,
            Some(move || {
                counter.fetch_add(1, Ordering::Relaxed);
                false
            }),
        )
        .unwrap();
    let page = snapshot
        .page_query(&f.source, &f.spec(vec![]), Some(&sha(99000)), 16)
        .unwrap();
    assert_eq!(page.hits[0].key.asset_id, sha(99001));
    assert_eq!(page.hits.len(), 16);
    assert!(
        ticks.load(Ordering::Relaxed) < 100,
        "deep seek scanned the preceding lake"
    );
    ticks.store(0, Ordering::Relaxed);
    let mut spec = f.spec(vec![QueryCondition {
        field: "stored.extension".into(),
        operator: QueryOperator::Eq,
        value: Some(QueryValue::Text("jpg".into())),
    }]);
    spec.order = QueryOrder::AssetKeyDesc;
    let page = snapshot.page_query(&f.source, &spec, None, 16).unwrap();
    assert!(page.hits.is_empty());
    assert_eq!(page.scanned, 2048);
    assert!(page.next.is_some());
    assert!(
        ticks.load(Ordering::Relaxed) < 400,
        "empty filter scanned beyond its candidate budget"
    );
}

#[test]
fn missing_active_pointer_cannot_fall_back_to_native_indexes() {
    let f = Fixture::new();
    let root = f.source.index_root.as_ref().unwrap();
    fs::write(root.join("ONLINE-BUILD.json"), r#"{"state":"active"}"#).unwrap();
    fs::remove_file(root.join("ONLINE.json")).unwrap();
    assert!(available(&f.source));
    assert!(
        QueryReader::default()
            .read_version(&f.source, true)
            .is_err()
    );
}

#[test]
fn publication_keeps_old_pages_metadata_and_raw_available_without_native_duckdb() {
    let f = Fixture::new();
    let q = QueryReader::default();
    let old = q.read_version(&f.source, true).unwrap();
    let first = q
        .query_page(
            &f.source,
            &f.spec(vec![]),
            &old,
            None,
            1,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
    assert_eq!(first.hits[0].key.asset_id, sha(1));
    let db = Connection::open(&f.path).unwrap();
    db.execute("INSERT INTO observations(row_id,observation_id,post_id,rating,score,tag_string,commit_seq) VALUES(4,?1,1,'e',99,'tag_y',2)",[sha(104)]).unwrap();
    db.execute("INSERT INTO tag_index(rowid,tokens) VALUES(4,'t2')", [])
        .unwrap();
    db.execute_batch("UPDATE post_versions SET valid_until=2 WHERE post_id=1 AND valid_from=1;")
        .unwrap();
    db.execute(
        "INSERT INTO post_versions VALUES(1,2,NULL,4,?1)",
        [sha(201)],
    )
    .unwrap();
    assert_eq!(
        q.read_version(&f.source, true).unwrap(),
        old,
        "staging cannot advance readers"
    );
    assert_eq!(
        read(&f, &f.spec(vec![tags(&["tag_x"])]), &old).hits.len(),
        1
    );
    db.execute_batch("BEGIN; UPDATE online_state SET value='2' WHERE key='served_seq'; INSERT INTO publications VALUES(2,'update','now',2,4,3); COMMIT;").unwrap();
    let latest = q.read_version(&f.source, true).unwrap();
    assert_ne!(latest, old);
    assert!(
        read(&f, &f.spec(vec![tags(&["tag_x"])]), &latest)
            .hits
            .is_empty()
    );
    assert_eq!(
        read(&f, &f.spec(vec![tags(&["tag_x"])]), &old).hits.len(),
        1
    );
    let second = q
        .query_page(
            &f.source,
            &f.spec(vec![]),
            &old,
            first.next.as_deref(),
            1,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
    assert_eq!(second.hits[0].key.asset_id, sha(2));
    q.validate_version(&f.source, &old).unwrap();
    let token = metadata::expected_metadata(&f.source, &old.catalog_revision).unwrap();
    let reader = MetadataReader::default();
    let observations = reader
        .observations(
            &f.source,
            &sha(1),
            &sha(201),
            MetadataRequest {
                version: Some(token.clone()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(observations.items.len(), 1);
    let raw = reader
        .raw_metadata(&f.source, &sha(1), &sha(201), &sha(101), &token)
        .unwrap();
    assert_eq!(raw.status, "available");
    assert!(raw.json.unwrap().contains("123456789012345678901"));
    assert_eq!(
        SourceRouter
            .page(&f.source, None, 1, Some(&old.catalog_revision))
            .unwrap()
            .items
            .len(),
        1
    );
    drop(db);
    assert!(!f.root.path().join("analysis.duckdb").exists());
}
