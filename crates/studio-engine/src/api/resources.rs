use super::*;
fn cache_status(m: studio_resources::CacheMetrics) -> PreviewCacheStatus {
    PreviewCacheStatus {
        directory: m.directory,
        quota_bytes: m.quota_bytes.to_string(),
        bytes: m.bytes.to_string(),
        entries: m.entries,
        pinned: m.pinned,
        hits: m.hits,
        misses: m.misses,
        corrupt: m.corrupt,
        evicted: m.evicted,
        writes: m.writes,
        read_bytes: m.read_bytes.to_string(),
        maintenance_removed: m.maintenance_removed,
        maintenance_pending: m.maintenance_pending,
        index_rebuilt: m.index_rebuilt,
        clear_pending: m.clear_pending,
    }
}
#[utoipa::path(get,path="/v1/resources",responses((status=200,body=ReadServiceStatus)))]
pub(super) async fn status(State(s): State<AppState>) -> ApiResult<ReadServiceStatus> {
    let m = s.previews.metrics();
    let state = s.clone();
    let query_cache = blocking(move || query_cache_status(&state)).await?;
    let [
        busy_errors,
        protocol_errors,
        lease_retries,
        max_lease_write_ms,
    ] = studio_sources::online::contention_metrics();
    Ok(Json(ReadServiceStatus {
        protocol_version: 1,
        resources: s.resources.metrics().into_iter().map(Into::into).collect(),
        cache: cache_status(s.previews.cache.metrics()),
        process_memory: process_memory(),
        online_sqlite: OnlineSqliteStatus {
            busy_errors,
            protocol_errors,
            lease_retries,
            max_lease_write_ms,
        },
        query_limits: query_limits(&s)?,
        query_cache,
        aesthetic: aesthetic_status(&s),
        previews: PreviewActivity {
            shared: m.shared,
            queued: m.queued,
            active_subscriptions: m.active_subscriptions,
            generated: m.generated,
            cancelled_last: m.cancelled_last,
            cancelled_before_read: m.cancelled_before_read,
            source_bytes: m.source_bytes.to_string(),
            pack_opens: m.pack_opens,
            seeks: m.seeks,
            decode_ms: m.decode_ms,
            read_ms: m.read_ms,
            batches: m.batches,
            max_batch: m.max_batch,
            queue_wait_ms: m.queue_wait_ms,
            max_queue_wait_ms: m.max_queue_wait_ms,
            cancelled_finished: m.cancelled_finished,
            max_cancel_latency_ms: m.max_cancel_latency_ms,
        },
    }))
}
pub(super) fn query_cache_status(s: &AppState) -> domain::Result<QueryCacheStatus> {
    use std::sync::atomic::Ordering;
    let config = s.queries.cache.config()?;
    for pid in s.store.owned_projects()? {
        s.queries.cache.track(&s.store, &pid)?;
    }
    let records = s.queries.cache.projects()?;
    let (index_bytes, index_count) = s.queries.source_indexes.browse_index.storage()?;
    let ranked = s.queries.ranked_indexes.metrics()?;
    Ok(QueryCacheStatus {
        quota_bytes: (u64::from(config.query_mib()) << 20).to_string(),
        max_age_days: config.temporary_idle_hours.div_ceil(24),
        retained_queries: records.iter().map(|p| p.retained).sum(),
        member_versions: records.iter().map(|p| p.members).sum(),
        result_storage_bytes: (records.iter().map(|p| p.bytes).sum::<u64>() + ranked.bytes)
            .to_string(),
        database_free_bytes: records
            .iter()
            .map(|p| p.free_bytes)
            .sum::<u64>()
            .to_string(),
        protected_results: records.iter().map(|p| p.protected).sum(),
        active_views: records
            .iter()
            .map(|p| s.queries.cache.live(&p.id).len() as u64)
            .sum(),
        reused_results: records.iter().map(|p| p.reused).sum(),
        incremental_results: records.iter().map(|p| p.incremental).sum(),
        source_index_bytes: index_bytes.to_string(),
        source_indexes: index_count,
        ranked_index_bytes: ranked.bytes.to_string(),
        ranked_indexes: ranked.entries,
        ranked_index_builds: ranked.builds,
        ranked_index_reuses: ranked.reuses,
        cleanup_pending: s.queries.cache.busy.load(Ordering::Acquire)
            || s.queries.cache.requested.load(Ordering::Acquire)
            || records.iter().any(|p| p.cleanup_pending)
            || s.queries.ranked_indexes.busy(),
        reclaimed_queries: s.queries.cache.reclaimed.load(Ordering::Relaxed),
    })
}
#[utoipa::path(put,path="/v1/resources/query-cache",request_body=SetQueryCache,responses((status=200,body=QueryCacheStatus)))]
pub(super) async fn configure_query_cache(
    State(s): State<AppState>,
    Body(body): Body<SetQueryCache>,
) -> ApiResult<QueryCacheStatus> {
    Ok(Json(
        blocking(move || {
            s.queries
                .cache
                .configure(crate::query_cache::CacheConfig::legacy(
                    body.quota_mib,
                    body.max_age_days,
                    s.queries.cache.config()?.preview_mib,
                )?)?;
            query_cache_status(&s)
        })
        .await?,
    ))
}
#[utoipa::path(post,path="/v1/resources/query-cache/clear",responses((status=200,body=QueryCacheStatus)))]
pub(super) async fn clear_query_cache(State(s): State<AppState>) -> ApiResult<QueryCacheStatus> {
    s.queries.cache.clear();
    Ok(Json(blocking(move || query_cache_status(&s)).await?))
}
pub(super) fn query_limits(s: &AppState) -> domain::Result<QueryResourceLimits> {
    let (configured, active) = s.queries.budget.status()?;
    Ok(QueryResourceLimits {
        metadata_memory_bytes: domain::METADATA_MEMORY_BYTES.to_string(),
        query_memory_bytes: configured.to_string(),
        native_query_memory_bytes: (configured
            - crate::query_budget::result_work_memory(configured))
        .to_string(),
        result_work_memory_bytes: crate::query_budget::result_work_memory(configured).to_string(),
        active_query_memory_bytes: active.map(|n| n.to_string()),
        temporary_disk_bytes: domain::QUERY_TEMP_BYTES.to_string(),
        result_staging_disk_bytes: domain::QUERY_STAGE_BYTES.to_string(),
    })
}
fn aesthetic_status(s: &AppState) -> AestheticEngineStatus {
    let (running_stages, max_running_stages) = s.aesthetic.stage_limits();
    AestheticEngineStatus {
        running_stages,
        max_running_stages,
        max_running_stages_limit: domain::aesthetic::AESTHETIC_MAX_RUNNING_STAGES,
    }
}
#[utoipa::path(put,path="/v1/resources/aesthetic",request_body=SetAestheticEngine,responses((status=200,body=AestheticEngineStatus)))]
pub(super) async fn configure_aesthetic(
    State(s): State<AppState>,
    Body(body): Body<SetAestheticEngine>,
) -> ApiResult<AestheticEngineStatus> {
    Ok(Json(
        blocking(move || {
            s.aesthetic.configure(body.max_running_stages)?;
            Ok(aesthetic_status(&s))
        })
        .await?,
    ))
}
#[utoipa::path(put,path="/v1/resources/query",request_body=SetQueryMemory,responses((status=200,body=QueryResourceLimits)))]
pub(super) async fn configure_query(
    State(s): State<AppState>,
    Body(body): Body<SetQueryMemory>,
) -> ApiResult<QueryResourceLimits> {
    Ok(Json(
        blocking(move || {
            s.queries.budget.configure(body.memory_gib)?;
            query_limits(&s)
        })
        .await?,
    ))
}
#[cfg(windows)]
fn process_memory() -> Option<ReadProcessMemory> {
    use windows_sys::Win32::System::{
        ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX},
        Threading::GetCurrentProcess,
    };
    let mut counters: PROCESS_MEMORY_COUNTERS_EX = unsafe { std::mem::zeroed() };
    counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    // The pseudo handle is borrowed; the exact-size output struct remains live.
    if unsafe {
        K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
            counters.cb,
        )
    } == 0
    {
        return None;
    }
    Some(ReadProcessMemory {
        resident_bytes: counters.WorkingSetSize.to_string(),
        peak_resident_bytes: counters.PeakWorkingSetSize.to_string(),
        private_bytes: counters.PrivateUsage.to_string(),
    })
}
#[cfg(not(windows))]
fn process_memory() -> Option<ReadProcessMemory> {
    None
}
#[utoipa::path(put,path="/v1/resources/cache",request_body=SetCacheQuota,responses((status=200,body=PreviewCacheStatus)))]
pub(super) async fn configure(
    State(s): State<AppState>,
    Body(body): Body<SetCacheQuota>,
) -> ApiResult<PreviewCacheStatus> {
    Ok(Json(cache_status(
        blocking(move || {
            let _gate = s.queries.cache.lock()?;
            let previous = s.queries.cache.config()?;
            let mut next = previous.clone();
            next.preview_mib = body.quota_mib;
            next.total_mib = next
                .total_mib
                .max(next.query_mib().saturating_add(next.preview_mib));
            s.queries.cache.configure(next)?;
            match s.previews.cache.set_quota(u64::from(body.quota_mib) << 20) {
                Ok(metrics) => Ok(metrics),
                Err(error) => {
                    let _ = s.queries.cache.configure(previous);
                    Err(error)
                }
            }
        })
        .await?,
    )))
}
#[utoipa::path(post,path="/v1/resources/cache/clear",responses((status=200,body=PreviewCacheStatus)))]
pub(super) async fn clear(State(s): State<AppState>) -> ApiResult<PreviewCacheStatus> {
    Ok(Json(cache_status(
        blocking(move || s.previews.cache.clear()).await?,
    )))
}
#[utoipa::path(post,path="/v1/projects/{project_id}/read-requests/{request_id}/cancel",operation_id="cancel_read_subscription",params(("project_id"=String,Path),("request_id"=String,Path)),responses((status=200,body=OkResponse)))]
pub(super) async fn cancel(
    State(s): State<AppState>,
    Path((pid, id)): Path<(String, String)>,
) -> ApiResult<OkResponse> {
    s.previews.cancel(&pid, &id)?;
    Ok(Json(OkResponse { ok: true }))
}
