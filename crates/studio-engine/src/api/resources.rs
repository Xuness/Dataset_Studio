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
    Ok(Json(ReadServiceStatus {
        protocol_version: 1,
        resources: s.resources.metrics().into_iter().map(Into::into).collect(),
        cache: cache_status(s.previews.cache.metrics()),
        process_memory: process_memory(),
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
        blocking(move || s.previews.cache.set_quota(u64::from(body.quota_mib) << 20)).await?,
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
