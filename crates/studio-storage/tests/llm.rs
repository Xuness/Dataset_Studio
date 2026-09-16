use rusqlite::Connection;
use studio_application::llm::LlmRepository;
use studio_storage::SqliteStore;

#[test]
fn v2_registry_upgrade_preserves_preferences_and_backs_up_before_llm_tables() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("registry.sqlite");
    let db = Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE projects(id TEXT PRIMARY KEY,directory TEXT UNIQUE NOT NULL,opened_at TEXT NOT NULL,summary TEXT,background_pending INTEGER NOT NULL DEFAULT 1,issue TEXT); CREATE TABLE source_locations(id TEXT PRIMARY KEY,json TEXT NOT NULL); CREATE TABLE preferences(key TEXT PRIMARY KEY,schema_version INTEGER NOT NULL,revision INTEGER NOT NULL,value_json TEXT NOT NULL); INSERT INTO preferences VALUES('existing',1,4,'{\"retain\":true}'); PRAGMA user_version=2;").unwrap();
    let store = SqliteStore::new(root.path().into()).unwrap();
    assert!(store.providers().unwrap().is_empty());
    assert_eq!(
        db.query_row(
            "SELECT value_json FROM preferences WHERE key='existing'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "{\"retain\":true}"
    );
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        3
    );
    let backup = std::fs::read_dir(root.path().join(".backups"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let backup = Connection::open(backup.join("registry.sqlite")).unwrap();
    assert_eq!(
        backup
            .query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        2
    );
    drop(store);
    let store = SqliteStore::new(root.path().into()).unwrap();
    assert!(store.providers().unwrap().is_empty());
    assert_eq!(
        std::fs::read_dir(root.path().join(".backups"))
            .unwrap()
            .count(),
        1
    );
}
