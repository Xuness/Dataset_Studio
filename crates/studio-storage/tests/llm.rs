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
        4
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

fn v3_registry(root: &std::path::Path) -> Connection {
    let db = Connection::open(root.join("registry.sqlite")).unwrap();
    db.execute_batch("CREATE TABLE projects(id TEXT PRIMARY KEY,directory TEXT UNIQUE NOT NULL,opened_at TEXT NOT NULL,summary TEXT,background_pending INTEGER NOT NULL DEFAULT 1,issue TEXT); CREATE TABLE source_locations(id TEXT PRIMARY KEY,json TEXT NOT NULL); CREATE TABLE preferences(key TEXT PRIMARY KEY,schema_version INTEGER NOT NULL,revision INTEGER NOT NULL,value_json TEXT NOT NULL); INSERT INTO preferences VALUES('existing',1,4,'{\"retain\":true}'); PRAGMA user_version=3;").unwrap();
    db.execute_batch(include_str!("../src/llm/schema.sql"))
        .unwrap();
    // Migration must leave all existing LLM configuration bytes untouched.
    db.execute_batch("INSERT INTO llm_providers VALUES('existing-provider',7,'provider-with-credential-ref'); INSERT INTO llm_models VALUES('existing-model','existing-provider','remote','openai_chat',4,'model-parameters'); INSERT INTO llm_presets VALUES('existing-preset',2,'parameter-preset'); INSERT INTO llm_catalogs VALUES('existing-provider','remote-catalog');").unwrap();
    db
}

#[test]
fn v3_upgrade_preserves_all_llm_rows_and_backs_up_before_system_prompts() {
    let root = tempfile::tempdir().unwrap();
    let db = v3_registry(root.path());
    let store = SqliteStore::new(root.path().into()).unwrap();
    assert!(store.system_prompts().unwrap().is_empty());
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        4
    );
    let backup = std::fs::read_dir(root.path().join(".backups"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let old = Connection::open(backup.join("registry.sqlite")).unwrap();
    assert_eq!(
        old.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        3
    );
    for table in ["llm_providers", "llm_models", "llm_presets", "llm_catalogs"] {
        let sql = format!("SELECT json FROM {table}");
        assert_eq!(
            db.query_row(&sql, [], |r| r.get::<_, String>(0)).unwrap(),
            old.query_row(&sql, [], |r| r.get::<_, String>(0)).unwrap()
        );
    }
    assert_eq!(
        old.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name='llm_system_prompts'",
            [],
            |r| r.get::<_, u32>(0)
        )
        .unwrap(),
        0
    );
    drop(old);
    drop(store);
    let _reopened = SqliteStore::new(root.path().into()).unwrap();
    assert_eq!(
        std::fs::read_dir(root.path().join(".backups"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn failed_v3_upgrade_keeps_version_and_backup() {
    let root = tempfile::tempdir().unwrap();
    let db = v3_registry(root.path());
    db.execute_batch("CREATE TABLE llm_system_prompts(conflicting TEXT);")
        .unwrap();
    assert_eq!(
        SqliteStore::new(root.path().into()).err().unwrap().code,
        "DATABASE_ERROR"
    );
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        db.query_row("SELECT json FROM llm_providers", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "provider-with-credential-ref"
    );
    assert_eq!(
        std::fs::read_dir(root.path().join(".backups"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn system_prompt_capacity_allows_existing_updates_and_reclaims_deleted_slots() {
    use studio_domain::{llm::*, new_id};
    let root = tempfile::tempdir().unwrap();
    let store = SqliteStore::new(root.path().into()).unwrap();
    let mut value = LlmSystemPrompt {
        id: new_id(),
        revision: 1,
        config: LlmSystemPromptConfig {
            name: "test".into(),
            description: String::new(),
            text: "instructions".into(),
        },
    };
    for _ in 0..128 {
        value.id = new_id();
        store.save_system_prompt(value.clone(), 0).unwrap();
    }
    let last = value.clone();
    value.id = new_id();
    assert_eq!(
        store.save_system_prompt(value.clone(), 0).unwrap_err().code,
        "INVALID_INPUT"
    );
    let updated = store.save_system_prompt(last, 1).unwrap();
    assert_eq!(updated.revision, 2);
    assert_eq!(
        store.remove_system_prompt(&updated.id, 1).unwrap_err().code,
        "REVISION_CONFLICT"
    );
    store.remove_system_prompt(&updated.id, 2).unwrap();
    store.save_system_prompt(value, 0).unwrap();
    assert_eq!(store.system_prompts().unwrap().len(), 128);
}
