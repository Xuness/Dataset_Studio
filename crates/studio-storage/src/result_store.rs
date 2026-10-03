//! Large fixed memberships live behind their own writer. Only a sealed header
//! is registered in the small project database after all member chunks commit.
use crate::*;
#[cfg(test)]
mod tests;

const SCHEMA:&str="CREATE TABLE IF NOT EXISTS owner(project_id TEXT PRIMARY KEY);
 CREATE TABLE IF NOT EXISTS datasets(id TEXT PRIMARY KEY,state TEXT NOT NULL,count INTEGER NOT NULL DEFAULT 0,after_source TEXT NOT NULL DEFAULT '',after_asset TEXT NOT NULL DEFAULT '');
 CREATE TABLE IF NOT EXISTS members(dataset_id TEXT NOT NULL,source_id TEXT NOT NULL,asset_id TEXT NOT NULL,post_id INTEGER,PRIMARY KEY(dataset_id,source_id,asset_id)) WITHOUT ROWID;
 CREATE INDEX IF NOT EXISTS member_post_order ON members(dataset_id,post_id,source_id,asset_id);";

pub(super) fn accounting(project: &Connection) -> Result<(u64, u64, u64)> {
    let Some(path) = project
        .path()
        .filter(|p| !p.is_empty())
        .and_then(|p| Path::new(p).parent())
        .map(|p| p.join("members.sqlite"))
    else {
        return Ok((0, 0, 0));
    };
    if !path.is_file() {
        return Ok((0, 0, 0));
    }
    let db = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(db_error)?;
    db.busy_timeout(std::time::Duration::from_millis(500))
        .map_err(db_error)?;
    db.execute_batch("BEGIN").map_err(db_error)?;
    let scalar = |sql: &str| db.query_row(sql, [], |r| unsigned(r, 0)).map_err(db_error);
    let pages = scalar("PRAGMA page_count")?;
    let free = scalar("PRAGMA freelist_count")?;
    let size = scalar("PRAGMA page_size")?;
    Ok((
        (pages - free) * size,
        scalar("SELECT coalesce(sum(count),0) FROM datasets WHERE state='sealed'")?,
        free * size,
    ))
}

pub(super) fn initialize(directory: &Path, pid: &str) -> Result<Connection> {
    let path = directory.join("members.sqlite");
    if path.is_symlink() {
        return Err(Error::invalid("成员数据库不能是链接"));
    }
    let db = connection(&path)?;
    db.execute_batch("PRAGMA auto_vacuum=INCREMENTAL;")
        .map_err(db_error)?;
    db.execute_batch(SCHEMA).map_err(db_error)?;
    let saved: Option<String> = db
        .query_row("SELECT project_id FROM owner", [], |r| r.get(0))
        .optional()
        .map_err(db_error)?;
    if saved.as_deref().is_some_and(|old| old != pid) {
        return Err(Error::invalid("成员数据库属于另一个项目"));
    }
    db.execute("INSERT OR IGNORE INTO owner VALUES(?1)", [pid])
        .map_err(db_error)?;
    Ok(db)
}

pub(super) fn attach(db: &Connection, directory: &Path) -> Result<()> {
    let path = directory.join("members.sqlite");
    if !path.exists() {
        return Ok(());
    }
    let path = path.canonicalize().map_err(Error::io)?;
    if path.parent() != Some(directory.canonicalize().map_err(Error::io)?.as_path()) {
        return Err(Error::invalid("成员数据库超出项目目录"));
    }
    // Only result_writer writes this attachment. Connection-local views expose
    // sealed rows to all existing scope consumers without copying memberships.
    let mut uri =
        url::Url::from_file_path(&path).map_err(|_| Error::invalid("成员数据库路径无效"))?;
    uri.set_query(Some("mode=ro"));
    db.execute("ATTACH DATABASE ?1 AS result_store", [uri.as_str()])
        .map_err(db_error)?;
    db.execute_batch("CREATE TEMP VIEW result_members AS
      SELECT r.id AS result_id,m.source_id,m.asset_id FROM main.query_results r JOIN main.query_member_data m ON m.family_id=r.family_id
      WHERE r.storage_kind='legacy' AND m.valid_from<=r.member_revision AND (m.valid_until IS NULL OR m.valid_until>r.member_revision)
      UNION ALL SELECT r.id,m.source_id,m.asset_id FROM main.query_results r JOIN result_store.datasets d ON d.id=r.id AND d.state='sealed'
      JOIN result_store.members m ON m.dataset_id=d.id WHERE r.storage_kind='sealed' AND r.status='ready';
      CREATE TEMP TRIGGER result_member_legacy_insert INSTEAD OF INSERT ON result_members BEGIN
        INSERT OR IGNORE INTO query_member_data(family_id,source_id,asset_id,valid_from)
          SELECT family_id,NEW.source_id,NEW.asset_id,member_revision FROM query_results WHERE id=NEW.result_id AND storage_kind='legacy';
      END;
      CREATE TEMP VIEW collection_members AS
      SELECT * FROM main.collection_member_legacy
      UNION ALL SELECT b.collection_id,m.source_id,m.asset_id FROM main.collection_bases b JOIN result_members m ON m.result_id=b.result_id
        WHERE NOT EXISTS(SELECT 1 FROM main.collection_exclusions e WHERE e.collection_id=b.collection_id AND e.source_id=m.source_id AND e.asset_id=m.asset_id)
      UNION ALL SELECT i.collection_id,i.source_id,i.asset_id FROM main.collection_inclusions i
        WHERE NOT EXISTS(SELECT 1 FROM main.collection_bases b JOIN result_members m ON m.result_id=b.result_id WHERE b.collection_id=i.collection_id AND m.source_id=i.source_id AND m.asset_id=i.asset_id);
      CREATE TEMP TRIGGER collection_member_legacy_insert INSTEAD OF INSERT ON collection_members BEGIN
        INSERT OR IGNORE INTO collection_member_legacy VALUES(NEW.collection_id,NEW.source_id,NEW.asset_id);
      END;
      CREATE TEMP VIEW job_inputs AS SELECT * FROM main.job_input_legacy
        UNION ALL SELECT b.job_id,m.source_id,m.asset_id FROM main.job_input_bases b JOIN result_members m ON m.result_id=b.result_id
          WHERE NOT EXISTS(SELECT 1 FROM main.job_input_exclusions e WHERE e.job_id=b.job_id AND e.source_id=m.source_id AND e.asset_id=m.asset_id)
          AND NOT EXISTS(SELECT 1 FROM main.job_input_legacy i WHERE i.job_id=b.job_id AND i.source_id=m.source_id AND i.asset_id=m.asset_id);
      CREATE TEMP TRIGGER job_input_legacy_insert INSTEAD OF INSERT ON job_inputs BEGIN
        INSERT OR IGNORE INTO job_input_legacy VALUES(NEW.job_id,NEW.source_id,NEW.asset_id);
      END;
      CREATE TEMP TRIGGER job_input_legacy_delete INSTEAD OF DELETE ON job_inputs BEGIN
        DELETE FROM job_input_legacy WHERE job_id=OLD.job_id AND source_id=OLD.source_id AND asset_id=OLD.asset_id;
      END;
      CREATE TEMP TRIGGER collection_member_legacy_delete INSTEAD OF DELETE ON collection_members BEGIN
        DELETE FROM collection_member_legacy WHERE collection_id=OLD.collection_id AND source_id=OLD.source_id AND asset_id=OLD.asset_id;
      END;").map_err(db_error)?;
    Ok(())
}

impl SqliteStore {
    pub fn query_storage_kind(&self, pid: &str, rid: &str) -> Result<String> {
        let p = self.handle(pid)?;
        let db = p.read()?;
        db.query_row(
            "SELECT storage_kind FROM query_results WHERE id=?1",
            [rid],
            |r| r.get(0),
        )
        .map_err(db_error)
    }
    pub fn create_snapshot_result(
        &self,
        pid: &str,
        spec: QuerySpec,
        versions: Vec<QuerySourceVersion>,
        view: bool,
    ) -> Result<QueryResult> {
        self.create_snapshot_result_from(pid, None, spec, versions, view)
    }
    pub fn create_snapshot_result_from(
        &self,
        pid: &str,
        definition: Option<(&str, u64)>,
        spec: QuerySpec,
        versions: Vec<QuerySourceVersion>,
        view: bool,
    ) -> Result<QueryResult> {
        let spec = spec.normalize()?;
        self.validate_derived(pid, &spec)?;
        if !view {
            self.mark_background(pid)?;
        }
        let p = self.handle(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.project_transaction().map_err(db_error)?;
        query::validate_sources(&tx, &spec)?;
        query::validate_input(&tx, pid, &spec)?;
        if let Some((id, revision)) = definition {
            let saved = query::read_definition(&tx, pid, id)?;
            if saved.revision != revision || saved.spec.clone().normalize()? != spec {
                return Err(Error::new("REVISION_CONFLICT", "查询定义已变化"));
            }
        }
        let result = query::insert_result(&tx, pid, definition, &spec, &versions)?;
        tx.execute("UPDATE query_results SET storage_kind=?2,status=?3,cache_mode=?4,count=?5,view_expires_ms=?6 WHERE id=?1",params![result.id,if view{"view"}else{"sealed"},if view{"ready"}else{"queued"},if view{"view"}else{"full"},if view{None}else{Some(0_i64)},view.then(||now().parse::<i64>().unwrap_or(0)+30*60*1000)]).map_err(db_error)?;
        tx.execute(
            "UPDATE query_families SET fixed=?2,cached=0 WHERE id=?1",
            params![result.id, !view],
        )
        .map_err(db_error)?;
        if view {
            query::clear_input_references(&tx, &result.id)?;
        }
        let result = query::read_result(&tx, pid, &result.id)?;
        tx.commit().map_err(db_error)?;
        Ok(result)
    }
    pub fn touch_query_view(&self, pid: &str, rid: &str) -> Result<QueryResult> {
        let p = self.handle(pid)?;
        let current = now().parse::<i64>().map_err(Error::io)?;
        let (expiry, result) = {
            let db = p.read()?;
            let expiry:Option<i64>=db.query_row("SELECT view_expires_ms FROM query_results WHERE id=?1 AND storage_kind='view' AND status='ready'",[rid],|r|r.get(0)).optional().map_err(db_error)?.flatten();
            if expiry.is_none_or(|v| v < current) {
                return Err(Error::new("VIEW_EXPIRED", "浏览视图已过期，请刷新数据"));
            }
            (expiry, query::read_result(&db, pid, rid)?)
        };
        if expiry.is_some_and(|v| v < current + 15 * 60 * 1000)
            && let Ok(db) = p.db.try_lock()
        {
            db.execute("UPDATE query_results SET view_expires_ms=?2 WHERE id=?1 AND storage_kind='view' AND status='ready'",params![rid,current+30*60*1000]).map_err(db_error)?;
        }
        Ok(result)
    }

    pub fn publish_snapshot_stage(
        &self,
        pid: &str,
        rid: &str,
        stage: &QueryStage,
        cancelled: &AtomicBool,
    ) -> Result<()> {
        self.publish_snapshot_stage_with_memory(pid, rid, stage, cancelled, 256 << 20)
    }
    pub fn publish_snapshot_stage_with_memory(
        &self,
        pid: &str,
        rid: &str,
        stage: &QueryStage,
        cancelled: &AtomicBool,
        cache_bytes: u64,
    ) -> Result<()> {
        let p = self.handle(pid)?;
        {
            let db = p.read()?;
            let result = query::read_result(&db, pid, rid)?;
            if result.state != ResultState::Running {
                return Err(Error::new("CANCELLED", "固定结果构建已停止"));
            }
        }
        let mut members = p.result_writer.lock().map_err(lock_error)?;
        let previous_cache: i64 = members
            .pragma_query_value(None, "cache_size", |r| r.get(0))
            .map_err(db_error)?;
        members
            .pragma_update(
                None,
                "cache_size",
                -((cache_bytes.clamp(64 << 20, 2 << 30) / 1024) as i64),
            )
            .map_err(db_error)?;
        let copied = stage.copy_to_members(&mut members, rid, cancelled);
        let restored = members
            .pragma_update(None, "cache_size", previous_cache)
            .map_err(db_error);
        copied.and(restored)?;
        let count: i64 = members
            .query_row(
                "SELECT count FROM datasets WHERE id=?1 AND state='sealed'",
                [rid],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        drop(members);
        // The large writer is released before acquiring the control writer.
        // A crash here leaves a recognizable sealed orphan, never a partial ready result.
        let mut db = p.write_cancelled(cancelled)?;
        let tx = db.project_transaction().map_err(db_error)?;
        let result = query::read_result(&tx, pid, rid)?;
        if result.state != ResultState::Running {
            return Err(Error::new("CANCELLED", "固定结果构建已停止"));
        }
        tx.execute("UPDATE query_results SET count=?2,processed=?3,evaluated_count=?4,changed_members=?2,post_ready=?5 WHERE id=?1",params![rid,count,stage.processed as i64,stage.evaluated as i64,stage.post_ready]).map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        Ok(())
    }

    pub fn collect_snapshot_orphans(&self, pid: &str) -> Result<u64> {
        let p = self.handle(pid)?;
        let mut db = match p.result_writer.try_lock() {
            Ok(db) => db,
            Err(std::sync::TryLockError::WouldBlock) => return Ok(0),
            Err(e) => return Err(Error::new("INTERNAL_ERROR", e.to_string())),
        };
        let active = {
            let db = p.read()?;
            let mut stmt=db.prepare("SELECT id FROM query_results WHERE storage_kind='sealed' AND status IN ('queued','running','ready')").map_err(db_error)?;
            stmt.query_map([], |r| r.get::<_, String>(0))
                .map_err(db_error)?
                .collect::<std::result::Result<std::collections::HashSet<_>, _>>()
                .map_err(db_error)?
        };
        let Some(_readers) = p.reads.collect_gate()? else {
            return Ok(0);
        };
        let mut cursor = p.result_gc_cursor.lock().map_err(lock_error)?;
        let mut stmt = db
            .prepare("SELECT id FROM datasets WHERE id>?1 ORDER BY id LIMIT 512")
            .map_err(db_error)?;
        let ids = stmt
            .query_map([cursor.as_deref().unwrap_or("")], |r| r.get::<_, String>(0))
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        drop(stmt);
        *cursor = if ids.len() == 512 {
            ids.last().cloned()
        } else {
            None
        };
        for id in ids {
            if active.contains(&id) {
                continue;
            }
            let tx = db.transaction().map_err(db_error)?;
            let removed=tx.execute("DELETE FROM members WHERE (dataset_id,source_id,asset_id) IN (SELECT dataset_id,source_id,asset_id FROM members WHERE dataset_id=?1 LIMIT 4096)",[&id]).map_err(db_error)?;
            tx.execute("DELETE FROM datasets WHERE id=?1 AND NOT EXISTS(SELECT 1 FROM members WHERE dataset_id=?1)",[&id]).map_err(db_error)?;
            tx.commit().map_err(db_error)?;
            db.execute_batch("PRAGMA incremental_vacuum(128);")
                .map_err(db_error)?;
            return Ok(removed as u64);
        }
        Ok(0)
    }
    pub fn expire_query_views(&self, pid: &str) -> Result<()> {
        let p = self.handle(pid)?;
        if let Ok(db) = p.db.try_lock() {
            db.execute("UPDATE query_results SET status='released' WHERE id IN (SELECT id FROM query_results WHERE storage_kind='view' AND status='ready' AND view_expires_ms<CAST(?1 AS INTEGER) LIMIT 64)",[now()]).map_err(db_error)?;
        }
        Ok(())
    }
}
