use rusqlite::{Connection, OpenFlags};
use std::{fs, path::Path};
use studio_application::ProjectRepository;
use studio_domain::new_id;
use studio_storage::SqliteStore;

fn old_registry(root: &Path) -> Connection {
    let db = Connection::open(root.join("registry.sqlite")).unwrap();
    db.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;
         CREATE TABLE projects(id TEXT PRIMARY KEY,directory TEXT UNIQUE NOT NULL,opened_at TEXT NOT NULL);
         CREATE TABLE source_locations(id TEXT PRIMARY KEY,json TEXT NOT NULL);",
    ).unwrap();
    db.execute(
        "INSERT INTO projects VALUES (?1,'offline-project','123')",
        [new_id()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO source_locations VALUES ('lake','original location JSON')",
        [],
    )
    .unwrap();
    db
}

fn assert_old_rows(db: &Connection) {
    assert_eq!(
        db.query_row("SELECT directory,opened_at FROM projects", [], |r| Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?
        )))
        .unwrap(),
        ("offline-project".into(), "123".into())
    );
    assert_eq!(
        db.query_row(
            "SELECT json FROM source_locations WHERE id='lake'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "original location JSON"
    );
}

#[test]
fn legacy_registry_upgrade_backs_up_committed_wal_and_is_idempotent() {
    let root = tempfile::tempdir().unwrap();
    let old = old_registry(root.path());
    assert!(
        root.path()
            .join("registry.sqlite-wal")
            .metadata()
            .unwrap()
            .len()
            > 0
    );
    let store = SqliteStore::new(root.path().into()).unwrap();
    assert_eq!(store.list().unwrap().len(), 1);
    assert!(store.owned_projects().unwrap().is_empty());
    assert_old_rows(&old);
    assert_eq!(
        old.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        2
    );
    let backups = fs::read_dir(root.path().join(".backups"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1);
    let backup = Connection::open_with_flags(
        backups[0].join("registry.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_old_rows(&backup);
    assert_eq!(
        backup
            .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    assert_eq!(
        backup
            .query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        0
    );
    drop(backup);
    drop(old);
    drop(store);
    let reopened = SqliteStore::new(root.path().into()).unwrap();
    assert_eq!(reopened.list().unwrap().len(), 1);
    assert_eq!(
        fs::read_dir(root.path().join(".backups")).unwrap().count(),
        1
    );
}

#[test]
fn v1_registry_upgrade_preserves_summary_and_issue_and_creates_preferences() {
    let root = tempfile::tempdir().unwrap();
    let old = old_registry(root.path());
    old.execute_batch("ALTER TABLE projects ADD COLUMN summary TEXT; ALTER TABLE projects ADD COLUMN background_pending INTEGER NOT NULL DEFAULT 1; ALTER TABLE projects ADD COLUMN issue TEXT; UPDATE projects SET summary='saved summary',issue='saved issue'; PRAGMA user_version=1;").unwrap();
    let store = SqliteStore::new(root.path().into()).unwrap();
    assert!(store.owned_projects().unwrap().is_empty());
    assert_old_rows(&old);
    assert_eq!(
        old.query_row("SELECT summary,issue FROM projects", [], |r| Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?
        )))
        .unwrap(),
        ("saved summary".into(), "saved issue".into())
    );
    assert_eq!(
        old.query_row("SELECT COUNT(*) FROM preferences", [], |r| r
            .get::<_, u32>(0))
            .unwrap(),
        0
    );
    let backup = fs::read_dir(root.path().join(".backups"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let backup = Connection::open_with_flags(
        backup.join("registry.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        backup
            .query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        1
    );
    assert_old_rows(&backup);
}

#[test]
fn registry_upgrade_failure_preserves_old_version_rows_and_backup() {
    let root = tempfile::tempdir().unwrap();
    let old = old_registry(root.path());
    // A conflicting column makes the migration fail inside its transaction.
    old.execute_batch("ALTER TABLE projects ADD COLUMN summary TEXT;")
        .unwrap();
    assert_eq!(
        SqliteStore::new(root.path().into()).err().unwrap().code,
        "DATABASE_ERROR"
    );
    assert_old_rows(&old);
    assert_eq!(
        old.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        fs::read_dir(root.path().join(".backups")).unwrap().count(),
        1
    );
}

#[test]
fn future_registry_is_rejected_before_journal_or_file_changes() {
    let root = tempfile::tempdir().unwrap();
    let old = old_registry(root.path());
    old.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA user_version=999;")
        .unwrap();
    drop(old);
    let path = root.path().join("registry.sqlite");
    let before = fs::read(&path).unwrap();
    assert_eq!(
        SqliteStore::new(root.path().into()).err().unwrap().code,
        "REGISTRY_FORMAT_UNSUPPORTED"
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(!root.path().join("registry.sqlite-wal").exists());
    assert!(!root.path().join(".backups").exists());
}
