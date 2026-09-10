use super::*;
use std::{fs, io::Write};

fn hex(n: u64) -> String {
    format!("{n:064x}")
}
struct Fixture {
    root: tempfile::TempDir,
    source: Source,
    reader: MetadataReader,
    path: PathBuf,
    dll: PathBuf,
}
fn fixture() -> Fixture {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-runs");
    fs::create_dir_all(&base).unwrap();
    let root = tempfile::Builder::new()
        .prefix("metadata-")
        .tempdir_in(base)
        .unwrap();
    let source = Source {
        id: new_id(),
        name: "只读夹具".into(),
        kind: "danbooru".into(),
        index_root: Some(root.path().into()),
        media_root: Some(root.path().into()),
    };
    let generation = root.path().join("indexes/gen-test");
    fs::create_dir_all(&generation).unwrap();
    fs::write(
        root.path().join("CURRENT.json"),
        serde_json::to_vec(
            &serde_json::json!({"library_id":source.id,"index_version":1,"generation":"gen-test"}),
        )
        .unwrap(),
    )
    .unwrap();
    fs::write(root.path().join("library.json"),serde_json::to_vec(&serde_json::json!({"library_id":source.id,"format_version":1,"image_format":"uncompressed-pax-tar"})).unwrap()).unwrap();
    let sq = rusqlite::Connection::open(generation.join("catalog.sqlite")).unwrap();
    sq.execute_batch("CREATE TABLE state(key TEXT PRIMARY KEY,value INTEGER); INSERT INTO state VALUES ('seq',1); CREATE TABLE objects(sha256 TEXT PRIMARY KEY,length INTEGER,stored_ext TEXT);").unwrap();
    for n in [100, 200, 300] {
        sq.execute("INSERT INTO objects VALUES (?1,123,'webp')", [hex(n)])
            .unwrap();
    }
    drop(sq);
    let dll = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/duckdb/duckdb.dll");
    let path = generation.join("analysis.duckdb");
    let db = Session::fixture(&dll, &path).unwrap();
    db.query("CREATE TABLE applied(seq BIGINT PRIMARY KEY); INSERT INTO applied VALUES (1); CREATE TABLE assets(asset_id VARCHAR PRIMARY KEY,observation_id VARCHAR,post_id BIGINT,sha256 VARCHAR,source_md5 VARCHAR,storage_profile VARCHAR); CREATE TABLE raw_metadata(observation_id VARCHAR PRIMARY KEY,source_metadata_json VARCHAR,source_metadata_format VARCHAR,source_schema_id VARCHAR)").unwrap();
    let extra = FIELDS
        .iter()
        .map(|(_, column, kind)| {
            format!(
                "{column} {}",
                match *kind {
                    "integer" => "BIGINT",
                    "boolean" => "BOOLEAN",
                    _ => "VARCHAR",
                }
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    db.query(&format!("CREATE TABLE observations(observation_id VARCHAR PRIMARY KEY,row_id BIGINT,post_id BIGINT,source_key VARCHAR,source_kind VARCHAR,observed_at TIMESTAMPTZ,time_quality VARCHAR,ingested_at TIMESTAMPTZ,commit_seq BIGINT,{extra}); CREATE INDEX obs_post ON observations(post_id);")).unwrap();
    for (id, origin, post, blob) in [
        (1, Some(10), Some(42), 100),
        (2, Some(10), Some(42), 100),
        (3, Some(30), Some(84), 100),
        (4, None, None, 100),
        (5, Some(50), Some(99), 200),
    ] {
        db.query(&format!(
            "INSERT INTO assets VALUES ({},{},{},{},NULL,'fixture')",
            quote(&hex(id)),
            origin.map(|n| quote(&hex(n))).unwrap_or("NULL".into()),
            post.map(|n| n.to_string()).unwrap_or("NULL".into()),
            quote(&hex(blob))
        ))
        .unwrap();
    }
    for (row, post) in (10..=24).map(|n| (n, 42)).chain([(30, 84), (50, 99)]) {
        db.query(&format!("INSERT INTO observations(observation_id,row_id,post_id,source_key,source_kind,observed_at,time_quality,ingested_at,commit_seq,rating,tag_string,image_width,image_height,score,is_deleted) VALUES ({},{row},{post},'fixture-key','fixture','2024-01-01 00:00:00+00','date_only','2026-01-01 00:00:00+00',1,'s','first second',2480,3507,-2,false)",quote(&hex(row)))).unwrap();
    }
    db.query(&format!("INSERT INTO raw_metadata VALUES ({},'{{\"answer\":42}}','fixture-json','schema-1'),({},repeat('中',50000),'fixture-json',NULL); UPDATE observations SET tag_string=repeat('x',9000),is_deleted=true WHERE row_id=11",quote(&hex(10)),quote(&hex(11)))).unwrap();
    drop(db);
    Fixture {
        root,
        source,
        reader: MetadataReader::new(dll.clone()),
        path,
        dll,
    }
}
fn req(version: &str) -> MetadataRequest {
    MetadataRequest {
        version: Some(version.into()),
        ..Default::default()
    }
}

#[test]
fn page_identity_summaries_are_batched_distinct_bounded_and_readonly() {
    let f = fixture();
    let db = Session::fixture(&f.dll, &f.path).unwrap();
    for n in 1000..1012 {
        db.query(&format!(
            "INSERT INTO assets VALUES ('{}',NULL,{n},'{}',NULL,'fixture')",
            hex(n),
            hex(200)
        ))
        .unwrap();
    }
    drop(db);
    let before = fs::metadata(&f.path).unwrap().modified().unwrap();
    let values = f
        .reader
        .summaries(
            &f.source,
            &[hex(100), hex(200), hex(300)],
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .unwrap();
    let many = values.iter().find(|v| v.asset_id == hex(200)).unwrap();
    assert!(many.post_count >= 12);
    assert_eq!(many.post_ids.len(), 8);
    let missing = values.iter().find(|v| v.asset_id == hex(300)).unwrap();
    assert_eq!(missing.post_count, 0);
    assert!(missing.post_ids.is_empty());
    let one = values.iter().find(|v| v.asset_id == hex(100)).unwrap();
    assert_eq!(
        one.post_ids
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        one.post_ids.len()
    );
    assert_eq!(fs::metadata(&f.path).unwrap().modified().unwrap(), before);
}

#[test]
fn origin_field_freezes_value_record_observation_and_version() {
    let f = fixture();
    let catalog = Catalog::open(&f.source).unwrap();
    let expected = QuerySourceVersion {
        source_id: f.source.id.clone(),
        catalog_revision: catalog.revision.clone(),
        analysis_sequence: Some("1".into()),
        consistency: "request_transactions_matched_watermarks".into(),
    };
    drop(catalog);
    let field = f
        .reader
        .freeze_origin_width(&f.source, &hex(100), &expected)
        .unwrap();
    assert_eq!(field.value, ScalarValue::integer(2480));
    assert_eq!(field.basis.record_id, Some(hex(1)));
    assert_eq!(field.basis.observation_id, Some(hex(10)));
    let saved = serde_json::to_vec(&field).unwrap();
    assert!(matches!(
        f.reader
            .freeze_origin_width(&f.source, &hex(300), &expected)
            .unwrap()
            .value,
        ScalarValue::Missing { .. }
    ));
    let db = Session::fixture(&f.dll, &f.path).unwrap();
    db.query("UPDATE observations SET image_width=17; INSERT INTO applied VALUES(2)")
        .unwrap();
    drop(db);
    let db =
        rusqlite::Connection::open(f.root.path().join("indexes/gen-test/catalog.sqlite")).unwrap();
    db.execute("UPDATE state SET value=2 WHERE key='seq'", [])
        .unwrap();
    drop(db);
    assert_eq!(
        f.reader
            .freeze_origin_width(&f.source, &hex(100), &expected)
            .unwrap_err()
            .code,
        "SOURCE_CHANGED"
    );
    let restored: FrozenField = serde_json::from_slice(&saved).unwrap();
    assert_eq!(restored.value, ScalarValue::integer(2480));
    assert_eq!(restored.basis.observation_id, Some(hex(10)));
}

#[test]
fn blob_links_observations_missing_values_and_paging_remain_distinct() {
    let f = fixture();
    let first = f
        .reader
        .metadata(
            &f.source,
            &hex(100),
            MetadataRequest {
                limit: Some(2),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(first.records.len(), 2);
    assert!(first.stored_width.is_none());
    assert_eq!(
        first.records[0].origin_observation_id,
        first.records[1].origin_observation_id
    );
    let second = f
        .reader
        .metadata(
            &f.source,
            &hex(100),
            MetadataRequest {
                cursor: first.next_cursor.clone(),
                ..req(&first.version.token)
            },
        )
        .unwrap();
    assert_eq!(second.records.len(), 2);
    assert!(second.next_cursor.is_none());
    assert_eq!(second.records[0].post_id.as_deref(), Some("84"));
    let page = f
        .reader
        .observations(&f.source, &hex(100), &hex(1), req(&first.version.token))
        .unwrap();
    assert_eq!(page.items.len(), 10);
    assert_eq!(page.items[0].relation, "asset_origin");
    assert_eq!(page.items[1].relation, "same_post");
    assert_eq!(page.items[0].time_quality.as_deref(), Some("date_only"));
    let fields = &page.items[0].fields;
    assert!(
        matches!(fields.iter().find(|f|f.name=="source_width").unwrap().value,Some(MetadataValue::Integer(ref v)) if v=="2480")
    );
    assert_eq!(
        fields
            .iter()
            .find(|f| f.name == "source_bytes")
            .unwrap()
            .missing_reason
            .as_deref(),
        Some("not_recorded")
    );
    assert!(
        page.items[1]
            .fields
            .iter()
            .find(|f| f.name == "tags")
            .unwrap()
            .truncated
    );
    let rest = f
        .reader
        .observations(
            &f.source,
            &hex(100),
            &hex(1),
            MetadataRequest {
                cursor: page.next_cursor,
                ..req(&first.version.token)
            },
        )
        .unwrap();
    assert_eq!(rest.items.len(), 5);
    assert_eq!(rest.items[0].row_id, "20");
    assert!(rest.next_cursor.is_none());
    assert!(
        f.reader
            .observations(&f.source, &hex(100), &hex(4), req(&first.version.token))
            .unwrap()
            .items
            .is_empty()
    );
    assert!(
        f.reader
            .metadata(&f.source, &hex(300), MetadataRequest::default())
            .unwrap()
            .records
            .is_empty()
    );
    assert_eq!(
        f.reader
            .metadata(&f.source, &hex(999), MetadataRequest::default())
            .unwrap_err()
            .code,
        "NOT_FOUND"
    );
    assert_eq!(
        f.reader
            .metadata(&f.source, "' OR 1=1", MetadataRequest::default())
            .unwrap_err()
            .code,
        "INVALID_INPUT"
    );
    assert_eq!(
        f.reader
            .metadata(
                &f.source,
                &hex(200),
                MetadataRequest {
                    cursor: first.next_cursor,
                    ..Default::default()
                }
            )
            .unwrap_err()
            .code,
        "INVALID_INPUT"
    );
}

#[test]
fn raw_metadata_is_scoped_versioned_and_bounded_in_bytes() {
    let f = fixture();
    let v = f
        .reader
        .metadata(&f.source, &hex(100), MetadataRequest::default())
        .unwrap()
        .version
        .token;
    let raw = f
        .reader
        .raw_metadata(&f.source, &hex(100), &hex(1), &hex(10), &v)
        .unwrap();
    assert_eq!(raw.json.as_deref(), Some("{\"answer\":42}"));
    assert_eq!(raw.schema_id.as_deref(), Some("schema-1"));
    let big = f
        .reader
        .raw_metadata(&f.source, &hex(100), &hex(1), &hex(11), &v)
        .unwrap();
    assert_eq!(big.status, "too_large");
    assert_eq!(big.bytes.as_deref(), Some("150000"));
    assert!(big.json.is_none());
    assert_eq!(
        f.reader
            .raw_metadata(&f.source, &hex(100), &hex(3), &hex(30), &v)
            .unwrap()
            .status,
        "missing"
    );
    assert_eq!(
        f.reader
            .observations(&f.source, &hex(100), &hex(5), req(&v))
            .unwrap_err()
            .code,
        "NOT_FOUND"
    );
    assert_eq!(
        f.reader
            .raw_metadata(&f.source, &hex(100), &hex(1), &hex(50), &v)
            .unwrap_err()
            .code,
        "NOT_FOUND"
    );
    let sq = rusqlite::Connection::open(f.path.with_file_name("catalog.sqlite")).unwrap();
    sq.execute("UPDATE state SET value=2", []).unwrap();
    assert_eq!(
        f.reader
            .metadata(&f.source, &hex(100), MetadataRequest::default())
            .unwrap_err()
            .code,
        "SOURCE_CHANGED"
    );
    let db = Session::fixture(&f.dll, &f.path).unwrap();
    db.query("INSERT INTO applied VALUES (2)").unwrap();
    drop(db);
    assert_eq!(
        f.reader
            .raw_metadata(&f.source, &hex(100), &hex(1), &hex(10), &v)
            .unwrap_err()
            .code,
        "SOURCE_CHANGED"
    );
}

#[test]
fn readonly_session_rejects_writes_and_detects_generation_change() {
    let f = fixture();
    let session = ReadSession::open(&f.reader, &f.source, &hex(100), None).unwrap();
    assert!(session.db.query("UPDATE applied SET seq=2").is_err());
    fs::write(f.root.path().join("CURRENT.json"),serde_json::to_vec(&serde_json::json!({"library_id":f.source.id,"index_version":1,"generation":"missing-generation"})).unwrap()).unwrap();
    assert_eq!(
        session.finish(&f.source).unwrap_err().code,
        "SOURCE_CHANGED"
    );
    drop(session);
    let mut offline = f.source.clone();
    offline.index_root = Some(f.root.path().join("offline"));
    assert_eq!(
        f.reader
            .metadata(&offline, &hex(100), MetadataRequest::default())
            .unwrap_err()
            .code,
        "SOURCE_UNAVAILABLE"
    );
}

#[test]
fn external_writer_helper() {
    if let Some(path) = std::env::var_os("STUDIO_FIXTURE_WRITER") {
        let dll = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/duckdb/duckdb.dll");
        let path = PathBuf::from(path);
        let _db = Session::fixture(&dll, &path).unwrap();
        fs::File::create(path.with_extension("ready"))
            .unwrap()
            .write_all(b"ready")
            .unwrap();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).unwrap();
    }
}

#[test]
fn identity_index_preserves_full_record_links_and_updates_only_after_complete_refresh() {
    use crate::IdentityIndex;
    use std::sync::{Arc, atomic::AtomicBool};
    let f = fixture();
    {
        let db = Session::fixture(&f.dll, &f.path).unwrap();
        db.query("ALTER TABLE assets ADD COLUMN IF NOT EXISTS commit_seq BIGINT DEFAULT 1; ALTER TABLE observations ADD COLUMN IF NOT EXISTS commit_seq BIGINT DEFAULT 1; ALTER TABLE applied ADD COLUMN batch_id VARCHAR; UPDATE applied SET batch_id='one'; CREATE TABLE objects(sha256 VARCHAR,pack_path VARCHAR)").unwrap();
    }
    let index = Arc::new(IdentityIndex::new(f.root.path().join("identity-cache")));
    let metadata = MetadataReader::new(f.dll.clone()).with_identity_index(index.clone());
    let cached = index.reader(&f.source);
    assert_eq!(cached.err().unwrap().code, "SOURCE_INDEX_PREPARING");
    let before = fs::read(&f.path).unwrap();
    index
        .ensure(
            &f.source,
            1 << 30,
            &f.root.path().join("query-temp"),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
    assert_eq!(fs::read(&f.path).unwrap(), before);
    for asset in [100, 200, 300] {
        let key = hex(asset);
        let expected = f
            .reader
            .metadata(&f.source, &key, MetadataRequest::default())
            .unwrap();
        let actual = metadata
            .metadata(&f.source, &key, MetadataRequest::default())
            .unwrap();
        assert_eq!(
            actual
                .records
                .iter()
                .map(|r| &r.record_id)
                .collect::<Vec<_>>(),
            expected
                .records
                .iter()
                .map(|r| &r.record_id)
                .collect::<Vec<_>>()
        );
        let expected = f
            .reader
            .summaries(
                &f.source,
                std::slice::from_ref(&key),
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap();
        let actual = metadata
            .summaries(
                &f.source,
                std::slice::from_ref(&key),
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap();
        assert_eq!(actual[0].post_ids, expected[0].post_ids);
        assert_eq!(actual[0].post_count, expected[0].post_count);
    }
    {
        let db = Session::fixture(&f.dll, &f.path).unwrap();
        db.query(&format!("INSERT INTO assets(asset_id,sha256,post_id,commit_seq) VALUES('{}','{}',99999,2); INSERT INTO applied VALUES(2,'two')",hex(99999),hex(100))).unwrap();
    }
    let catalog = f.root.path().join("indexes/gen-test/catalog.sqlite");
    rusqlite::Connection::open(&catalog)
        .unwrap()
        .execute("UPDATE state SET value=2 WHERE key='seq'", [])
        .unwrap();
    assert!(!index.is_current(&f.source).unwrap());
    let target = f
        .root
        .path()
        .join("identity-cache")
        .join(format!("{}.identity.sqlite", f.source.id));
    let before = fs::read(&target).unwrap();
    assert_eq!(
        index
            .ensure(
                &f.source,
                1 << 30,
                &f.root.path().join("query-temp"),
                Arc::new(AtomicBool::new(true))
            )
            .unwrap_err()
            .code,
        "CANCELLED"
    );
    assert_eq!(fs::read(&target).unwrap(), before);
    index
        .ensure(
            &f.source,
            1 << 30,
            &f.root.path().join("query-temp"),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
    let expected = f
        .reader
        .summaries(&f.source, &[hex(100)], Arc::new(AtomicBool::new(false)))
        .unwrap();
    let actual = metadata
        .summaries(&f.source, &[hex(100)], Arc::new(AtomicBool::new(false)))
        .unwrap();
    assert_eq!(actual[0].post_count, expected[0].post_count);
    assert_eq!(actual[0].post_ids, expected[0].post_ids);
    assert!(index.is_current(&f.source).unwrap());
}
#[test]
fn external_writer_is_reported_as_busy_and_release_allows_retry() {
    let f = fixture();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "metadata::tests::external_writer_helper",
            "--nocapture",
        ])
        .env("STUDIO_FIXTURE_WRITER", &f.path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let ready = f.path.with_extension("ready");
    for _ in 0..100 {
        if ready.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let result = f
        .reader
        .metadata(&f.source, &hex(100), MetadataRequest::default());
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(b"done\n");
    }
    let exit = child.wait().unwrap();
    assert!(exit.success());
    assert!(ready.exists());
    let error = result.unwrap_err();
    assert_eq!(error.code, "SOURCE_BUSY", "{}", error.message);
    assert!(
        f.reader
            .metadata(&f.source, &hex(100), MetadataRequest::default())
            .is_ok()
    );
}
