use super::db_error;
use crate::lock_error;
use std::{
    any::Any,
    collections::VecDeque,
    path::Path,
    sync::{Arc, Condvar, Mutex, mpsc},
    thread::JoinHandle,
};
use studio_domain::{Error, Result};

type Value = Box<dyn Any + Send>;
type Operation = Box<dyn FnOnce(&rusqlite::Transaction<'_>) -> Result<Value> + Send>;
struct Command {
    bytes: usize,
    queued_at: std::time::Instant,
    operation: Operation,
    reply: mpsc::SyncSender<Result<Value>>,
    point: &'static str,
    key: String,
}
#[derive(Default)]
pub(super) struct QueueStats {
    pub bytes: usize,
    pub peak: usize,
    pub critical_bytes: usize,
    pub last_commit_ms: u64,
}
#[derive(Default)]
struct Queue {
    normal: VecDeque<Command>,
    critical: VecDeque<Command>,
    closed: bool,
}
pub(super) struct Writer {
    queue: Arc<(Mutex<Queue>, Condvar)>,
    thread: Option<JoinHandle<()>>,
    pub stats: Arc<Mutex<QueueStats>>,
}
impl Writer {
    pub fn open(path: &Path) -> Result<Self> {
        if path.exists() {
            let check = rusqlite::Connection::open_with_flags(
                path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .map_err(db_error)?;
            let version: u32 = check
                .pragma_query_value(None, "user_version", |r| r.get(0))
                .map_err(db_error)?;
            let occupied: bool = check
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table')",
                    [],
                    |r| r.get(0),
                )
                .map_err(db_error)?;
            if version > 9 || (version == 0 && occupied) {
                return Err(Error::new("FORMAT_UNSUPPORTED", "评审账本版本不兼容"));
            }
            if occupied {
                let integrity: String = check
                    .query_row("PRAGMA quick_check", [], |r| r.get(0))
                    .map_err(db_error)?;
                if integrity != "ok" {
                    return Err(Error::new(
                        "EVALUATION_CORRUPT",
                        "评审账本完整性校验失败，请恢复备份",
                    ));
                }
            }
        }
        let mut db = crate::connection(path)?;
        let version: u32 = db
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(db_error)?;
        if version > 0 && version < 9 {
            let parent = path
                .parent()
                .ok_or_else(|| Error::invalid("评审路径无效"))?;
            let directory = parent.join(".backups");
            std::fs::create_dir_all(&directory).map_err(Error::io)?;
            if !directory
                .canonicalize()
                .map_err(Error::io)?
                .starts_with(parent.canonicalize().map_err(Error::io)?)
            {
                return Err(Error::invalid("评审备份目录必须在项目内"));
            }
            let destination = directory.join(format!(
                "evaluation-v{version}-to-v9-{}-{}.sqlite",
                crate::now(),
                studio_domain::new_id()
            ));
            let mut target = rusqlite::Connection::open(&destination).map_err(db_error)?;
            {
                let backup = rusqlite::backup::Backup::new(&db, &mut target).map_err(db_error)?;
                backup
                    .run_to_completion(128, std::time::Duration::from_millis(10), None)
                    .map_err(db_error)?;
            }
            target
                .execute_batch("PRAGMA journal_mode=DELETE")
                .map_err(db_error)?;
            let integrity: String = target
                .query_row("PRAGMA quick_check", [], |r| r.get(0))
                .map_err(db_error)?;
            if integrity != "ok" {
                return Err(Error::new("BACKUP_INVALID", "评审升级备份校验失败"));
            }
            drop(target);
            std::fs::OpenOptions::new()
                .write(true)
                .open(&destination)
                .map_err(Error::io)?
                .sync_all()
                .map_err(Error::io)?;
        }
        if version < 9 {
            let tx = db.transaction().map_err(db_error)?;
            if version == 0 {
                tx.execute_batch(include_str!("schema.sql"))
                    .map_err(db_error)?;
            }
            if version < 2 {
                tx.execute_batch(include_str!("schema_v2.sql"))
                    .map_err(db_error)?;
            }
            if version < 3 {
                tx.execute_batch(include_str!("schema_v3.sql"))
                    .map_err(db_error)?;
            }
            if version < 4 {
                tx.execute_batch(include_str!("schema_v4.sql"))
                    .map_err(db_error)?;
            }
            if version < 5 {
                tx.execute_batch(include_str!("schema_v5.sql"))
                    .map_err(db_error)?;
            }
            if version < 6 {
                tx.execute_batch(include_str!("schema_v6.sql"))
                    .map_err(db_error)?;
            }
            if version < 7 {
                tx.execute_batch(include_str!("schema_v7.sql"))
                    .map_err(db_error)?;
            }
            if version < 8 {
                tx.execute_batch(include_str!("schema_v8.sql"))
                    .map_err(db_error)?;
            }
            tx.execute_batch(include_str!("schema_v9.sql"))
                .map_err(db_error)?;
            let violations = tx
                .prepare("PRAGMA foreign_key_check")
                .map_err(db_error)?
                .exists([])
                .map_err(db_error)?;
            let invalid: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM stages WHERE total<1 OR frozen<0 OR frozen>total OR eligible<0 OR eligible>frozen OR attempts<0 OR accepted<0 OR invalid<0 OR unknown<0 OR protected<0 OR comparable>frozen OR excluded>frozen OR unresolved>frozen) OR EXISTS(SELECT 1 FROM candidates WHERE exposures<0 OR bytes<0 OR ordinal<0)",[],|r|r.get(0)).map_err(db_error)?;
            if violations || invalid {
                return Err(Error::new(
                    "MIGRATION_INVALID",
                    "评审历史关联或计数校验失败；保留升级备份",
                ));
            }
            tx.commit().map_err(db_error)?;
        } else if version != 9 {
            return Err(Error::new("FORMAT_UNSUPPORTED", "评审账本版本不兼容"));
        }
        // A new writer is created only under the exclusive project lease.
        let tx = db.transaction().map_err(db_error)?;
        tx.execute_batch("WITH interrupted AS MATERIALIZED (
            SELECT b.stage_id,json_extract(m.value,'$.candidate.ordinal') ordinal,b.sequence batch
            FROM batches b,json_each(b.members) m WHERE b.state='sent'
          ) UPDATE candidates SET reserved=0,blocked=1,blocked_batch=(SELECT batch FROM interrupted i WHERE i.stage_id=candidates.stage_id AND i.ordinal=candidates.ordinal)
            WHERE (stage_id,ordinal) IN (SELECT stage_id,ordinal FROM interrupted);
          UPDATE attempts SET state='outcome_unknown' WHERE state='sent';
          UPDATE batches SET state='outcome_unknown',error='引擎中断；上游可能已受理，请核对后明确选择重试' WHERE state='sent';
          UPDATE batches SET state='queued' WHERE state='preparing';
          UPDATE stages SET state='paused',error='执行已中断；本地结果可以重新解析，远端请求不会自动重发' WHERE state IN ('preparing','running','pausing');
          UPDATE stages SET state='cancelled' WHERE state='cancelling';
          UPDATE stages SET unknown=(SELECT count(*) FROM batches b WHERE b.stage_id=stages.id AND b.state='outcome_unknown');
          UPDATE analysis_jobs SET state='interrupted',error='引擎中断；可从冻结证据重算，不会调用远端模型' WHERE state IN ('queued','running','cancelling');").map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        let queue = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
        let worker_queue = queue.clone();
        let stats = Arc::new(Mutex::new(QueueStats::default()));
        let shared = stats.clone();
        let thread = std::thread::Builder::new()
            .name("aesthetic-writer".into())
            .spawn(move || {
                let mut burst = 0;
                loop {
                    let command = {
                        let (lock, wake) = &*worker_queue;
                        let mut q = match lock.lock() {
                            Ok(q) => q,
                            Err(_) => break,
                        };
                        while q.normal.is_empty() && q.critical.is_empty() && !q.closed {
                            q = match wake.wait(q) {
                                Ok(q) => q,
                                Err(_) => return,
                            };
                        }
                        let next = if !q.critical.is_empty() && (burst < 8 || q.normal.is_empty()) {
                            burst += 1;
                            q.critical.pop_front()
                        } else {
                            burst = 0;
                            q.normal.pop_front()
                        };
                        match next {
                            Some(c) => c,
                            None => break,
                        }
                    };
                    let commit_started = std::time::Instant::now();
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let tx = db.transaction().map_err(db_error)?;
                        let value = (command.operation)(&tx)?;
                        crate::faults::check(
                            &format!("{}_before_commit", command.point),
                            &command.key,
                        )?;
                        tx.commit().map_err(db_error)?;
                        crate::faults::check(
                            &format!("{}_after_commit", command.point),
                            &command.key,
                        )?;
                        Ok(value)
                    }));
                    let fatal = outcome.is_err();
                    let result = outcome.unwrap_or_else(|_| {
                        Err(Error::new(
                            "EVALUATION_WRITER_EXITED",
                            "评审写入线程异常，事务已回滚",
                        ))
                    });
                    if let Ok(mut stats) = shared.lock() {
                        stats.last_commit_ms =
                            commit_started.elapsed().as_millis().min(u64::MAX as u128) as u64;
                        stats.bytes = stats.bytes.saturating_sub(command.bytes);
                        if critical(command.point) {
                            stats.critical_bytes =
                                stats.critical_bytes.saturating_sub(command.bytes);
                        }
                    }
                    // Successful acknowledgement is sent only after the durable commit.
                    let _ = command.reply.send(result);
                    if fatal {
                        if let Ok(mut q) = worker_queue.0.lock() {
                            q.closed = true;
                            q.normal.clear();
                            q.critical.clear();
                            worker_queue.1.notify_all();
                        }
                        if let Ok(mut stats) = shared.lock() {
                            stats.bytes = 0;
                            stats.critical_bytes = 0;
                        }
                        break;
                    }
                }
                let _ = db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");
            })
            .map_err(Error::io)?;
        Ok(Self {
            queue,
            thread: Some(thread),
            stats,
        })
    }
    pub fn queue_metrics(&self) -> Result<(u64, u64, u64, u64)> {
        let queue = self.queue.0.lock().map_err(lock_error)?;
        let oldest = queue
            .normal
            .front()
            .into_iter()
            .chain(queue.critical.front())
            .map(|c| c.queued_at.elapsed().as_millis().min(u64::MAX as u128) as u64)
            .max()
            .unwrap_or(0);
        let stats = self.stats.lock().map_err(lock_error)?;
        Ok((
            (queue.normal.len() + queue.critical.len()) as u64,
            oldest,
            stats.last_commit_ms,
            stats.critical_bytes as u64,
        ))
    }
    pub fn submit<T: Send + 'static>(
        &self,
        bytes: usize,
        operation: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        self.submit_named(bytes, "write", "", operation)
    }
    pub fn submit_named<T: Send + 'static>(
        &self,
        bytes: usize,
        point: &'static str,
        key: &str,
        operation: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let bytes = bytes.max(1024);
        {
            let mut stats = self.stats.lock().map_err(lock_error)?;
            let normal = stats.bytes.saturating_sub(stats.critical_bytes);
            if bytes > 32 << 20
                || stats.bytes.saturating_add(bytes) > 128 << 20
                || (!critical(point) && normal.saturating_add(bytes) > 64 << 20)
                || (critical(point) && stats.critical_bytes.saturating_add(bytes) > 64 << 20)
            {
                return Err(Error::new("EVALUATION_BUSY", "评审写入字节队列已满"));
            }
            stats.bytes += bytes;
            if critical(point) {
                stats.critical_bytes += bytes;
            }
            stats.peak = stats.peak.max(stats.bytes);
        }
        let (reply, receive) = mpsc::sync_channel(1);
        let command = Command {
            bytes,
            queued_at: std::time::Instant::now(),
            reply,
            operation: Box::new(move |db| operation(db).map(|v| Box::new(v) as Value)),
            point,
            key: key.into(),
        };
        {
            let (lock, wake) = &*self.queue;
            let mut queue = lock.lock().map_err(lock_error)?;
            if queue.closed {
                let mut stats = self.stats.lock().map_err(lock_error)?;
                stats.bytes = stats.bytes.saturating_sub(bytes);
                if critical(point) {
                    stats.critical_bytes = stats.critical_bytes.saturating_sub(bytes);
                }
                return Err(Error::new("EVALUATION_WRITER_EXITED", "评审写入线程已退出"));
            }
            let target = if critical(point) {
                &mut queue.critical
            } else {
                &mut queue.normal
            };
            if target.len() >= 64 {
                let mut stats = self.stats.lock().map_err(lock_error)?;
                stats.bytes = stats.bytes.saturating_sub(bytes);
                if critical(point) {
                    stats.critical_bytes = stats.critical_bytes.saturating_sub(bytes);
                }
                return Err(Error::new("EVALUATION_BUSY", "评审写入队列已满"));
            }
            target.push_back(command);
            wake.notify_one();
        }
        let result = match receive.recv() {
            Ok(value) => value?,
            Err(_) => {
                self.stats.lock().map_err(lock_error)?.bytes = 0;
                return Err(Error::new("EVALUATION_WRITER_EXITED", "评审写入线程已退出"));
            }
        };
        result
            .downcast::<T>()
            .map(|v| *v)
            .map_err(|_| Error::new("INTERNAL_ERROR", "评审写入返回类型无效"))
    }
}
impl Drop for Writer {
    fn drop(&mut self) {
        if let Ok(mut q) = self.queue.0.lock() {
            q.closed = true;
            self.queue.1.notify_all();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn critical(point: &str) -> bool {
    matches!(
        point,
        "raw_receipt" | "receipt" | "receipt_parse" | "settle" | "dispatch" | "parse" | "failure"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receipts_pass_waiting_projection_writes_without_starving_them() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-runs");
        std::fs::create_dir_all(&root).unwrap();
        let dir = tempfile::Builder::new()
            .prefix("writer-priority-")
            .tempdir_in(root)
            .unwrap();
        let writer = Arc::new(Writer::open(&dir.path().join("evaluation.sqlite")).unwrap());
        let order = Arc::new(Mutex::new(Vec::new()));
        let (started, ready) = mpsc::sync_channel(1);
        let (release, held) = mpsc::sync_channel(1);
        let w = writer.clone();
        let first = std::thread::spawn(move || {
            w.submit(1024, move |_| {
                started.send(()).unwrap();
                held.recv().unwrap();
                Ok(())
            })
            .unwrap()
        });
        ready.recv().unwrap();
        let w = writer.clone();
        let rows = order.clone();
        let normal = std::thread::spawn(move || {
            w.submit(1024, move |_| {
                rows.lock().unwrap().push("projection");
                Ok(())
            })
            .unwrap()
        });
        let mut threads = Vec::new();
        for _ in 0..10 {
            let w = writer.clone();
            let rows = order.clone();
            threads.push(std::thread::spawn(move || {
                w.submit_named(1024, "raw_receipt", "", move |_| {
                    rows.lock().unwrap().push("receipt");
                    Ok(())
                })
                .unwrap()
            }));
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let q = writer.queue.0.lock().unwrap();
            let ready = q.normal.len() == 1 && q.critical.len() == 10;
            drop(q);
            if ready {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        release.send(()).unwrap();
        first.join().unwrap();
        normal.join().unwrap();
        for thread in threads {
            thread.join().unwrap();
        }
        let order = order.lock().unwrap();
        assert_eq!(order.len(), 11);
        assert_eq!(order[0], "receipt");
        assert!(order.iter().position(|v| *v == "projection").unwrap() <= 8);
        assert_eq!(writer.stats.lock().unwrap().bytes, 0);
    }
}
