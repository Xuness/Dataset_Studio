use std::{
    cell::{Cell, RefCell},
    time::{Duration, Instant},
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use studio_application::{QueryAdapter, ReadResources};
use studio_domain::*;
use studio_sources::{
    BrowseIndex, BrowseIndexStamp, IdentityIndex, QueryReader, RatingCache, rating_candidates,
};
use studio_storage::{QueryStage, SqliteStore};

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
pub struct QueryRunner {
    pub reader: QueryReader,
    running: Mutex<HashMap<String, Arc<AtomicBool>>>,
    indexes: Mutex<IndexJobs>,
    rating_builds: Mutex<RatingBuilds>,
    stopping: AtomicBool,
    resources: Arc<dyn ReadResources>,
    pub budget: crate::query_budget::QueryBudget,
    pub cache: crate::query_cache::CacheControl,
    pub browse_index: BrowseIndex,
    pub identity_index: Arc<IdentityIndex>,
    pub ranked_indexes: Arc<crate::ranked_indexes::RankedIndexes>,
    pub rating_cache: Arc<RatingCache>,
    query_directory: std::path::PathBuf,
}
impl QueryRunner {
    pub fn new(
        resources: Arc<dyn ReadResources>,
        query_directory: std::path::PathBuf,
        budget: crate::query_budget::QueryBudget,
        cache: crate::query_cache::CacheControl,
        index_directory: std::path::PathBuf,
    ) -> Self {
        Self {
            reader: QueryReader::with_query_directory(query_directory.clone()),
            running: Mutex::new(HashMap::new()),
            indexes: Mutex::new(HashMap::new()),
            rating_builds: Mutex::new(HashMap::new()),
            stopping: AtomicBool::new(false),
            resources,
            budget,
            cache,
            rating_cache: Arc::new(RatingCache::new(
                index_directory.with_file_name("rating-cache"),
            )),
            identity_index: Arc::new(IdentityIndex::new(index_directory.clone())),
            ranked_indexes: Arc::new(crate::ranked_indexes::RankedIndexes::new(
                index_directory.with_file_name("ranked-index"),
            )),
            browse_index: BrowseIndex::new(index_directory),
            query_directory,
        }
    }
    pub fn ensure_browse_index(
        &self,
        source: &Source,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Option<BrowseIndexStamp>> {
        if source.kind != "danbooru" || self.browse_index.is_current(source)? {
            return Ok(None);
        }
        let budget = self.budget.wait(&cancelled)?;
        let _permit = self.resources.acquire(
            ReadRequest {
                class: ReadClass::NativeQuery,
                priority: ReadPriority::Background,
                bytes: budget.memory_bytes,
            },
            &cancelled,
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
    pub fn prepare_browse_index(self: &Arc<Self>, source: &Source) -> Result<bool> {
        if source.kind != "danbooru" || self.browse_index.is_current(source)? {
            return Ok(true);
        }
        let mut jobs = self
            .indexes
            .lock()
            .map_err(|_| Error::new("INTERNAL_ERROR", "排序准备状态不可用"))?;
        if let Some((_, error)) = jobs.get(&source.id) {
            if error.is_some() {
                return Err(jobs
                    .remove(&source.id)
                    .and_then(|(_, e)| e)
                    .expect("present error"));
            }
            return Ok(false);
        }
        if jobs.len() >= 32 {
            return Err(Error::new("READ_BUDGET_EXCEEDED", "等待排序准备的来源过多"));
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        jobs.insert(source.id.clone(), (cancelled.clone(), None));
        let runner = self.clone();
        let source = source.clone();
        tokio::task::spawn_blocking(move || {
            let result = runner.ensure_browse_index(&source, cancelled);
            if let Ok(mut jobs) = runner.indexes.lock() {
                if let Err(error) = result {
                    if let Some((_, state)) = jobs.get_mut(&source.id) {
                        *state = Some(error);
                    }
                } else {
                    jobs.remove(&source.id);
                }
            }
        });
        Ok(false)
    }
    pub fn prepare_identity_index(self: &Arc<Self>, source: &Source) -> Result<bool> {
        if source.kind != "danbooru" || self.identity_index.is_current(source)? {
            return Ok(true);
        }
        let key = format!("identity:{}", source.id);
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
        tokio::task::spawn_blocking(move || {
            let result = (|| {
                let budget = runner.budget.wait(&cancelled)?;
                let _permit = runner.resources.acquire(
                    ReadRequest {
                        class: ReadClass::NativeQuery,
                        priority: ReadPriority::Background,
                        bytes: budget.memory_bytes,
                    },
                    &cancelled,
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
    pub fn cancel(&self, id: &str) {
        if let Ok(running) = self.running.lock()
            && let Some(cancel) = running.get(id)
        {
            cancel.store(true, Ordering::Release);
        }
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
        if source.kind != "danbooru" {
            return Err(Error::new(
                "QUERY_UNSUPPORTED",
                "分级基础缓存用于 Danbooru 数据湖",
            ));
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
        tokio::task::spawn_blocking(move || {
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
                    let _permit = runner.resources.acquire(
                        ReadRequest {
                            class: ReadClass::NativeQuery,
                            priority: ReadPriority::Background,
                            bytes: budget.memory_bytes,
                        },
                        &cancelled,
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
        self.stopping.store(true, Ordering::Release);
        self.ranked_indexes.shutdown();
        if let Ok(jobs) = self.rating_builds.lock() {
            for (cancelled, _) in jobs.values() {
                cancelled.store(true, Ordering::Release);
            }
        }
        if let Ok(running) = self.running.lock() {
            for cancelled in running.values() {
                cancelled.store(true, Ordering::Release);
            }
        }
        if let Ok(indexes) = self.indexes.lock() {
            for (cancelled, _) in indexes.values() {
                cancelled.store(true, Ordering::Release);
            }
        }
    }
    pub fn versions(
        &self,
        store: &SqliteStore,
        pid: &str,
        spec: &QuerySpec,
    ) -> Result<Vec<QuerySourceVersion>> {
        store.validate_derived(pid, spec)?;
        let native = studio_storage::native_spec(spec);
        spec.source_ids
            .iter()
            .map(|id| self.reader.query_version(&store.source(pid, id)?, &native))
            .collect()
    }
    pub fn validate_result(&self, store: &SqliteStore, result: &QueryResult) -> Result<()> {
        if result.state != ResultState::Ready {
            return Err(Error::new(
                match result.state {
                    ResultState::Failed => "SCOPE_SORT_FAILED",
                    ResultState::Cancelled => "CANCELLED",
                    ResultState::Interrupted => "INTERRUPTED",
                    _ => "RESULT_NOT_READY",
                },
                result
                    .error
                    .clone()
                    .unwrap_or_else(|| "结果尚未完整构建或已释放".into()),
            ));
        }
        store.validate_derived(&result.project_id, &result.spec)?;
        if !result.spec.uses_only_fixed_project_data()
            && self.versions(store, &result.project_id, &result.spec)? != result.source_versions
        {
            return Err(Error::new(
                "SOURCE_CHANGED",
                "来源版本已变化；已有结果保留固定成员，请重新计算后使用查询范围",
            ));
        }
        Ok(())
    }
    fn build(
        &self,
        store: &SqliteStore,
        result: &QueryResult,
        cancelled: Arc<AtomicBool>,
    ) -> Result<()> {
        let _lease = store.operation_lease(&result.project_id)?;
        let fixed = result.spec.uses_only_fixed_project_data();
        let sources = result
            .source_versions
            .iter()
            .map(|v| store.source(&result.project_id, &v.source_id))
            .collect::<Result<Vec<_>>>()?;
        for source in &sources {
            if !fixed && source.kind == "danbooru" {
                store.query_build_phase(&result.project_id, &result.id, "index")?;
                if let Err(error) = self.ensure_browse_index(source, cancelled.clone())
                    && (studio_storage::native_spec(&result.spec).uses_metadata()
                        || result.spec.order.by_post()
                        || !matches!(error.code, "IO_ERROR" | "SOURCE_UNAVAILABLE"))
                {
                    return Err(error);
                }
            }
        }
        let budget = self.budget.wait(&cancelled)?;
        let work_memory = crate::query_budget::result_work_memory(budget.memory_bytes);
        let reader = QueryReader::with_query_directory(self.query_directory.clone())
            .with_query_memory(budget.memory_bytes - work_memory);
        let retain_bases = self.cache.config()?.long_term_mib > 0;
        let reader = if retain_bases {
            reader.with_rating_cache(self.rating_cache.clone())
        } else {
            reader
        };
        let basis = store.cached_basis(&result.project_id, &result.id)?;
        let post_refresh = basis
            .as_ref()
            .map(|b| store.query_post_order_ready(&result.project_id, &b.id))
            .transpose()?
            .is_some_and(|ready| !ready);
        let stage = RefCell::new(QueryStage::with_memory(&self.query_directory, work_memory)?);
        let last_progress = Cell::new(Instant::now());
        let mut mode = if basis.is_some() {
            "incremental"
        } else {
            "full"
        };
        store.query_build_phase(&result.project_id, &result.id, mode)?;
        let input_count = store.query_input_count(&result.project_id, &result.spec)?;
        let _permit = self.resources.acquire(
            ReadRequest {
                class: ReadClass::NativeQuery,
                priority: ReadPriority::Background,
                bytes: budget.memory_bytes,
            },
            &cancelled,
        )?;
        let mut basis_pins = Vec::new();
        let ranking = crate::ranking_query::RankingQuery::open(
            store,
            &result.project_id,
            &result.spec,
            cancelled.clone(),
        )?;
        for (expected, source) in result.source_versions.iter().zip(&sources) {
            if retain_bases
                && source.kind == "danbooru"
                && let Some(ratings) = rating_candidates(&studio_storage::native_spec(&result.spec))
            {
                basis_pins.push(self.rating_cache.pin(&source.id, &ratings)?);
                store.query_build_phase(&result.project_id, &result.id, "rating_basis")?;
                for rating in ratings {
                    self.rating_cache.ensure(
                        source,
                        &rating,
                        budget.memory_bytes,
                        &self.query_directory,
                        cancelled.clone(),
                    )?;
                }
                store.query_build_phase(&result.project_id, &result.id, mode)?;
            }
            let previous = basis
                .as_ref()
                .and_then(|b| b.source_versions.iter().find(|v| v.source_id == source.id));
            if previous == Some(expected) && !post_refresh {
                continue;
            }
            let index = if !fixed && source.kind == "danbooru" {
                self.browse_index.reader(source).ok()
            } else {
                None
            };
            if source.kind == "danbooru" && index.is_none() {
                stage.borrow_mut().post_ready = false;
            }
            let native = studio_storage::native_spec(&result.spec);
            let ranked_candidates = ranking.active() && native.conditions.is_empty();
            let mut sink = |keys: &[AssetKey], processed| {
                let scoped = store.filter_query_input(&result.project_id, &result.spec, keys)?;
                let ranked = ranking.filter(&scoped, ranked_candidates)?;
                let filtered = store.filter_derived(&result.project_id, &result.spec, &ranked)?;
                let posts = if let Some(index) = &index {
                    index.post_ids(&filtered)?
                } else {
                    vec![None; filtered.len()]
                };
                let mut staging = stage.borrow_mut();
                staging.append(&filtered, &posts, processed)?;
                if last_progress.get().elapsed() >= Duration::from_millis(600) {
                    store.query_build_progress(
                        &result.project_id,
                        &result.id,
                        staging.processed,
                        staging.evaluated,
                    )?;
                    last_progress.set(Instant::now());
                }
                Ok(())
            };
            if ranked_candidates {
                stage.borrow_mut().full_source(&source.id);
                let evaluated = ranking.stream(&source.id, &cancelled, &mut sink)?;
                stage.borrow_mut().evaluated += evaluated;
            } else if fixed || input_count.is_some_and(|count| count <= 4096) {
                stage.borrow_mut().full_source(&source.id);
                // Small project scopes use indexed identity predicates, not a lake scan.
                let mut after = None;
                loop {
                    let keys = store.query_input_keys(
                        &result.project_id,
                        &result.spec,
                        &source.id,
                        after.as_deref(),
                    )?;
                    if keys.is_empty() {
                        break;
                    }
                    stage.borrow_mut().evaluated += keys.len() as u64;
                    if fixed {
                        sink(&keys, keys.len() as u64)?;
                    } else {
                        reader.execute_query_keys(
                            source,
                            &native,
                            expected,
                            cancelled.clone(),
                            &keys,
                            &mut sink,
                        )?;
                    }
                    after = keys.last().map(|k| k.asset_id.clone());
                }
            } else {
                let old_sequence = previous
                    .and_then(|v| v.catalog_revision.rsplit(':').next())
                    .and_then(|s| s.parse::<u64>().ok());
                let anchor = old_sequence
                    .map(|seq| self.browse_index.anchor(&source.id, seq))
                    .transpose()?
                    .flatten()
                    .filter(|anchor| {
                        previous.is_some_and(|v| {
                            v.catalog_revision
                                == format!("catalog-v1:{}:{}", anchor.generation, anchor.sequence)
                        })
                    });
                let mut affected = |keys: &[AssetKey]| stage.borrow_mut().affected(keys);
                let updated = if !post_refresh && let Some(anchor) = anchor {
                    reader.execute_delta(
                        source,
                        &native,
                        expected,
                        &anchor,
                        cancelled.clone(),
                        &mut affected,
                        &mut sink,
                    )?
                } else {
                    false
                };
                if !updated {
                    if basis.is_some() {
                        mode = "rebuilt";
                    }
                    stage.borrow_mut().full_source(&source.id);
                    store.query_build_phase(&result.project_id, &result.id, mode)?;
                    reader.execute_query(
                        source,
                        &native,
                        expected,
                        cancelled.clone(),
                        &mut sink,
                    )?;
                }
            }
        }
        if cancelled.load(Ordering::Acquire) {
            return Err(Error::new("CANCELLED", "构建已取消"));
        }
        // Multi-source builds have per-source transactions; check every source again
        // before publication. This fence is explicitly not a historical snapshot.
        if !fixed
            && self.versions(store, &result.project_id, &result.spec)? != result.source_versions
        {
            return Err(Error::new(
                "SOURCE_CHANGED",
                "构建期间来源已更新，请重新计算",
            ));
        }
        let stage = stage.into_inner();
        stage.seal()?;
        let (ratings, candidates) = reader.rating_usage()?;
        store.query_basis_usage(&result.project_id, &result.id, &ratings, candidates)?;
        store.query_build_phase(&result.project_id, &result.id, "publishing")?;
        store.publish_stage_with_budget(
            &result.project_id,
            &result.id,
            &stage,
            mode,
            &cancelled,
            (budget.memory_bytes / 2).min(4 << 30),
        )?;
        if !fixed
            && self.versions(store, &result.project_id, &result.spec)? != result.source_versions
        {
            return Err(Error::new(
                "SOURCE_CHANGED",
                "发布查询期间来源已更新，请刷新后重试",
            ));
        }
        Ok(())
    }
}
pub async fn scheduler(store: Arc<SqliteStore>, runner: Arc<QueryRunner>) {
    while !runner.stopping.load(Ordering::Acquire) {
        let s = store.clone();
        let next = tokio::task::spawn_blocking(move || -> Result<Option<QueryResult>> {
            s.reap_closed()?;
            for id in s.owned_projects()? {
                match s.next_result(&id) {
                    Ok(Some(result)) => return Ok(Some(result)),
                    Err(error) => tracing::warn!(project_id=%id,%error,"query scheduling failed"),
                    _ => {}
                }
            }
            Ok(None)
        })
        .await;
        match next {
            Ok(Ok(Some(result))) => {
                let cancelled = Arc::new(AtomicBool::new(false));
                if let Ok(mut running) = runner.running.lock() {
                    running.insert(result.id.clone(), cancelled.clone());
                }
                let s = store.clone();
                let r = runner.clone();
                let query = result.clone();
                let completed=tokio::task::spawn_blocking(move||->Result<()> {
                    // Pin through status publication, including user cancellation.
                    let _lease=s.operation_lease(&query.project_id)?;
                    if !s.start_result(&query.project_id,&query.id)? { return Ok(()); }
                    let started=std::time::Instant::now();
                    let built=r.build(&s,&query,cancelled);
                    let error=if r.stopping.load(Ordering::Acquire) { Some(Error::new("INTERRUPTED","引擎已停止，结果需要重新计算")) } else { built.err() };
                    if let Some(e)=&error { tracing::warn!(result_id=%query.id,code=e.code,message=%e.message,"query build stopped"); }
                    let _cache_gate=r.cache.lock()?;
                    let published=s.finish_result(&query.project_id,&query.id,error.as_ref())?;
                    if published.state==ResultState::Ready {r.cache.recent(&query.project_id,&query.id);}
                    r.cache.track_committed(&s,&query.project_id);
                    r.cache.requested.store(true,Ordering::Release);
                    tracing::info!(result_id=%query.id,state=?published.state,count=?published.count,elapsed_ms=started.elapsed().as_millis(),"query build finished");
                    Ok(())
                }).await;
                if let Ok(mut running) = runner.running.lock() {
                    running.remove(&result.id);
                }
                if let Err(error) = completed.map_err(Error::io).and_then(|r| r) {
                    let _ = store.finish_result(&result.project_id, &result.id, Some(&error));
                    tracing::warn!(%error,"query worker failed");
                }
            }
            Ok(Err(error)) => tracing::warn!(%error,"query scheduler unavailable"),
            Err(error) => tracing::error!(%error,"query scheduler panic"),
            _ => {}
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}

fn sweep_cache(
    store: &SqliteStore,
    runner: &Arc<QueryRunner>,
    preview: &studio_resources::PreviewCache,
    force: bool,
) -> Result<u64> {
    // Page accounting must finish before taking the lease/eviction gate.
    let owned = store.owned_projects()?;
    for pid in &owned {
        runner.cache.phase("accounting", Some(pid));
        store.refresh_query_cache_sizes(pid)?;
        runner.cache.track(store, pid)?;
    }
    let config = runner.cache.config()?;
    runner.cache.phase("planning", None);
    let policy = config.policy();
    let mut projects = runner.cache.projects()?;
    projects.sort_by_key(|p| (p.temporary_families == 0, p.touched));
    let source_bytes = runner.browse_index.storage()?.0;
    let basis_bytes = runner.rating_cache.storage_bytes()?;
    let rank_budget = crate::ranked_indexes::available_budget(runner, preview)?;
    let rank_sessions = if config.temporary_session_only {
        Some(crate::ranked_indexes::live_session_projects(runner)?)
    } else {
        None
    };
    runner.cache.phase("indexes", None);
    runner.ranked_indexes.prune_released(store)?;
    runner.ranked_indexes.prune(
        rank_budget,
        force
            && runner
                .cache
                .clear_target()?
                .is_none_or(|tier| tier == QueryCacheTier::Temporary),
        u64::from(config.temporary_idle_hours) * 3600,
        rank_sessions.as_ref(),
    )?;
    let ranked_bytes = runner.ranked_indexes.metrics()?.bytes;
    let query_quota = (u64::from(config.total_mib) << 20)
        .saturating_sub(source_bytes + basis_bytes + preview.metrics().bytes + ranked_bytes)
        .min(policy.quota_bytes);
    let long_quota = policy
        .long_term_quota_bytes
        .saturating_sub(source_bytes + basis_bytes);
    let total = projects.iter().map(|p| p.bytes).sum::<u64>();
    let long_bytes = projects.iter().map(|p| p.long_term_bytes).sum::<u64>();
    let temporary_bytes = projects.iter().map(|p| p.temporary_bytes).sum::<u64>();
    let now = studio_storage::now().parse::<u64>().unwrap_or(0);
    let needs_closed = |p: &crate::query_cache::CachedProject| {
        force
            || p.unreferenced_members.is_none_or(|n| n > 0)
            || p.cleanup_pending
            || (p.retained > 0 && p.long_term_families + p.temporary_families == 0)
            || p.session_families > 0
            || (policy.session_only && p.temporary_families > 0)
            || (p.oldest_temporary_millis > 0
                && p.temporary_families > 0
                && p.oldest_temporary_millis <= now.saturating_sub(policy.max_age_seconds * 1000))
            || (p.oldest_long_term_millis > 0
                && p.long_term_families > 0
                && policy
                    .long_term_max_age_seconds
                    .is_some_and(|age| p.oldest_long_term_millis <= now.saturating_sub(age * 1000)))
            || p.free_bytes >= 32 << 20
            || total > query_quota
            || long_bytes > long_quota
            || temporary_bytes + ranked_bytes > policy.temporary_quota_bytes
    };
    let mut closed = HashMap::new();
    for p in projects
        .iter()
        .filter(|p| !owned.contains(&p.id) && needs_closed(p))
        .take(2)
    {
        if runner.stopping.load(Ordering::Acquire) {
            return Ok(0);
        }
        match SqliteStore::inspect_query_cache(&p.directory) {
            Ok(snapshot) => {
                runner
                    .cache
                    .record(&p.id, p.directory.clone(), snapshot.stats.clone())?;
                closed.insert(p.id.clone(), snapshot);
            }
            Err(error) => {
                tracing::warn!(project_id=%p.id,%error,"closed query cache inspection deferred")
            }
        }
    }
    let mut changed_projects = Vec::new();
    let mut reclaimed = 0;
    {
        let _gate = runner.cache.lock()?;
        // Configuration changes during a slow inspection require a fresh plan.
        if serde_json::to_vec(&runner.cache.config()?).map_err(Error::io)?
            != serde_json::to_vec(&config).map_err(Error::io)?
        {
            runner.cache.requested.store(true, Ordering::Release);
            return Ok(0);
        }
        let clear_tier = runner.cache.clear_target()?;
        for p in &projects {
            if !owned.contains(&p.id) && !needs_closed(p) {
                continue;
            }
            let local = studio_storage::QueryCachePolicy {
                quota_bytes: query_quota.saturating_sub(total.saturating_sub(p.bytes)),
                long_term_quota_bytes: long_quota
                    .saturating_sub(long_bytes.saturating_sub(p.long_term_bytes)),
                temporary_quota_bytes: policy
                    .temporary_quota_bytes
                    .saturating_sub(ranked_bytes)
                    .saturating_sub(temporary_bytes.saturating_sub(p.temporary_bytes)),
                live_sessions: runner.cache.live_sessions(&p.id)?,
                clear_tier,
                ..policy.clone()
            };
            runner.cache.phase(
                if p.free_bytes >= 32 << 20 {
                    "compacting"
                } else {
                    "deleting"
                },
                Some(&p.id),
            );
            let outcome = if owned.contains(&p.id) {
                store
                    .maintain_query_cache_step(&p.id, &local, &runner.cache.live(&p.id), force)
                    .map(Some)
            } else if let Some(before) = closed.get(&p.id) {
                SqliteStore::maintain_closed_cache_step(
                    store.root(),
                    &p.id,
                    &p.directory,
                    &local,
                    force,
                    before,
                )
            } else {
                continue;
            };
            match outcome {
                Ok(Some((count, changed))) => {
                    reclaimed += count;
                    changed_projects.push(p);
                    if changed {
                        runner.cache.requested.store(true, Ordering::Release);
                        break;
                    }
                }
                Ok(None) => {}
                Err(error) if error.code == "CACHE_BUSY" => {
                    runner.cache.phase("waiting", Some(&p.id));
                    runner.cache.requested.store(true, Ordering::Release);
                }
                Err(error) => {
                    tracing::warn!(project_id=%p.id,%error,"query cache maintenance deferred")
                }
            }
        }
        let mut bases = runner.rating_cache.entries()?;
        bases.sort_by_key(|b| b.last_used_millis);
        let mut retained_basis = basis_bytes;
        for basis in bases {
            if reclaimed >= 2 {
                break;
            }
            let expired = policy
                .long_term_max_age_seconds
                .is_some_and(|age| basis.last_used_millis <= now.saturating_sub(age * 1000));
            let clear = force && clear_tier.is_none_or(|v| v == QueryCacheTier::LongTerm);
            let excess = long_bytes + source_bytes + retained_basis > policy.long_term_quota_bytes;
            if !basis.fixed
                && !basis.active
                && (expired || clear || excess)
                && runner
                    .rating_cache
                    .remove(&basis.source_id, &basis.rating, false)?
            {
                retained_basis = retained_basis.saturating_sub(basis.bytes);
                reclaimed += 1;
            }
        }
    }
    // Reinspect after the mutation, outside both locks. A stale snapshot never
    // drives another eviction; the next bounded pass plans from current sizes.
    for p in changed_projects {
        runner.cache.phase("accounting", Some(&p.id));
        let stats = if owned.contains(&p.id) {
            store.query_cache_stats(&p.id)?
        } else {
            let snapshot = SqliteStore::inspect_query_cache_after(&p.directory, closed.get(&p.id))?;
            SqliteStore::settle_closed_cache(store.root(), &p.id, &p.directory, &snapshot)?
                .unwrap_or(snapshot.stats)
        };
        runner.cache.record(&p.id, p.directory.clone(), stats)?;
    }
    Ok(reclaimed)
}

pub async fn cache_maintenance(
    store: Arc<SqliteStore>,
    runner: Arc<QueryRunner>,
    previews: studio_resources::PreviewCache,
) {
    let mut last = Instant::now();
    while !runner.stopping.load(Ordering::Acquire) {
        runner.ranked_indexes.tick();
        if runner.cache.requested.swap(false, Ordering::AcqRel)
            || last.elapsed() >= Duration::from_secs(30)
        {
            last = Instant::now();
            let s = store.clone();
            let r = runner.clone();
            let preview = previews.clone();
            let force = runner.cache.force.swap(false, Ordering::AcqRel);
            runner.cache.busy.store(true, Ordering::Release);
            let result =
                tokio::task::spawn_blocking(move || sweep_cache(&s, &r, &preview, force)).await;
            match result {
                Ok(Ok(n)) => {
                    runner.cache.reclaimed.fetch_add(n, Ordering::Relaxed);
                    if n >= 1 {
                        runner.cache.requested.store(true, Ordering::Release);
                    }
                }
                Ok(Err(e)) => {
                    runner.cache.requested.store(true, Ordering::Release);
                    tracing::warn!(%e,"query cache maintenance deferred");
                    runner.cache.failed(&e.to_string());
                }
                Err(e) => {
                    runner.cache.requested.store(true, Ordering::Release);
                    tracing::warn!(%e,"query cache maintenance failed");
                    runner.cache.failed(&e.to_string());
                }
            }
            if force && runner.cache.requested.load(Ordering::Acquire) {
                runner.cache.force.store(true, Ordering::Release);
            }
            if !runner.cache.requested.load(Ordering::Acquire) {
                runner.cache.phase("completed", None);
            }
            runner.cache.busy.store(false, Ordering::Release);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
