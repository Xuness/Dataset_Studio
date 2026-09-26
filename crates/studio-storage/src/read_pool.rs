use crate::*;
use std::ops::Deref;

/// Short WAL snapshots are independent of the single project writer. At most
/// four idle connections (4 MiB page cache each) are retained; API read budgets
/// continue to govern concurrent expensive work.
#[derive(Default)]
pub(super) struct ReadPool {
    idle: Mutex<Vec<Connection>>,
    active: Mutex<usize>,
}
struct ActiveRead<'a>(&'a Mutex<usize>);
impl Drop for ActiveRead<'_> {
    fn drop(&mut self) {
        if let Ok(mut count) = self.0.lock() {
            *count -= 1;
        }
    }
}
pub(super) struct ReadGuard<'a> {
    db: Option<Connection>,
    pool: &'a ReadPool,
    _active: ActiveRead<'a>,
}
impl ReadPool {
    pub fn read(&self, path: &Path) -> Result<ReadGuard<'_>> {
        *self.active.lock().map_err(lock_error)? += 1;
        let active = ActiveRead(&self.active);
        let cached = self.idle.lock().map_err(lock_error)?.pop();
        let db = if let Some(db) = cached {
            db
        } else {
            let db = Connection::open_with_flags(
                path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
            )
            .map_err(db_error)?;
            db.busy_timeout(std::time::Duration::from_secs(3))
                .map_err(db_error)?;
            if let Some(directory) = path.parent() {
                crate::result_store::attach(&db, directory)?;
            }
            db.execute_batch("PRAGMA query_only=ON; PRAGMA cache_size=-4096;")
                .map_err(db_error)?;
            db
        };
        db.execute_batch("BEGIN").map_err(db_error)?;
        let _: i64 = db
            .query_row("SELECT count(*) FROM main.sqlite_master", [], |r| r.get(0))
            .map_err(db_error)?;
        Ok(ReadGuard {
            db: Some(db),
            pool: self,
            _active: active,
        })
    }
    pub fn collect_gate(&self) -> Result<Option<std::sync::MutexGuard<'_, usize>>> {
        let gate = self.active.lock().map_err(lock_error)?;
        if *gate > 0 {
            return Ok(None);
        }
        Ok(Some(gate))
    }
}
impl Deref for ReadGuard<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.db.as_ref().expect("live read snapshot")
    }
}
impl Drop for ReadGuard<'_> {
    fn drop(&mut self) {
        if let Some(db) = self.db.take() {
            if db.execute_batch("ROLLBACK").is_err() {
                return;
            }
            if let Ok(mut idle) = self.pool.idle.lock()
                && idle.len() < 4
            {
                idle.push(db);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use studio_application::ProjectRepository;

    fn fixture() -> (tempfile::TempDir, Arc<SqliteStore>, Project) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-runs");
        fs::create_dir_all(&root).unwrap();
        let temp = tempfile::Builder::new()
            .prefix("read-pool-")
            .tempdir_in(root)
            .unwrap();
        let store = Arc::new(SqliteStore::new(temp.path().join("state")).unwrap());
        let project = store.create("读取快照", None).unwrap();
        (temp, store, project)
    }
    #[test]
    fn readers_observe_only_committed_state_while_writer_is_held() {
        let (_temp, store, project) = fixture();
        let handle = store.handle(&project.id).unwrap();
        std::thread::scope(|scope| {
            let mut writer = handle.db.lock().unwrap();
            let tx = writer.project_transaction().unwrap();
            tx.execute(
                "INSERT INTO collections VALUES (?1,'尚未发布',0)",
                [new_id()],
            )
            .unwrap();
            let (send, receive) = mpsc::channel();
            let reader = store.clone();
            let pid = project.id.clone();
            scope.spawn(move || {
                send.send(reader.collections(&pid).map(|c| c.len()))
                    .unwrap();
            });
            let result = receive.recv_timeout(std::time::Duration::from_secs(2));
            tx.commit().unwrap();
            drop(writer);
            assert_eq!(
                result.expect("reads must not wait for the writer").unwrap(),
                0
            );
        });
        assert_eq!(store.collections(&project.id).unwrap().len(), 1);
        let readers = (0..8).map(|_| handle.read().unwrap()).collect::<Vec<_>>();
        drop(readers);
        assert_eq!(handle.reads.idle.lock().unwrap().len(), 4);
    }
    #[test]
    fn draft_reader_does_not_wait_for_the_writer_or_observe_uncommitted_drafts() {
        use studio_application::DraftRepository;
        let (_temp, store, project) = fixture();
        store
            .save_draft(
                &project.id,
                "query",
                "default",
                SaveDraft {
                    schema_version: 1,
                    expected_revision: 0,
                    value: serde_json::json!({"value": "committed"}),
                },
            )
            .unwrap();
        let handle = store.handle(&project.id).unwrap();
        std::thread::scope(|scope| {
            let mut writer = handle.db.lock().unwrap();
            let tx = writer.project_transaction().unwrap();
            tx.execute("UPDATE tool_drafts SET value_json='{}',revision=2", [])
                .unwrap();
            let (send, receive) = mpsc::channel();
            let reader = store.clone();
            let pid = project.id.clone();
            scope.spawn(move || send.send(reader.draft(&pid, "query", "default")).unwrap());
            let result = receive.recv_timeout(std::time::Duration::from_secs(2));
            tx.commit().unwrap();
            drop(writer);
            let draft = result
                .expect("draft read waited for the project writer")
                .unwrap()
                .unwrap();
            assert_eq!(draft.revision, 1);
            assert_eq!(draft.value["value"], "committed");
        });
        assert_eq!(
            store
                .draft(&project.id, "query", "default")
                .unwrap()
                .unwrap()
                .revision,
            2
        );
        let conflict = store
            .save_draft(
                &project.id,
                "query",
                "default",
                SaveDraft {
                    schema_version: 1,
                    expected_revision: 1,
                    value: serde_json::json!({}),
                },
            )
            .unwrap_err();
        assert_eq!(conflict.code, "REVISION_CONFLICT");
    }
    #[test]
    fn cold_accounting_is_off_writer_and_rejects_stale_storage_revisions() {
        let (_temp, store, project) = fixture();
        let handle = store.handle(&project.id).unwrap();
        std::thread::scope(|scope| {
            let mut writer = handle.db.lock().unwrap();
            let tx = writer.project_transaction().unwrap();
            let rid = new_id();
            let sid = new_id();
            tx.execute("INSERT INTO sources VALUES (?1,'{}')", [&sid])
                .unwrap();
            tx.execute("INSERT INTO query_results(id,spec_json,versions_json,status,count,created_at) VALUES (?1,'{}','[]','ready',1,'1')",[&rid]).unwrap();
            tx.execute("INSERT INTO query_member_data(family_id,source_id,asset_id,valid_from) VALUES(?1,?2,'one',1)",params![rid,sid]).unwrap();
            tx.execute(
                "UPDATE query_families SET stored_members=1 WHERE id=?1",
                [&rid],
            )
            .unwrap();
            crate::query_cache::touch_sizes(&tx).unwrap();
            let (send, receive) = mpsc::channel();
            let reader = store.clone();
            let pid = project.id.clone();
            scope.spawn(move || {
                send.send(reader.query_cache_stats(&pid).map(|s| s.member_versions))
                    .unwrap();
            });
            let result = receive.recv_timeout(std::time::Duration::from_secs(2));
            tx.commit().unwrap();
            drop(writer);
            assert_eq!(
                result
                    .expect("cold accounting must not hold/wait for the writer")
                    .unwrap(),
                0
            );
        });
        assert!(store.try_query_cache_stats(&project.id).unwrap().is_none());
        assert_eq!(
            store
                .query_cache_stats(&project.id)
                .unwrap()
                .member_versions,
            1
        );
        assert_eq!(
            store
                .try_query_cache_stats(&project.id)
                .unwrap()
                .unwrap()
                .member_versions,
            1
        );
    }
}
