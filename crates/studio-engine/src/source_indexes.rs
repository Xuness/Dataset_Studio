use crate::sources::SourceRead;
use crate::{query_budget::QueryBudget, query_cache::CacheControl, sources::SourceService};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use studio_application::SourceAdapter;
use studio_domain::*;
use studio_sources::{BrowseIndex, BrowseIndexStamp, IdentityIndex, RatingCache};
type IndexJobs = HashMap<String, (Arc<AtomicBool>, Option<Error>)>;
#[derive(Debug, Clone)]
pub struct RatingBuildStatus {
    pub source_id: String,
    pub state: String,
    pub current_rating: Option<String>,
    pub completed: Vec<String>,
    pub error: Option<String>,
}
type RatingBuilds = HashMap<String, (Arc<AtomicBool>, RatingBuildStatus)>;
pub struct SourceIndexHandles {
    pub browse: BrowseIndex,
    pub identities: Arc<IdentityIndex>,
    pub ratings: Arc<RatingCache>,
}
impl SourceIndexHandles {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            browse: BrowseIndex::new(directory.clone()),
            identities: Arc::new(IdentityIndex::new(directory.clone())),
            ratings: Arc::new(RatingCache::new(directory.with_file_name("rating-cache"))),
        }
    }
}
pub struct SourceIndexService {
    indexes: Mutex<IndexJobs>,
    rating_builds: Mutex<RatingBuilds>,
    sources: Arc<SourceService>,
    budget: Arc<QueryBudget>,
    cache: Arc<CacheControl>,
    pub browse_index: BrowseIndex,
    pub identity_index: Arc<IdentityIndex>,
    pub rating_cache: Arc<RatingCache>,
    query_directory: PathBuf,
    workers: Arc<tokio::sync::Semaphore>,
}
impl SourceIndexService {
    pub fn new(
        handles: SourceIndexHandles,
        sources: Arc<SourceService>,
        budget: Arc<QueryBudget>,
        cache: Arc<CacheControl>,
        query_directory: PathBuf,
    ) -> Arc<Self> {
        Arc::new(Self {
            indexes: Mutex::new(HashMap::new()),
            rating_builds: Mutex::new(HashMap::new()),
            sources,
            budget,
            cache,
            query_directory,
            browse_index: handles.browse,
            identity_index: handles.identities,
            rating_cache: handles.ratings,
            workers: Arc::new(tokio::sync::Semaphore::new(1)),
        })
    }
    fn schedule(self: &Arc<Self>, work: impl FnOnce() + Send + 'static) {
        let workers = self.workers.clone();
        tokio::spawn(async move {
            if let Ok(permit) = workers.acquire_owned().await {
                let _ = tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    work();
                })
                .await;
            }
        });
    }
    pub fn ensure_browse_index(
        &self,
        source: &Source,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Option<BrowseIndexStamp>> {
        if !self.sources.has(source, |c| c.post_order) || self.browse_index.is_current(source)? {
            return Ok(None);
        }
        let budget = self.budget.wait(&cancelled)?;
        let _permit = self.sources.background(
            ReadClass::NativeQuery,
            budget.memory_bytes,
            cancelled.clone(),
        )?;
        self.browse_index
            .ensure(
                source,
                budget.memory_bytes,
                &self.query_directory,
                cancelled,
            )
            .map(Some)
    }
    pub fn prepare_browse_index(
        self: &Arc<Self>,
        source: &Source,
        read: &SourceRead,
    ) -> Result<bool> {
        if !self.sources.has(source, |c| c.post_order) || self.browse_index.is_current(source)? {
            return Ok(true);
        }
        let revision = read.probe(source)?.revision;
        let key = format!("browse:{}:{revision}:v1", source.id);
        let mut jobs = self
            .indexes
            .lock()
            .map_err(|_| Error::new("INTERNAL_ERROR", "排序准备状态不可用"))?;
        if let Some((_, error)) = jobs.get(&key) {
            if error.is_some() {
                return Err(jobs
                    .remove(&key)
                    .and_then(|(_, e)| e)
                    .expect("present error"));
            }
            return Ok(false);
        }
        if jobs.len() >= 32 {
            return Err(Error::new("READ_BUDGET_EXCEEDED", "等待排序准备的来源过多"));
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        jobs.insert(key.clone(), (cancelled.clone(), None));
        let runner = self.clone();
        let source = source.clone();
        self.schedule(move || {
            let result = runner.ensure_browse_index(&source, cancelled);
            if let Ok(mut jobs) = runner.indexes.lock() {
                if let Err(error) = result {
                    if let Some((_, state)) = jobs.get_mut(&key) {
                        *state = Some(error);
                    }
                } else {
                    jobs.remove(&key);
                }
            }
        });
        Ok(false)
    }
    pub fn prepare_identity_index(
        self: &Arc<Self>,
        source: &Source,
        read: &SourceRead,
    ) -> Result<bool> {
        if !self.sources.has(source, |c| c.post_order) || self.identity_index.is_current(source)? {
            return Ok(true);
        }
        let key = format!("identity:{}:{}:v1", source.id, read.probe(source)?.revision);
        let mut jobs = self
            .indexes
            .lock()
            .map_err(|_| Error::new("INTERNAL_ERROR", "身份准备状态不可用"))?;
        if let Some((_, error)) = jobs.get(&key) {
            if error.is_some() {
                return Err(jobs
                    .remove(&key)
                    .and_then(|(_, e)| e)
                    .expect("present error"));
            }
            return Ok(false);
        }
        if jobs.len() >= 32 {
            return Err(Error::new("READ_BUDGET_EXCEEDED", "等待身份准备的来源过多"));
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        jobs.insert(key.clone(), (cancelled.clone(), None));
        let runner = self.clone();
        let source = source.clone();
        self.schedule(move || {
            let result = (|| {
                let budget = runner.budget.wait(&cancelled)?;
                let _permit = runner.sources.background(
                    ReadClass::NativeQuery,
                    budget.memory_bytes,
                    cancelled.clone(),
                )?;
                runner.identity_index.ensure(
                    &source,
                    budget.memory_bytes,
                    &runner.query_directory,
                    cancelled,
                )
            })();
            if let Ok(mut jobs) = runner.indexes.lock() {
                match result {
                    Ok(()) => {
                        jobs.remove(&key);
                        runner.cache.requested.store(true, Ordering::Release);
                    }
                    Err(error) => {
                        if let Some((_, state)) = jobs.get_mut(&key) {
                            *state = Some(error);
                        }
                    }
                }
            }
        });
        Ok(false)
    }
    pub fn rating_builds(&self) -> Result<Vec<RatingBuildStatus>> {
        self.rating_builds
            .lock()
            .map(|jobs| jobs.values().map(|(_, status)| status.clone()).collect())
            .map_err(|_| Error::new("INTERNAL_ERROR", "基础缓存任务状态不可用"))
    }
    pub fn cancel_rating_build(&self, source: &str) -> Result<()> {
        validate_id(source)?;
        if let Some((cancelled, _)) = self
            .rating_builds
            .lock()
            .map_err(|_| Error::new("INTERNAL_ERROR", "基础缓存任务状态不可用"))?
            .get(source)
        {
            cancelled.store(true, Ordering::Release);
        }
        Ok(())
    }
    pub fn start_rating_build(self: &Arc<Self>, source: &Source) -> Result<RatingBuildStatus> {
        if !self.sources.has(source, |c| c.post_order) {
            return Err(Error::new("QUERY_UNSUPPORTED", "此来源不支持分级基础缓存"));
        }
        if self.cache.config()?.long_term_mib == 0 {
            return Err(Error::invalid("请先为长期缓存配置容量"));
        }
        let initial = RatingBuildStatus {
            source_id: source.id.clone(),
            state: "queued".into(),
            current_rating: None,
            completed: Vec::new(),
            error: None,
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        {
            let mut jobs = self
                .rating_builds
                .lock()
                .map_err(|_| Error::new("INTERNAL_ERROR", "基础缓存任务状态不可用"))?;
            if let Some((_, status)) = jobs.get(&source.id)
                && matches!(status.state.as_str(), "queued" | "running")
            {
                return Ok(status.clone());
            }
            if jobs.len() >= 64 {
                jobs.retain(|_, (_, status)| matches!(status.state.as_str(), "queued" | "running"));
            }
            if jobs.len() >= 64 {
                return Err(Error::new("RESOURCE_LIMIT", "基础缓存构建任务过多"));
            }
            jobs.insert(source.id.clone(), (cancelled.clone(), initial.clone()));
        }
        let runner = self.clone();
        let source = source.clone();
        self.schedule(move || {
            let outcome = (|| -> Result<()> {
                let ratings = studio_sources::RATINGS.map(str::to_owned);
                let _pins = runner.rating_cache.pin(&source.id, &ratings)?;
                for rating in ratings {
                    studio_application::read_cancelled(&cancelled)?;
                    if let Ok(mut jobs) = runner.rating_builds.lock()
                        && let Some((_, status)) = jobs.get_mut(&source.id)
                    {
                        status.state = "running".into();
                        status.current_rating = Some(rating.clone());
                    }
                    let budget = runner.budget.wait(&cancelled)?;
                    let _permit = runner.sources.background(
                        ReadClass::NativeQuery,
                        budget.memory_bytes,
                        cancelled.clone(),
                    )?;
                    runner.rating_cache.ensure(
                        &source,
                        &rating,
                        budget.memory_bytes,
                        &runner.query_directory,
                        cancelled.clone(),
                    )?;
                    if let Ok(mut jobs) = runner.rating_builds.lock()
                        && let Some((_, status)) = jobs.get_mut(&source.id)
                    {
                        status.completed.push(rating);
                    }
                }
                Ok(())
            })();
            if let Ok(mut jobs) = runner.rating_builds.lock()
                && let Some((_, status)) = jobs.get_mut(&source.id)
            {
                status.current_rating = None;
                match outcome {
                    Ok(()) => status.state = "ready".into(),
                    Err(error) => {
                        status.state = if error.code == "CANCELLED" {
                            "cancelled"
                        } else {
                            "failed"
                        }
                        .into();
                        status.error = Some(error.message);
                    }
                }
            }
            runner.cache.requested.store(true, Ordering::Release);
        });
        Ok(initial)
    }
    pub fn shutdown(&self) {
        self.workers.close();
        if let Ok(jobs) = self.rating_builds.lock() {
            for (cancel, _) in jobs.values() {
                cancel.store(true, Ordering::Release);
            }
        }
        if let Ok(jobs) = self.indexes.lock() {
            for (cancel, _) in jobs.values() {
                cancel.store(true, Ordering::Release);
            }
        }
    }
}
