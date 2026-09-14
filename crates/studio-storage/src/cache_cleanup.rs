//! Resumable bounded deletion of shared query members. State uses the existing
//! project metadata table; old project schemas and immutable artifacts are kept.
use crate::*;
use std::collections::HashSet;

const PREFIX: &str = "query_cleanup/";
const BATCH: usize = 16_384;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryCleanup {
    pub family_id: String,
    pub result_id: String,
    pub spec: Option<QuerySpec>,
    pub state: String,
    pub total: u64,
    pub processed: u64,
    pub removed: u64,
    pub started_millis: String,
    pub updated_millis: String,
    pub error: Option<String>,
    #[serde(default)]
    after: Option<(i64, String, String, i64)>,
    #[serde(default)]
    expired_only: bool,
    #[serde(default)]
    plan: String,
}
fn save(db: &Connection, task: &QueryCleanup) -> Result<()> {
    db.execute("INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![format!("{PREFIX}{}", task.family_id), serde_json::to_string(task).map_err(Error::io)?]).map_err(db_error)?;
    Ok(())
}
pub(super) fn list(db: &Connection) -> Result<Vec<QueryCleanup>> {
    let mut stmt = db
        .prepare("SELECT value FROM meta WHERE key>=?1 AND key<?2 ORDER BY key")
        .map_err(db_error)?;
    stmt.query_map(params![PREFIX, "query_cleanup0"], |r| r.get::<_, String>(0))
        .map_err(db_error)?
        .map(|r| serde_json::from_str(&r.map_err(db_error)?).map_err(Error::io))
        .collect()
}
fn kept_revisions(
    db: &Connection,
    family: &str,
    live: &HashSet<String>,
) -> Result<(bool, i64, Vec<i64>)> {
    let alive = crate::query_cache::checked_ids(live)?;
    let (cached, latest): (bool, i64) = db
        .query_row(
            "SELECT cached,latest_revision FROM query_families WHERE id=?1",
            [family],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(db_error)?;
    let mut stmt=db.prepare(&format!("SELECT DISTINCT member_revision FROM query_results r WHERE family_id=?1 AND (status IN ('queued','running') OR id IN ({alive}) OR EXISTS(SELECT 1 FROM result_references x WHERE x.result_id=r.id))")).map_err(db_error)?;
    let mut kept = stmt
        .query_map([family], |r| r.get::<_, i64>(0))
        .map_err(db_error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_error)?;
    if cached {
        kept.push(latest);
    }
    kept.sort_unstable();
    kept.dedup();
    Ok((cached, latest, kept))
}
pub(super) fn queue(
    db: &Connection,
    family: &str,
    explicit: bool,
    live: &HashSet<String>,
) -> Result<()> {
    let plan = serde_json::to_string(&kept_revisions(db, family, live)?).map_err(Error::io)?;
    let key = format!("{PREFIX}{family}");
    let previous: Option<String> = db
        .query_row("SELECT value FROM meta WHERE key=?1", [&key], |r| r.get(0))
        .optional()
        .map_err(db_error)?;
    if let Some(raw) = previous {
        let task: QueryCleanup = serde_json::from_str(&raw).map_err(Error::io)?;
        if matches!(task.state.as_str(), "queued" | "deleting" | "accounting")
            || (!explicit && task.plan == plan)
        {
            return Ok(());
        }
    }
    let (result_id, total): (String, u64) = db
        .query_row(
            "SELECT coalesce(latest_result_id,id),stored_members FROM query_families WHERE id=?1",
            [family],
            |r| Ok((r.get(0)?, unsigned(r, 1)?)),
        )
        .map_err(db_error)?;
    let raw: String = db
        .query_row(
            "SELECT spec_json FROM query_results WHERE id=?1",
            [&result_id],
            |r| r.get(0),
        )
        .map_err(db_error)?;
    save(
        db,
        &QueryCleanup {
            family_id: family.into(),
            result_id,
            spec: serde_json::from_str(&raw).ok(),
            state: "queued".into(),
            total,
            processed: 0,
            removed: 0,
            started_millis: now(),
            updated_millis: now(),
            error: None,
            after: None,
            expired_only: false,
            plan,
        },
    )
}
pub(super) fn pending(db: &Connection) -> Result<bool> {
    Ok(list(db)?
        .iter()
        .any(|t| matches!(t.state.as_str(), "queued" | "deleting")))
}
/// Removing an owner can expose a version-1 input that was already uncached.
/// Queue metadata only; the worker rechecks all live and persistent references.
pub(super) fn queue_unreferenced_inputs(db: &Connection) -> Result<()> {
    let mut stmt = db.prepare("SELECT id FROM query_families f WHERE cached=0 AND fixed=0 AND stored_members>0 AND NOT EXISTS(SELECT 1 FROM query_results r JOIN result_references x ON x.result_id=r.id WHERE r.family_id=f.id) AND NOT EXISTS(SELECT 1 FROM query_results r WHERE r.family_id=f.id AND r.status IN ('queued','running')) ORDER BY id LIMIT 16").map_err(db_error)?;
    let ids = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(db_error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_error)?;
    drop(stmt);
    for id in ids {
        queue(db, &id, false, &HashSet::new())?;
    }
    Ok(())
}
pub(super) fn finish_accounting(db: &Connection) -> Result<()> {
    for mut task in list(db)? {
        if task.state == "accounting" {
            task.state = "completed".into();
            task.updated_millis = now();
            save(db, &task)?;
        }
    }
    // Keep recent completion messages, and all active/error tasks.
    db.execute("DELETE FROM meta WHERE key IN (SELECT key FROM meta WHERE key>=?1 AND key<?2 AND json_extract(value,'$.state') IN ('completed','failed') ORDER BY CAST(json_extract(value,'$.updated_millis') AS INTEGER) DESC LIMIT -1 OFFSET 32)", params![PREFIX,"query_cleanup0"]).map_err(db_error)?;
    Ok(())
}
pub(super) fn step(db: &mut Connection, live: &HashSet<String>) -> Result<Option<bool>> {
    let Some(mut task) = list(db)?
        .into_iter()
        .filter(|t| matches!(t.state.as_str(), "queued" | "deleting"))
        .min_by_key(|t| t.started_millis.clone())
    else {
        return Ok(None);
    };
    let previous = task.clone();
    let result: Result<bool> = (|| {
        let tx = db.transaction().map_err(db_error)?;
        let (_, latest, kept) = kept_revisions(&tx, &task.family_id, live)?;
        if kept.is_empty() {
            tx.execute("UPDATE query_results SET status='released',count=NULL,error='无引用的输入缓存已回收' WHERE family_id=?1 AND status='ready'", [&task.family_id]).map_err(db_error)?;
            tx.execute(
                "UPDATE query_families SET latest_count=0 WHERE id=?1",
                [&task.family_id],
            )
            .map_err(db_error)?;
            tx.execute("DELETE FROM artifact_references WHERE owner_kind='query_result' AND owner_id IN (SELECT id FROM query_results WHERE family_id=?1 AND status='released')", [&task.family_id]).map_err(db_error)?;
        }
        let plan = serde_json::to_string(&kept_revisions(&tx, &task.family_id, live)?)
            .map_err(Error::io)?;
        if task.plan != plan {
            task.after = None;
            task.processed = 0;
            task.removed = 0;
            task.plan = plan;
            task.total = tx
                .query_row(
                    "SELECT stored_members FROM query_families WHERE id=?1",
                    [&task.family_id],
                    |r| unsigned(r, 0),
                )
                .map_err(db_error)?;
        }
        let expired_only = kept.contains(&latest);
        if task.expired_only != expired_only {
            task.after = None;
            task.processed = 0;
        }
        task.expired_only = expired_only;
        let index = if expired_only {
            "INDEXED BY query_members_expired"
        } else {
            ""
        };
        let extra = if expired_only {
            "AND valid_until IS NOT NULL"
        } else {
            ""
        };
        let seek = if expired_only {
            "(valid_until,source_id,asset_id,valid_from)>(?2,?3,?4,?5)"
        } else {
            "(source_id,asset_id,valid_from)>(?3,?4,?5)"
        };
        let order = if expired_only {
            "valid_until,source_id,asset_id,valid_from"
        } else {
            "source_id,asset_id,valid_from"
        };
        let after = task
            .after
            .clone()
            .unwrap_or((-1, String::new(), String::new(), -1));
        // Include one lookahead row so exact multiples also finish without a scan.
        let mut stmt = tx.prepare(&format!("SELECT source_id,asset_id,valid_from,valid_until FROM query_member_data {index} WHERE family_id=?1 {extra} AND {seek} ORDER BY {order} LIMIT ?6")).map_err(db_error)?;
        let mut rows = stmt
            .query_map(
                params![
                    task.family_id,
                    after.0,
                    after.1,
                    after.2,
                    after.3,
                    (BATCH + 1) as i64
                ],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, Option<i64>>(3)?,
                    ))
                },
            )
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        drop(stmt);
        let done = rows.len() <= BATCH;
        rows.truncate(BATCH);
        tx.execute_batch("CREATE TEMP TABLE IF NOT EXISTS cleanup_keys(source_id TEXT,asset_id TEXT,valid_from INTEGER,PRIMARY KEY(source_id,asset_id,valid_from)) WITHOUT ROWID; DELETE FROM cleanup_keys;").map_err(db_error)?;
        {
            let mut insert = tx
                .prepare("INSERT INTO cleanup_keys VALUES(?1,?2,?3)")
                .map_err(db_error)?;
            for (source, asset, from, until) in &rows {
                if !kept
                    .iter()
                    .any(|v| v >= from && until.is_none_or(|end| *v < end))
                {
                    insert
                        .execute(params![source, asset, from])
                        .map_err(db_error)?;
                }
            }
        }
        let removed = tx.execute("DELETE FROM query_member_data WHERE family_id=?1 AND (source_id,asset_id,valid_from) IN (SELECT source_id,asset_id,valid_from FROM cleanup_keys)", [&task.family_id]).map_err(db_error)? as u64;
        tx.execute(
            "UPDATE query_families SET stored_members=MAX(0,stored_members-?2) WHERE id=?1",
            params![task.family_id, removed as i64],
        )
        .map_err(db_error)?;
        task.processed += rows.len() as u64;
        task.removed += removed;
        if let Some((source, asset, from, until)) = rows.last() {
            task.after = Some((
                if expired_only { until.unwrap_or(0) } else { 0 },
                source.clone(),
                asset.clone(),
                *from,
            ));
        }
        task.state = if done { "accounting" } else { "deleting" }.into();
        task.updated_millis = now();
        if done {
            tx.execute(
                "UPDATE query_families SET prune_pending=?2 WHERE id=?1",
                params![task.family_id, kept.iter().any(|r| *r < latest)],
            )
            .map_err(db_error)?;
            if task.removed > 0 {
                crate::query_cache::touch_sizes(&tx)?;
            }
        }
        save(&tx, &task)?;
        tx.commit().map_err(db_error)?;
        Ok(true)
    })();
    match result {
        Ok(changed) => Ok(Some(changed)),
        Err(error) => {
            task = previous;
            task.state = "failed".into();
            task.error = Some(error.to_string());
            task.updated_millis = now();
            save(db, &task)?;
            Err(error)
        }
    }
}

impl SqliteStore {
    pub fn query_cleanup_status(&self, pid: &str) -> Result<Vec<QueryCleanup>> {
        let project = self.handle(pid)?;
        list(&*project.read()?)
    }
    pub fn query_cleanup_pending(&self, pid: &str) -> Result<bool> {
        let project = self.handle(pid)?;
        pending(&*project.read()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cleanup_is_bounded_and_resumes_from_committed_progress() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-runs");
        fs::create_dir_all(&root).unwrap();
        let temp = tempfile::Builder::new()
            .prefix("cache-cleanup-")
            .tempdir_in(root)
            .unwrap();
        let path = temp.path().join("project.sqlite");
        let mut db = Connection::open(&path).unwrap();
        migrations::initialize(&mut db).unwrap();
        db.execute_batch("INSERT INTO sources VALUES('source','{}'); INSERT INTO query_results(id,spec_json,versions_json,status,count,created_at) VALUES('family','{}','[]','released',0,'1'); UPDATE query_families SET latest_revision=1,latest_result_id='family',stored_members=70000 WHERE id='family'; WITH RECURSIVE n(x) AS(VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<70000) INSERT INTO query_member_data(family_id,source_id,asset_id,valid_from,post_id) SELECT 'family','source',printf('%064x',x),1,x FROM n;").unwrap();
        queue(&db, "family", true, &HashSet::new()).unwrap();
        let size_revision = crate::query_cache::sizes_revision(&db).unwrap();
        step(&mut db, &HashSet::new()).unwrap();
        let first = list(&db).unwrap().remove(0);
        assert_eq!(first.state, "deleting");
        assert!(first.removed > 0 && first.removed <= 16384);
        assert_eq!(first.processed, first.removed);
        assert_eq!(
            crate::query_cache::sizes_revision(&db).unwrap(),
            size_revision,
            "no full accounting invalidation for every deletion batch"
        );
        drop(db);
        let mut db = Connection::open(path).unwrap();
        let resumed = list(&db).unwrap().remove(0);
        assert_eq!(resumed.removed, first.removed);
        for _ in 0..8 {
            if step(&mut db, &HashSet::new()).unwrap().is_none() {
                break;
            }
        }
        let done = list(&db).unwrap().remove(0);
        assert_eq!(done.state, "accounting");
        assert_eq!(done.removed, 70000);
        assert_eq!(done.processed, 70000);
        assert_eq!(
            db.query_row("SELECT count(*) FROM query_member_data", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        finish_accounting(&db).unwrap();
        assert_eq!(list(&db).unwrap()[0].state, "completed");
        let before: i64 = db
            .query_row("PRAGMA freelist_count", [], |r| r.get(0))
            .unwrap();
        assert!(before > 128);
        crate::query_cache::vacuum_pages(&db, 128).unwrap();
        let after: i64 = db
            .query_row("PRAGMA freelist_count", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            before - after,
            128,
            "drain PRAGMA results instead of returning after one page"
        );
    }
}
