use crate::{db_error, lock_error};
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
            if version > 1 || (version == 0 && occupied) {
                return Err(Error::new("FORMAT_UNSUPPORTED", "评审账本版本不兼容"));
            }
        }
        let mut db = crate::connection(path)?;
        let version: u32 = db
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(db_error)?;
        if version == 0 {
            let tx = db.transaction().map_err(db_error)?;
            tx.execute_batch(include_str!("schema.sql"))
                .map_err(db_error)?;
            tx.commit().map_err(db_error)?;
        } else if version != 1 {
            return Err(Error::new("FORMAT_UNSUPPORTED", "评审账本版本不兼容"));
        }
        // A new writer is created only under the exclusive project lease.
        let tx = db.transaction().map_err(db_error)?;
        tx.execute_batch("UPDATE attempts SET state='outcome_unknown' WHERE state='sent';
          UPDATE batches SET state='outcome_unknown',error='引擎中断；上游可能已受理，请核对后明确选择重试' WHERE state='sent';
          UPDATE batches SET state='queued' WHERE state='preparing';
          UPDATE stages SET state='paused',error='执行已中断；本地结果可以重新解析，远端请求不会自动重发' WHERE state IN ('preparing','running','pausing');
          UPDATE stages SET state='cancelled' WHERE state='cancelling';
          UPDATE stages SET unknown=(SELECT count(*) FROM batches b WHERE b.stage_id=stages.id AND b.state='outcome_unknown');").map_err(db_error)?;
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
                        tx.commit().map_err(db_error)?;
                        Ok(value)
                    })();
                    if let Ok(mut stats) = shared.lock() {
                        stats.bytes -= command.bytes;
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
        };
        if self
            .sender
            .as_ref()
            .expect("writer alive")
            .try_send(command)
            .is_err()
        {
            self.stats.lock().map_err(lock_error)?.bytes -= bytes;
            return Err(Error::new("EVALUATION_BUSY", "评审写入队列已满或已关闭"));
        }
        let result = receive
            .recv()
            .map_err(|_| Error::new("DATABASE_ERROR", "评审写入线程已退出"))??;
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
