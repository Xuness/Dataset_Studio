//! Manifest v1 supports database v1 through v11. Only project.sqlite is migrated;
//! artifacts and the manifest are immutable during this upgrade.
use crate::{atomic_json, db_error, now};
use rusqlite::{
    Connection, OpenFlags,
    backup::{Backup, StepResult},
};
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};
use studio_domain::{Error, Result, new_id};

pub(super) const VERSION: u32 = 11;
const V2: &str = "CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL, backup_directory TEXT);";
const STEPS: &[(u32, &str)] = &[
    (2, V2),
    (3, include_str!("schema_v3.sql")),
    (4, include_str!("schema_v4.sql")),
    (5, include_str!("schema_v5.sql")),
    (6, include_str!("schema_v6.sql")),
    (7, include_str!("schema_v7.sql")),
    (8, include_str!("schema_v8.sql")),
    (9, include_str!("schema_v9.sql")),
    (10, include_str!("schema_v10.sql")),
    (11, include_str!("schema_v11.sql")),
];

fn version(db: &Connection) -> Result<u32> {
    db.query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(db_error)
}
fn supported(version: u32) -> Result<()> {
    if !(1..=VERSION).contains(&version) {
        return Err(Error::new(
            "FORMAT_UNSUPPORTED",
            format!("项目数据库版本 {version} 不兼容；当前支持 1–{VERSION}"),
        ));
    }
    Ok(())
}
pub(super) fn check_supported(path: &Path) -> Result<()> {
    let db =
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(db_error)?;
    supported(version(&db)?)
}
pub(super) fn initialize(db: &mut Connection) -> Result<()> {
    db.execute_batch("PRAGMA auto_vacuum=INCREMENTAL;")
        .map_err(db_error)?;
    let tx = db.transaction().map_err(db_error)?;
    tx.execute_batch(include_str!("schema.sql"))
        .map_err(db_error)?;
    apply_steps(&tx, 1, STEPS, None)?;
    tx.pragma_update(None, "user_version", VERSION)
        .map_err(db_error)?;
    tx.commit().map_err(db_error)?;
    db.execute_batch("PRAGMA auto_vacuum=INCREMENTAL; VACUUM;")
        .map_err(db_error)
}
pub(super) fn upgrade(db: &mut Connection, directory: &Path) -> Result<()> {
    upgrade_with(db, directory, STEPS)
}
fn apply_steps(
    db: &Connection,
    from: u32,
    steps: &[(u32, &str)],
    backup: Option<&str>,
) -> Result<()> {
    for (version, sql) in steps.iter().filter(|(version, _)| *version > from) {
        db.execute_batch(sql).map_err(db_error)?;
        db.execute(
            "INSERT INTO schema_migrations VALUES (?1,?2,?3)",
            (version, now(), backup),
        )
        .map_err(db_error)?;
    }
    Ok(())
}
fn upgrade_with(db: &mut Connection, directory: &Path, steps: &[(u32, &str)]) -> Result<()> {
    let from = version(db)?;
    supported(from)?;
    if from == VERSION {
        return Ok(());
    }
    let relative = format!(".backups/v{from}-to-v{VERSION}-{}-{}", now(), new_id());
    let backup_dir = directory.join(&relative);
    let backup_root = directory.join(".backups");
    fs::create_dir_all(&backup_root).map_err(Error::io)?;
    if !backup_root
        .canonicalize()
        .map_err(Error::io)?
        .starts_with(directory.canonicalize().map_err(Error::io)?)
    {
        return Err(Error::invalid("项目备份目录必须位于项目内"));
    }
    fs::create_dir(&backup_dir).map_err(Error::io)?;
    let result = (|| {
        let mut target = Connection::open(backup_dir.join("project.sqlite")).map_err(db_error)?;
        {
            let backup = Backup::new(db, &mut target).map_err(db_error)?;
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                if Instant::now() >= deadline {
                    return Err(Error::new("BACKUP_TIMEOUT", "项目备份超时，未开始升级"));
                }
                match backup.step(128).map_err(db_error)? {
                    StepResult::Done => break,
                    StepResult::Busy | StepResult::Locked => {
                        std::thread::sleep(Duration::from_millis(25))
                    }
                    _ => {}
                }
            }
        }
        // Self-contained backup, no external WAL needed for recovery.
        target
            .execute_batch("PRAGMA journal_mode=DELETE;")
            .map_err(db_error)?;
        let integrity: String = target
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .map_err(db_error)?;
        if integrity != "ok" || version(&target)? != from {
            return Err(Error::new(
                "BACKUP_INVALID",
                "项目备份完整性校验失败，未开始升级",
            ));
        }
        drop(target);
        fs::OpenOptions::new()
            .write(true)
            .open(backup_dir.join("project.sqlite"))
            .map_err(Error::io)?
            .sync_all()
            .map_err(Error::io)?;
        let manifest = fs::read(directory.join("project.json")).map_err(Error::io)?;
        let mut saved = fs::File::create(backup_dir.join("project.json")).map_err(Error::io)?;
        std::io::Write::write_all(&mut saved, &manifest).map_err(Error::io)?;
        saved.sync_all().map_err(Error::io)?;
        atomic_json(
            &backup_dir.join("backup.json"),
            &serde_json::json!({"from_database_version":from,"to_database_version":VERSION,"created_at":now(),"integrity_check":"ok","scope":"project.sqlite and project.json; artifact files are unchanged"}),
        )?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(db_error)?;
        if version(&tx)? != from {
            return Err(Error::new("FORMAT_CHANGED", "备份后项目数据库版本已变化"));
        }
        apply_steps(&tx, from, steps, Some(&relative))?;
        tx.pragma_update(None, "user_version", VERSION)
            .map_err(db_error)?;
        let violations: bool = tx
            .prepare("PRAGMA foreign_key_check")
            .map_err(db_error)?
            .exists([])
            .map_err(db_error)?;
        if violations {
            return Err(Error::new("MIGRATION_INVALID", "升级后的项目关联校验失败"));
        }
        tx.commit().map_err(db_error)
    })();
    result.map_err(|e| {
        Error::new(
            "PROJECT_MIGRATION_FAILED",
            format!(
                "项目升级未完成：{}。备份与诊断目录：{}",
                e.message,
                backup_dir.display()
            ),
        )
    })?;
    // Repack once while this project is exclusively owned and its pre-upgrade
    // backup is already durable. Subsequent cache eviction can return free pages
    // in small batches instead of preserving a file high-water mark forever.
    if db
        .query_row("PRAGMA auto_vacuum", [], |r| r.get::<_, u32>(0))
        .map_err(db_error)?
        != 2
        && let Err(error) = db.execute_batch("PRAGMA auto_vacuum=INCREMENTAL; VACUUM;")
    {
        tracing::warn!(%error,"project free-page reclamation setup deferred");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_migration_rolls_back_and_preserves_committed_wal_backup() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("project.json"), b"{\"format_version\":1}").unwrap();
        let mut db = Connection::open(root.path().join("project.sqlite")).unwrap();
        db.execute_batch(include_str!("schema.sql")).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; INSERT INTO drafts VALUES ('inspector','{\"kept\":true}');").unwrap();
        assert!(
            root.path()
                .join("project.sqlite-wal")
                .metadata()
                .unwrap()
                .len()
                > 0
        );
        let error = upgrade_with(
            &mut db,
            root.path(),
            &[
                (2, V2),
                (
                    3,
                    "CREATE TABLE partial(id); SELECT * FROM deliberate_failure;",
                ),
            ],
        )
        .unwrap_err();
        assert_eq!(error.code, "PROJECT_MIGRATION_FAILED");
        assert_eq!(version(&db).unwrap(), 1);
        assert!(
            !db.prepare("SELECT 1 FROM sqlite_master WHERE name='partial'")
                .unwrap()
                .exists([])
                .unwrap()
        );
        let folder = fs::read_dir(root.path().join(".backups"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let saved = Connection::open_with_flags(
            folder.join("project.sqlite"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        assert_eq!(version(&saved).unwrap(), 1);
        assert_eq!(
            saved
                .query_row(
                    "SELECT json FROM drafts WHERE tool_id='inspector'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "{\"kept\":true}"
        );
        assert!(folder.join("backup.json").is_file());
        upgrade(&mut db, root.path()).unwrap();
        assert_eq!(version(&db).unwrap(), VERSION);
    }
}
