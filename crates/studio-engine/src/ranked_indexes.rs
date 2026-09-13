use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};
use studio_application::{ReadResources, read_cancelled};
use studio_domain::*;
use studio_storage::{
    SqliteStore,
    ranked_index::{RankedIndex, RankedIndexPlan, RankedIndexProgress},
};
#[cfg(test)]
#[path = "ranked_indexes_tests.rs"]
mod tests;

struct Build {
    plan: RankedIndexPlan,
    cancelled: Arc<AtomicBool>,
    progress: Arc<RankedIndexProgress>,
    finished: AtomicBool,
    error: Mutex<Option<Error>>,
    touched: Mutex<Instant>,
}
#[derive(Default)]
struct State {
    jobs: HashMap<String, Arc<Build>>,
    pins: HashMap<String, usize>,
    touched: HashMap<String, (Instant, Instant)>,
    leases: HashMap<(String, String), Instant>,
}
pub struct RankedIndexes {
    root: PathBuf,
    state: Mutex<State>,
    build_gate: Mutex<()>,
    stopping: AtomicBool,
    builds: AtomicU64,
    reuses: AtomicU64,
}
pub struct IndexHandle {
    pub index: RankedIndex,
    owner: Arc<RankedIndexes>,
    key: String,
}
impl Drop for IndexHandle {
    fn drop(&mut self) {
        if let Ok(mut state) = self.owner.state.lock()
            && let Some(count) = state.pins.get_mut(&self.key)
        {
            *count -= 1;
            if *count == 0 {
                state.pins.remove(&self.key);
            }
        }
    }
}
#[derive(Default, Clone, Copy)]
pub struct Metrics {
    pub bytes: u64,
    pub working_bytes: u64,
    pub entries: u64,
    pub builds: u64,
    pub reuses: u64,
}
fn lock_error() -> Error {
    Error::new("INTERNAL_ERROR", "排名索引状态不可用")
}
fn scope_key(scope: &ScopeRef) -> Result<String> {
    serde_json::to_string(scope).map_err(Error::io)
}
fn phase(progress: &RankedIndexProgress) -> &'static str {
    match progress.phase.load(Ordering::Acquire) {
        0 => "正在等待建立当前范围的排名索引",
        1 => "正在建立当前范围的排名成员索引",
        2..=3 => "正在整理当前范围的主排名索引",
        4..=5 => "正在整理当前范围的补救排名索引",
        6..=7 => "正在整理当前范围的输入顺序索引",
        8..=9 => "正在整理当前范围的直算排名索引",
        10..=11 => "正在整理当前范围的融合排名索引",
        _ => "正在发布当前范围的排名索引",
    }
}
impl RankedIndexes {
    pub fn prune_released(&self, store: &SqliteStore) -> Result<()> {
        if !self.root.exists() {
            return Ok(());
        }
        for entry in fs::read_dir(&self.root).map_err(Error::io)?.take(256) {
            let entry = entry.map_err(Error::io)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(key) = name.strip_suffix(".sqlite") else {
                continue;
            };
            if self.path(key).is_err() {
                continue;
            }
            let Ok(meta) = RankedIndex::metadata(&entry.path()) else {
                continue;
            };
            if store
                .ranked_members_available(&meta.scope.project_id, &meta.scope)
                .ok()
                != Some(false)
            {
                continue;
            }
            let state = self.state.lock().map_err(|_| lock_error())?;
            if state.pins.contains_key(key) {
                continue;
            }
            drop(state);
            self.invalidate(key)?;
        }
        Ok(())
    }
    /// Existing alias caches can be adopted after an O(1) member-revision check.
    /// No full member scans, hashes or copies; readers pin files during rebinding.
    pub fn adopt(
        &self,
        plan: &RankedIndexPlan,
        matches: impl Fn(&studio_storage::ranked_index::RankedIndexMeta) -> Result<bool>,
    ) -> Result<()> {
        let target = self.path(&plan.meta.key)?;
        if target.is_file() || !self.root.exists() {
            return Ok(());
        }
        for entry in fs::read_dir(&self.root).map_err(Error::io)?.take(256) {
            let entry = entry.map_err(Error::io)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(key) = name.strip_suffix(".sqlite") else {
                continue;
            };
            if self.path(key).is_err() {
                continue;
            }
            let path = entry.path();
            let Ok(meta) = RankedIndex::metadata(&path) else {
                continue;
            };
            if meta.scope.project_id != plan.meta.scope.project_id {
                continue;
            }
            if meta != plan.meta && !matches(&meta).unwrap_or(false) {
                continue;
            }
            let mut state = self.state.lock().map_err(|_| lock_error())?;
            if target.is_file() {
                return Ok(());
            }
            if state.pins.contains_key(key) {
                continue;
            }
            if RankedIndex::rebind(&path, &meta, &plan.meta).is_err() {
                continue;
            }
            fs::rename(&path, &target).map_err(Error::io)?;
            state.touched.remove(key);
            return Ok(());
        }
        Ok(())
    }
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            state: Mutex::new(State::default()),
            build_gate: Mutex::new(()),
            stopping: AtomicBool::new(false),
            builds: AtomicU64::new(0),
            reuses: AtomicU64::new(0),
        }
    }
    fn path(&self, key: &str) -> Result<PathBuf> {
        if key.len() != 64
            || !key
                .bytes()
                .all(|v| v.is_ascii_hexdigit() && !v.is_ascii_uppercase())
        {
            return Err(Error::invalid("排名索引标识无效"));
        }
        Ok(self.root.join(format!("{key}.sqlite")))
    }
    pub fn lease(&self, scope: &ScopeRef, id: &str, release: bool) -> Result<()> {
        validate_id(id)?;
        scope.validate_project(&scope.project_id)?;
        let key = scope_key(scope)?;
        let mut state = self.state.lock().map_err(|_| lock_error())?;
        state
            .leases
            .retain(|_, t| t.elapsed() < Duration::from_secs(90));
        if release {
            state.leases.remove(&(key, id.into()));
        } else {
            if state.leases.len() >= 1024 && !state.leases.contains_key(&(key.clone(), id.into())) {
                return Err(Error::new("READ_BUDGET_EXCEEDED", "同时打开的排名视图过多"));
            }
            state.leases.insert((key, id.into()), Instant::now());
        }
        Ok(())
    }
    pub fn open(
        self: &Arc<Self>,
        plan: &RankedIndexPlan,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Option<IndexHandle>> {
        read_cancelled(&cancelled)?;
        let path = self.path(&plan.meta.key)?;
        let persist_touch = {
            let mut state = self.state.lock().map_err(|_| lock_error())?;
            if !path.is_file() {
                return Ok(None);
            }
            *state.pins.entry(plan.meta.key.clone()).or_default() += 1;
            let now = Instant::now();
            let touched = state
                .touched
                .entry(plan.meta.key.clone())
                .or_insert((now, now - Duration::from_secs(61)));
            touched.0 = now;
            let persist = touched.1.elapsed() >= Duration::from_secs(60);
            if persist {
                touched.1 = now;
            }
            persist
        };
        match RankedIndex::open(&path, &plan.meta, cancelled.clone()) {
            Ok(index) => {
                if persist_touch && let Ok(file) = fs::OpenOptions::new().write(true).open(&path) {
                    let _ = file.set_modified(SystemTime::now());
                }
                self.reuses.fetch_add(1, Ordering::Relaxed);
                Ok(Some(IndexHandle {
                    index,
                    owner: self.clone(),
                    key: plan.meta.key.clone(),
                }))
            }
            Err(error) => {
                let mut state = self.state.lock().map_err(|_| lock_error())?;
                if let Some(count) = state.pins.get_mut(&plan.meta.key) {
                    *count -= 1;
                    if *count == 0 {
                        state.pins.remove(&plan.meta.key);
                    }
                }
                read_cancelled(&cancelled)?;
                if error.code == "CANCELLED" {
                    return Err(error);
                }
                if !state.pins.contains_key(&plan.meta.key) {
                    let _ = fs::remove_file(path);
                    state.touched.remove(&plan.meta.key);
                }
                Ok(None)
            }
        }
    }
    pub fn metrics(&self) -> Result<Metrics> {
        let mut m = Metrics {
            builds: self.builds.load(Ordering::Relaxed),
            reuses: self.reuses.load(Ordering::Relaxed),
            ..Default::default()
        };
        if !self.root.exists() {
            return Ok(m);
        }
        for entry in fs::read_dir(&self.root).map_err(Error::io)? {
            let entry = entry.map_err(Error::io)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let size = match entry.metadata() {
                Ok(meta) if meta.is_file() => meta.len(),
                Ok(_) => continue,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(Error::io(e)),
            };
            if name.starts_with("rank-build-") {
                m.working_bytes += size;
            } else if name.ends_with(".sqlite") {
                m.bytes += size;
                m.entries += 1;
            }
        }
        Ok(m)
    }
    pub fn invalidate(&self, key: &str) -> Result<()> {
        let path = self.path(key)?;
        let mut state = self.state.lock().map_err(|_| lock_error())?;
        if !state.pins.contains_key(key) && path.is_file() {
            fs::remove_file(path).map_err(Error::io)?;
            state.touched.remove(key);
        }
        Ok(())
    }
    pub fn tick(&self) {
        if let Ok(mut state) = self.state.lock() {
            state
                .leases
                .retain(|_, t| t.elapsed() < Duration::from_secs(90));
            state.jobs.retain(|_, job| {
                !job.finished.load(Ordering::Acquire)
                    || job
                        .touched
                        .lock()
                        .is_ok_and(|t| t.elapsed() < Duration::from_secs(120))
            });
            for job in state.jobs.values() {
                let alive = scope_key(&job.plan.meta.scope)
                    .ok()
                    .is_some_and(|key| state.leases.keys().any(|(scope, _)| scope == &key));
                if !alive
                    && job
                        .touched
                        .lock()
                        .is_ok_and(|t| t.elapsed() > Duration::from_secs(5))
                {
                    job.cancelled.store(true, Ordering::Release);
                }
            }
        }
    }
    pub fn shutdown(&self) {
        self.stopping.store(true, Ordering::Release);
        if let Ok(state) = self.state.lock() {
            for job in state.jobs.values() {
                job.cancelled.store(true, Ordering::Release);
            }
        }
    }
    pub fn busy(&self) -> bool {
        self.state
            .lock()
            .is_ok_and(|s| s.jobs.values().any(|j| !j.finished.load(Ordering::Acquire)))
    }
    pub fn prune(
        &self,
        budget: u64,
        force: bool,
        idle_seconds: u64,
        session_projects: Option<&HashSet<String>>,
    ) -> Result<()> {
        if !self.root.exists() {
            return Ok(());
        }
        let mut entries = Vec::new();
        for entry in fs::read_dir(&self.root).map_err(Error::io)? {
            let entry = entry.map_err(Error::io)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(key) = name.strip_suffix(".sqlite") else {
                continue;
            };
            if self.path(key).is_err() {
                continue;
            }
            let meta = match entry.metadata() {
                Ok(meta) if meta.is_file() => meta,
                Ok(_) => continue,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(Error::io(e)),
            };
            entries.push((
                key.to_owned(),
                entry.path(),
                meta.len(),
                meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            ));
        }
        entries.sort_by_key(|e| e.3);
        let mut bytes = entries.iter().map(|e| e.2).sum::<u64>();
        let mut count = entries.len();
        for (key, path, size, modified) in entries {
            let metadata = RankedIndex::metadata(&path).ok();
            let mut state = self.state.lock().map_err(|_| lock_error())?;
            let live = metadata
                .as_ref()
                .and_then(|m| scope_key(&m.scope).ok())
                .is_some_and(|key| {
                    state.leases.iter().any(|((scope, _), at)| {
                        scope == &key && at.elapsed() < Duration::from_secs(90)
                    })
                });
            if live || state.pins.contains_key(&key) {
                continue;
            }
            let age = state
                .touched
                .get(&key)
                .map(|(used, _)| used.elapsed())
                .unwrap_or_else(|| modified.elapsed().unwrap_or_default());
            if metadata.is_some()
                && !force
                && !session_projects.is_some_and(|projects| {
                    metadata
                        .as_ref()
                        .is_some_and(|m| !projects.contains(&m.scope.project_id))
                })
                && bytes <= budget
                && count < 128
                && age < Duration::from_secs(idle_seconds)
            {
                continue;
            }
            match fs::remove_file(path) {
                Ok(()) => {
                    bytes = bytes.saturating_sub(size);
                    count -= 1;
                    state.touched.remove(&key);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => tracing::debug!(%e,"ranked index eviction deferred"),
            }
        }
        Ok(())
    }
    pub fn prepare(
        self: &Arc<Self>,
        store: Arc<SqliteStore>,
        runner: Arc<crate::query_jobs::QueryRunner>,
        resources: Arc<dyn ReadResources>,
        preview: studio_resources::PreviewCache,
        plan: RankedIndexPlan,
    ) -> Result<(String, u64)> {
        if self.stopping.load(Ordering::Acquire) {
            return Err(Error::new("CANCELLED", "引擎正在退出"));
        }
        let key = plan.meta.key.clone();
        {
            let mut state = self.state.lock().map_err(|_| lock_error())?;
            if let Some(job) = state.jobs.get(&key).cloned() {
                *job.touched.lock().map_err(|_| lock_error())? = Instant::now();
                if let Some(error) = job.error.lock().map_err(|_| lock_error())?.clone() {
                    state.jobs.remove(&key);
                    return Err(error);
                }
                if !job.finished.load(Ordering::Acquire) {
                    return Ok((
                        phase(&job.progress).into(),
                        job.progress.completed.load(Ordering::Acquire),
                    ));
                }
                // A completed job is not a cache entry: its published file may
                // already have been evicted or invalidated by the preceding read.
                state.jobs.remove(&key);
            }
            if self.path(&key)?.is_file() {
                // Publication may have won the race with open(). The next poll
                // will open that file instead of starting a duplicate build.
                return Ok(("正在打开当前范围的排名索引".into(), plan.meta.count));
            }
            state
                .jobs
                .retain(|_, j| !j.finished.load(Ordering::Acquire));
            if state.jobs.len() >= 16 {
                return Err(Error::new("READ_BUDGET_EXCEEDED", "等待排名索引的范围过多"));
            }
        }
        let job = Arc::new(Build {
            plan: plan.clone(),
            cancelled: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(RankedIndexProgress::default()),
            finished: AtomicBool::new(false),
            error: Mutex::new(None),
            touched: Mutex::new(Instant::now()),
        });
        {
            let mut state = self.state.lock().map_err(|_| lock_error())?;
            if let Some(existing) = state.jobs.get(&key) {
                return Ok((
                    phase(&existing.progress).into(),
                    existing.progress.completed.load(Ordering::Acquire),
                ));
            }
            state.jobs.insert(key.clone(), job.clone());
        }
        let owner = self.clone();
        tokio::task::spawn_blocking(move || {
            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
                    let _project = store.operation_lease(&plan.meta.scope.project_id)?;
                    let _serial = loop {
                        read_cancelled(&job.cancelled)?;
                        match owner.build_gate.try_lock() {
                            Ok(g) => break g,
                            Err(std::sync::TryLockError::WouldBlock) => {
                                std::thread::sleep(Duration::from_millis(20))
                            }
                            Err(_) => return Err(lock_error()),
                        }
                    };
                    let _budget = runner.budget.wait(&job.cancelled)?;
                    let memory = _budget.memory_bytes.min(512 << 20);
                    let _permit = resources.acquire(
                        ReadRequest {
                            class: ReadClass::NativeQuery,
                            priority: ReadPriority::Background,
                            bytes: memory,
                        },
                        &job.cancelled,
                    )?;
                    store.refresh_query_cache_sizes(&plan.meta.scope.project_id)?;
                    runner.cache.track(&store, &plan.meta.scope.project_id)?;
                    let budget = available_budget(&runner, &preview)?;
                    let config = runner.cache.config()?;
                    let sessions = if config.temporary_session_only {
                        Some(live_session_projects(&runner)?)
                    } else {
                        None
                    };
                    owner.prune(
                        budget,
                        false,
                        u64::from(config.temporary_idle_hours) * 3600,
                        sessions.as_ref(),
                    )?;
                    let metrics = owner.metrics()?;
                    if metrics.entries >= 128 {
                        return Err(Error::new(
                            "CACHE_BUDGET_EXCEEDED",
                            "排名索引数量已达缓存上限",
                        ));
                    }
                    let maximum = ((budget.saturating_sub(metrics.bytes)) / 2).min(8 << 30);
                    fs::create_dir_all(&owner.root).map_err(Error::io)?;
                    let candidate = tempfile::Builder::new()
                        .prefix("rank-build-")
                        .suffix(".sqlite")
                        .tempfile_in(&owner.root)
                        .map_err(Error::io)?;
                    RankedIndex::build(
                        candidate.path(),
                        &plan,
                        memory,
                        maximum,
                        job.cancelled.clone(),
                        job.progress.clone(),
                    )?;
                    let live = store
                        .ranked_scope(&plan.meta.scope.project_id, &plan.requested_scope)?
                        .ok_or_else(|| Error::new("RANKING_SCOPE_UNSUPPORTED", "排名范围已移除"))?;
                    if live.count != plan.meta.count {
                        return Err(Error::new("SOURCE_CHANGED", "排名范围成员已变化"));
                    }
                    read_cancelled(&job.cancelled)?;
                    let current = available_budget(&runner, &preview)?;
                    let length = candidate.as_file().metadata().map_err(Error::io)?.len();
                    if owner.metrics()?.bytes.saturating_add(length) > current {
                        return Err(Error::new(
                            "CACHE_BUDGET_EXCEEDED",
                            "缓存预算已变化，未发布排名索引",
                        ));
                    }
                    candidate.as_file().sync_all().map_err(Error::io)?;
                    let _state = owner.state.lock().map_err(|_| lock_error())?;
                    candidate.persist(owner.path(&key)?).map_err(Error::io)?;
                    owner.builds.fetch_add(1, Ordering::Relaxed);
                    Ok(())
                }))
                .unwrap_or_else(|_| Err(Error::new("INTERNAL_ERROR", "排名索引准备异常结束")));
            if let Err(error) = outcome {
                if let Ok(mut state) = job.error.lock() {
                    *state = Some(error.clone());
                }
                tracing::warn!(%key,%error,"ranked scope index build stopped");
            }
            job.finished.store(true, Ordering::Release);
            runner.cache.requested.store(true, Ordering::Release);
        });
        Ok(("正在准备当前范围的排名索引".into(), 0))
    }
}
pub fn available_budget(
    runner: &crate::query_jobs::QueryRunner,
    preview: &studio_resources::PreviewCache,
) -> Result<u64> {
    let config = runner.cache.config()?;
    let projects = runner.cache.projects()?;
    let temporary = projects.iter().map(|p| p.temporary_bytes).sum::<u64>();
    let other = projects.iter().map(|p| p.bytes).sum::<u64>()
        + runner.browse_index.storage()?.0
        + runner.rating_cache.storage_bytes()?
        + preview.metrics().bytes;
    Ok((u64::from(config.temporary_mib) << 20)
        .saturating_sub(temporary)
        .min((u64::from(config.total_mib) << 20).saturating_sub(other)))
}
pub fn live_session_projects(runner: &crate::query_jobs::QueryRunner) -> Result<HashSet<String>> {
    let mut projects = HashSet::new();
    for p in runner.cache.projects()? {
        if !runner.cache.live_sessions(&p.id)?.is_empty() {
            projects.insert(p.id);
        }
    }
    Ok(projects)
}
