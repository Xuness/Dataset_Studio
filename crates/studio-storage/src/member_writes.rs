use crate::*;
use std::time::{Duration, Instant};

struct Operation {
    cancelled: Arc<AtomicBool>,
    active: AtomicBool,
    started: Instant,
    progress: Mutex<MemberWriteProgress>,
}
#[derive(Default)]
pub(super) struct MemberWrites {
    entries: Mutex<HashMap<(String, String), Arc<Operation>>>,
}
pub(super) struct MemberWrite {
    operation: Arc<Operation>,
    finished: bool,
}
fn waiting() -> MemberWriteProgress {
    MemberWriteProgress {
        state: "waiting".into(),
        completed: 0,
        total: None,
        error: None,
    }
}
impl MemberWrites {
    fn room(entries: &mut HashMap<(String, String), Arc<Operation>>) -> Result<()> {
        entries.retain(|_, v| {
            v.active.load(Ordering::Acquire) || v.started.elapsed() < Duration::from_secs(900)
        });
        if entries.len() >= 64 {
            if let Some(key) = entries
                .iter()
                .filter(|(_, v)| !v.active.load(Ordering::Acquire))
                .min_by_key(|(_, v)| v.started)
                .map(|(k, _)| k.clone())
            {
                entries.remove(&key);
            } else {
                return Err(Error::new("READ_BUDGET_EXCEEDED", "正在进行的保存操作过多"));
            }
        }
        Ok(())
    }
    fn start(&self, pid: &str, id: &str) -> Result<MemberWrite> {
        validate_id(id)?;
        let mut entries = self.entries.lock().map_err(lock_error)?;
        let key = (pid.into(), id.into());
        if let Some(old) = entries.get(&key) {
            if old.cancelled.load(Ordering::Acquire)
                && old.progress.lock().map_err(lock_error)?.state != "complete"
            {
                return Err(Error::new("CANCELLED", "保存已取消"));
            }
            if old.active.load(Ordering::Acquire) {
                let old = old.clone();
                drop(entries);
                while old.active.load(Ordering::Acquire) {
                    studio_application::read_cancelled(&old.cancelled)?;
                    std::thread::sleep(Duration::from_millis(20));
                }
                return self.start(pid, id);
            }
        }
        Self::room(&mut entries)?;
        let operation = Arc::new(Operation {
            cancelled: Arc::new(AtomicBool::new(false)),
            active: AtomicBool::new(true),
            started: Instant::now(),
            progress: Mutex::new(waiting()),
        });
        entries.insert(key, operation.clone());
        Ok(MemberWrite {
            operation,
            finished: false,
        })
    }
    fn progress(&self, pid: &str, id: &str) -> Result<MemberWriteProgress> {
        validate_id(id)?;
        let entry = self
            .entries
            .lock()
            .map_err(lock_error)?
            .get(&(pid.into(), id.into()))
            .cloned();
        entry
            .map(|op| op.progress.lock().map(|p| p.clone()).map_err(lock_error))
            .unwrap_or_else(|| Ok(waiting()))
    }
    fn cancel(&self, pid: &str, id: &str) -> Result<()> {
        validate_id(id)?;
        let mut entries = self.entries.lock().map_err(lock_error)?;
        let key = (pid.into(), id.into());
        if let Some(entry) = entries.get(&key) {
            let mut progress = entry.progress.lock().map_err(lock_error)?;
            if entry.active.load(Ordering::Acquire) && progress.state != "complete" {
                entry.cancelled.store(true, Ordering::Release);
                progress.state = "cancelling".into();
            }
        } else {
            Self::room(&mut entries)?;
            entries.insert(
                key,
                Arc::new(Operation {
                    cancelled: Arc::new(AtomicBool::new(true)),
                    active: AtomicBool::new(false),
                    started: Instant::now(),
                    progress: Mutex::new(MemberWriteProgress {
                        state: "cancelled".into(),
                        ..waiting()
                    }),
                }),
            );
        }
        Ok(())
    }
}
impl MemberWrite {
    pub fn cancelled(&self) -> Arc<AtomicBool> {
        self.operation.cancelled.clone()
    }
    pub fn update(&self, completed: u64, total: Option<u64>) {
        if let Ok(mut progress) = self.operation.progress.lock() {
            if !self.operation.cancelled.load(Ordering::Acquire) {
                progress.state = "saving".into();
            }
            progress.completed = completed;
            progress.total = total;
        }
    }
    pub fn finish<T>(mut self, result: Result<T>) -> Result<T> {
        let result = if result.is_err() && self.operation.cancelled.load(Ordering::Acquire) {
            Err(Error::new("CANCELLED", "保存已取消，没有发布未完成的成员"))
        } else {
            result
        };
        if let Ok(mut progress) = self.operation.progress.lock() {
            match &result {
                Ok(_) => {
                    progress.state = "complete".into();
                    progress.error = None;
                }
                Err(error) => {
                    progress.state = if error.code == "CANCELLED" {
                        "cancelled"
                    } else {
                        "failed"
                    }
                    .into();
                    progress.error = Some(error.to_string());
                }
            }
        }
        self.finished = true;
        self.operation.active.store(false, Ordering::Release);
        result
    }
}
impl Drop for MemberWrite {
    fn drop(&mut self) {
        if !self.finished {
            if let Ok(mut progress) = self.operation.progress.lock() {
                progress.state = if self.operation.cancelled.load(Ordering::Acquire) {
                    "cancelled"
                } else {
                    "failed"
                }
                .into();
                progress.error = Some("保存未完成，可重试".into());
            }
            self.operation.active.store(false, Ordering::Release);
        }
    }
}
impl SqliteStore {
    pub(super) fn begin_member_write(&self, pid: &str, id: &str) -> Result<MemberWrite> {
        self.member_writes.start(pid, id)
    }
    pub fn member_write_progress(&self, pid: &str, id: &str) -> Result<MemberWriteProgress> {
        self.handle(pid)?;
        self.member_writes.progress(pid, id)
    }
    pub fn cancel_member_write(&self, pid: &str, id: &str) -> Result<()> {
        self.handle(pid)?;
        self.member_writes.cancel(pid, id)
    }
    pub fn stop_member_writes(&self) {
        if let Ok(entries) = self.member_writes.entries.lock() {
            for op in entries.values() {
                op.cancelled.store(true, Ordering::Release);
            }
        }
    }
    pub(super) fn annotate_member_write(&self, mut job: Job) -> Job {
        if let Ok(progress) = self.member_writes.progress(&job.project_id, &job.id)
            && matches!(progress.state.as_str(), "saving" | "cancelling")
        {
            job.stage = Some(JobStage {
                name: "fixing_members".into(),
                completed: progress.completed,
                total: progress.total.unwrap_or(0),
                ..Default::default()
            });
        }
        job
    }
}
