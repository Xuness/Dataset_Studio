use rusqlite::{Connection, OpenFlags};
use std::fs;
use studio_application::ProjectRepository;
use studio_domain::{AssetKey, new_id};
use studio_storage::SqliteStore;

// Literal v1 schema is kept independently of current initialization code.
const V1: &str = include_str!("../src/schema.sql");
#[test]
fn v1_upgrade_preserves_every_persistent_relationship_and_is_idempotent() {
    upgrade_preserves_every_relationship(1);
}
#[test]
fn v2_upgrade_keeps_earlier_migration_ledger_and_every_relationship() {
    upgrade_preserves_every_relationship(2);
}
#[test]
fn v3_upgrade_preserves_query_members_scopes_and_legacy_artifact_paths() {
    upgrade_preserves_every_relationship(3);
}
#[test]
fn v4_upgrade_preserves_tools_and_drafts_before_query_v2_use() {
    upgrade_preserves_every_relationship(4);
}
#[test]
fn v5_upgrade_preserves_original_query_columns_and_members() {
    upgrade_preserves_every_relationship(5);
}
fn upgrade_preserves_every_relationship(from: u32) {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("中文旧项目");
    fs::create_dir(&dir).unwrap();
    fs::create_dir(dir.join("artifacts")).unwrap();
    let id = new_id();
    let sid = new_id();
    let cid = new_id();
    let jid = new_id();
    let manifest =
        serde_json::json!({"format_version":1,"id":id,"name":"旧项目","created_at":"123"});
    fs::write(
        dir.join("project.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(
        dir.join("artifacts").join(format!("{jid}.jsonl")),
        b"preserved-result\n",
    )
    .unwrap();
    let db = Connection::open(dir.join("project.sqlite")).unwrap();
    db.execute_batch(V1).unwrap();
    if from >= 2 {
        db.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL, backup_directory TEXT); INSERT INTO schema_migrations VALUES (2,'earlier-migration','previous-backup'); PRAGMA user_version=2;").unwrap();
    }
    if from >= 3 {
        db.execute_batch(include_str!("../src/schema_v3.sql"))
            .unwrap();
        db.execute_batch("INSERT INTO schema_migrations VALUES (3,'earlier-scope-migration','scope-backup'); PRAGMA user_version=3;").unwrap();
    }
    let source = serde_json::json!({"id":sid,"name":"旧来源","kind":"demo","index_root":null,"media_root":null});
    db.execute(
        "INSERT INTO sources VALUES (?1,?2)",
        (&sid, source.to_string()),
    )
    .unwrap();
    db.execute("INSERT INTO selection VALUES (?1,'kept-object')", [&sid])
        .unwrap();
    db.execute_batch("UPDATE meta SET value='1' WHERE key='selection_count'; UPDATE meta SET value='7' WHERE key='selection_revision'; UPDATE meta SET value='9' WHERE key='revision'; INSERT INTO drafts VALUES ('tool','{\"kept\":true}');").unwrap();
    db.execute("INSERT INTO collections VALUES (?1,'保存集',1)", [&cid])
        .unwrap();
    db.execute(
        "INSERT INTO collection_members VALUES (?1,?2,'kept-object')",
        (&cid, &sid),
    )
    .unwrap();
    db.execute("INSERT INTO jobs VALUES (?1,'export-manifest','succeeded',1,1,2,'100',NULL,?2,'original-key','original-hash',0)",(&jid,format!("artifacts/{jid}.jsonl"))).unwrap();
    db.execute(
        "INSERT INTO job_inputs VALUES (?1,?2,'kept-object')",
        (&jid, &sid),
    )
    .unwrap();
    db.execute("INSERT INTO events VALUES (42,'job.succeeded',?1)", [&jid])
        .unwrap();
    if from >= 3 {
        let qid = new_id();
        let rid = new_id();
        let spec = serde_json::json!({"version":1,"source_ids":[sid],"conditions":[],"observation_rule":"any_observation","order":"asset_key_asc"}).to_string();
        db.execute(
            "INSERT INTO query_definitions VALUES (?1,'保存条件',1,?2,'1')",
            (&qid, &spec),
        )
        .unwrap();
        db.execute(
            "INSERT INTO query_results VALUES (?1,?2,1,?3,'[]','ready',1,1,'1',NULL)",
            (&rid, &qid, &spec),
        )
        .unwrap();
        db.execute(
            "INSERT INTO result_members VALUES (?1,?2,'kept-object')",
            (&rid, &sid),
        )
        .unwrap();
        db.execute(
            "INSERT INTO result_references VALUES ('collection',?1,?2)",
            (&cid, &rid),
        )
        .unwrap();
        db.execute(
            "INSERT INTO collection_scopes VALUES (?1,'{}','{}')",
            [&cid],
        )
        .unwrap();
    }
    if from >= 4 {
        db.execute_batch(include_str!("../src/schema_v4.sql"))
            .unwrap();
        db.execute_batch("INSERT INTO schema_migrations VALUES (4,'earlier-tools-migration','tools-backup'); INSERT INTO tool_drafts VALUES ('core.query','default',1,17,'kept-time','{\"name\":\"kept draft\"}'); PRAGMA user_version=4;").unwrap();
    }
    if from >= 5 {
        db.execute_batch("INSERT INTO schema_migrations VALUES(5,'query-v2-migration','v5-backup'); PRAGMA user_version=5;").unwrap();
    }
    // Last committed page remains in WAL while the upgrade takes its consistent backup.
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; INSERT INTO events VALUES (43,'fixture.wal','kept');").unwrap();
    let store = SqliteStore::new(root.path().join("runtime")).unwrap();
    let p = store.open(dir.clone()).unwrap();
    assert_eq!(p.id, id);
    assert_eq!(p.revision, 9);
    assert_eq!(store.selection(&id).unwrap().revision, 7);
    let key = AssetKey {
        source_id: sid.clone(),
        asset_id: "kept-object".into(),
    };
    assert_eq!(
        store.selection_keys(&id, None, 10).unwrap(),
        vec![key.clone()]
    );
    assert_eq!(
        store.collection_keys(&id, &cid, None, 10).unwrap(),
        vec![key.clone()]
    );
    assert_eq!(store.job_inputs(&id, &jid, None).unwrap(), vec![key]);
    assert_eq!(store.job(&id, &jid).unwrap().attempt, 2);
    assert_eq!(store.sources(&id).unwrap()[0].id, sid);
    assert_eq!(store.events(&id, 0).unwrap().last().unwrap().sequence, 43);
    assert_eq!(
        fs::read(dir.join("artifacts").join(format!("{jid}.jsonl"))).unwrap(),
        b"preserved-result\n"
    );
    store.open(dir.clone()).unwrap();
    drop(store);
    drop(db);
    let store = SqliteStore::new(root.path().join("runtime")).unwrap();
    store.open(dir.clone()).unwrap();
    assert_eq!(fs::read_dir(dir.join(".backups")).unwrap().count(), 1);
    let backup = fs::read_dir(dir.join(".backups"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let before = Connection::open_with_flags(
        backup.join("project.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let after =
        Connection::open_with_flags(dir.join("project.sqlite"), OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    assert_eq!(
        before
            .query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        from
    );
    assert_eq!(
        after
            .query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        6
    );
    // Exact rows, including drafts, idempotency keys, event sequences and artifact references.
    if from >= 2 {
        assert_eq!(
            after
                .query_row(
                    "SELECT applied_at,backup_directory FROM schema_migrations WHERE version=2",
                    [],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                )
                .unwrap(),
            ("earlier-migration".into(), "previous-backup".into())
        );
    }
    let mut tables = vec![
        "meta",
        "sources",
        "selection",
        "collections",
        "collection_members",
        "jobs",
        "job_inputs",
        "events",
        "drafts",
    ];
    if from >= 3 {
        tables.extend([
            "query_definitions",
            "query_results",
            "result_members",
            "selection_base",
            "selection_exclusions",
            "result_references",
            "job_scopes",
            "collection_scopes",
        ]);
    }
    if from >= 4 {
        tables.extend([
            "tool_drafts",
            "job_runs",
            "artifacts",
            "artifact_rows",
            "artifact_references",
        ]);
    }
    for table in tables {
        let columns = if table == "query_results" {
            "id,definition_id,definition_revision,spec_json,versions_json,status,processed,count,created_at,error"
        } else {
            "*"
        };
        let sql = format!("SELECT {columns} FROM {table} ORDER BY 1");
        fn rows(db: &Connection, sql: &str) -> Vec<Vec<rusqlite::types::Value>> {
            let mut stmt = db.prepare(sql).unwrap();
            let n = stmt.column_count();
            stmt.query_map([], |r| (0..n).map(|i| r.get(i)).collect())
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        }
        assert_eq!(rows(&before, &sql), rows(&after, &sql), "{table}");
    }
    assert_eq!(
        fs::read(backup.join("project.json")).unwrap(),
        fs::read(dir.join("project.json")).unwrap()
    );
}

#[test]
fn future_database_is_rejected_without_journal_or_content_changes() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("future");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("project.json"),serde_json::to_vec(&serde_json::json!({"format_version":1,"id":new_id(),"name":"未来项目","created_at":"1"})).unwrap()).unwrap();
    let path = dir.join("project.sqlite");
    let db = Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE future(value); PRAGMA user_version=500;")
        .unwrap();
    drop(db);
    let before = fs::read(&path).unwrap();
    let store = SqliteStore::new(root.path().join("runtime")).unwrap();
    assert_eq!(
        store.open(dir.clone()).unwrap_err().code,
        "FORMAT_UNSUPPORTED"
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(!dir.join("project.sqlite-wal").exists());
    assert!(!dir.join(".backups").exists());
}
