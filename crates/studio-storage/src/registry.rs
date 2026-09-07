//! The app registry has its own version. Listing it never opens a project DB.
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

pub(super) fn check(path: &Path) -> Result<()> {
    if path.exists() {
        let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(db_error)?;
        let version: u32 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(db_error)?;
        if version > 1 {
            return Err(Error::new(
                "REGISTRY_FORMAT_UNSUPPORTED",
                "应用注册表版本过新",
            ));
        }
    }
    Ok(())
}

pub(super) fn initialize(db: &mut Connection, root: &Path) -> Result<()> {
    let version: u32 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(db_error)?;
    if version == 1 {
        return Ok(());
    }
    let existing = db
        .prepare("SELECT 1 FROM sqlite_master WHERE name='projects'")
        .map_err(db_error)?
        .exists([])
        .map_err(db_error)?;
    if existing {
        let directory =
            root.join(".backups")
                .join(format!("registry-v0-to-v1-{}-{}", now(), new_id()));
        fs::create_dir_all(&directory).map_err(Error::io)?;
        let mut target = Connection::open(directory.join("registry.sqlite")).map_err(db_error)?;
        {
            let backup = Backup::new(db, &mut target).map_err(db_error)?;
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                if Instant::now() >= deadline {
                    return Err(Error::new("BACKUP_TIMEOUT", "注册表备份超时，未开始升级"));
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
        target
            .execute_batch("PRAGMA journal_mode=DELETE;")
            .map_err(db_error)?;
        let integrity: String = target
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .map_err(db_error)?;
        if integrity != "ok" {
            return Err(Error::new("BACKUP_INVALID", "注册表备份校验失败"));
        }
        drop(target);
        fs::OpenOptions::new()
            .write(true)
            .open(directory.join("registry.sqlite"))
            .map_err(Error::io)?
            .sync_all()
            .map_err(Error::io)?;
        atomic_json(
            &directory.join("backup.json"),
            &serde_json::json!({"from":0,"to":1,"created_at":now(),"integrity_check":"ok"}),
        )?;
    }
    let tx = db.transaction().map_err(db_error)?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS projects(id TEXT PRIMARY KEY, directory TEXT UNIQUE NOT NULL, opened_at TEXT NOT NULL); CREATE TABLE IF NOT EXISTS source_locations(id TEXT PRIMARY KEY,json TEXT NOT NULL);
        ALTER TABLE projects ADD COLUMN summary TEXT;
        ALTER TABLE projects ADD COLUMN background_pending INTEGER NOT NULL DEFAULT 1;
        ALTER TABLE projects ADD COLUMN issue TEXT;
        PRAGMA user_version=1;").map_err(db_error)?;
    tx.commit().map_err(db_error)
}
