use super::db_error;
use crate::lock_error;
use std::{
    any::Any,
    path::Path,
    sync::{Arc, Mutex, mpsc},
    thread::JoinHandle,
};
use studio_domain::{Error, Result};

type Value = Box<dyn Any + Send>;
type Operation = Box<dyn FnOnce(&rusqlite::Transaction<'_>) -> Result<Value> + Send>;
struct Command {
    bytes: usize,
    operation: Operation,
    reply: mpsc::SyncSender<Result<Value>>,
    point: &'static str,
    key: String,
}
#[derive(Default)]
pub(super) struct QueueStats {
    pub bytes: usize,
    pub peak: usize,
}
pub(super) struct Writer {
    sender: Option<mpsc::SyncSender<Command>>,
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
            if version > 3 || (version == 0 && occupied) {
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
        if version > 0 && version < 3 {
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
                "evaluation-v{version}-to-v3-{}-{}.sqlite",
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
        if version < 3 {
            let tx = db.transaction().map_err(db_error)?;
            if version == 0 {
                tx.execute_batch(include_str!("schema.sql"))
                    .map_err(db_error)?;
            }
            if version < 2 {
                tx.execute_batch(include_str!("schema_v2.sql"))
                    .map_err(db_error)?;
            }
            tx.execute_batch(include_str!("schema_v3.sql"))
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
        } else if version != 3 {
            return Err(Error::new("FORMAT_UNSUPPORTED", "评审账本版本不兼容"));
        }
        // A new writer is created only under the exclusive project lease.
        let tx = db.transaction().map_err(db_error)?;
        tx.execute_batch("UPDATE attempts SET state='outcome_unknown' WHERE state='sent';
          UPDATE batches SET state='outcome_unknown',error='引擎中断；上游可能已受理，请核对后明确选择重试' WHERE state='sent';
          UPDATE batches SET state='queued' WHERE state='preparing';
          UPDATE stages SET state='paused',error='执行已中断；本地结果可以重新解析，远端请求不会自动重发' WHERE state IN ('preparing','running','pausing');
          UPDATE stages SET state='cancelled' WHERE state='cancelling';
          UPDATE stages SET unknown=(SELECT count(*) FROM batches b WHERE b.stage_id=stages.id AND b.state='outcome_unknown');
          UPDATE analysis_jobs SET state='interrupted',error='引擎中断；可从冻结证据重算，不会调用远端模型' WHERE state IN ('queued','running','cancelling');").map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        let (sender, receiver) = mpsc::sync_channel::<Command>(64);
        let stats = Arc::new(Mutex::new(QueueStats::default()));
        let shared = stats.clone();
        let thread = std::thread::Builder::new()
            .name("aesthetic-writer".into())
            .spawn(move || {
                while let Ok(command) = receiver.recv() {
                    let result = (|| {
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
                    })();
                    if let Ok(mut stats) = shared.lock() {
                        stats.bytes = stats.bytes.saturating_sub(command.bytes);
                    }
                    // Successful acknowledgement is sent only after the durable commit.
                    let _ = command.reply.send(result);
                }
                let _ = db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");
            })
            .map_err(Error::io)?;
        Ok(Self {
            sender: Some(sender),
            thread: Some(thread),
            stats,
        })
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
            if bytes > 32 << 20 || stats.bytes.saturating_add(bytes) > 64 << 20 {
                return Err(Error::new("EVALUATION_BUSY", "评审写入字节队列已满"));
            }
            stats.bytes += bytes;
            stats.peak = stats.peak.max(stats.bytes);
        }
        let (reply, receive) = mpsc::sync_channel(1);
        let command = Command {
            bytes,
            reply,
            operation: Box::new(move |db| operation(db).map(|v| Box::new(v) as Value)),
            point,
            key: key.into(),
        };
        if let Err(error) = self
            .sender
            .as_ref()
            .expect("writer alive")
            .try_send(command)
        {
            let mut stats = self.stats.lock().map_err(lock_error)?;
            stats.bytes = stats.bytes.saturating_sub(bytes);
            if matches!(&error, mpsc::TrySendError::Disconnected(_)) {
                stats.bytes = 0;
            }
            return Err(match error {
                mpsc::TrySendError::Full(_) => Error::new("EVALUATION_BUSY", "评审写入队列已满"),
                mpsc::TrySendError::Disconnected(_) => Error::new(
                    "EVALUATION_WRITER_EXITED",
                    "评审写入线程已退出，需要重新打开项目",
                ),
            });
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
        self.sender.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
