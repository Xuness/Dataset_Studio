use super::*;
use crate::cache_config::CacheConfig;
use std::path::Path as FilePath;

fn cache_settings(config: CacheConfig) -> CacheSettings {
    CacheSettings {
        total_mib: config.total_mib,
        long_term_mib: config.long_term_mib,
        temporary_mib: config.temporary_mib,
        preview_mib: config.preview_mib,
        long_term_idle_days: config.long_term_idle_days,
        temporary_idle_hours: config.temporary_idle_hours,
        temporary_session_only: config.temporary_session_only,
    }
}
fn cache_config(value: CacheSettings) -> CacheConfig {
    CacheConfig {
        schema_version: 2,
        total_mib: value.total_mib,
        long_term_mib: value.long_term_mib,
        temporary_mib: value.temporary_mib,
        preview_mib: value.preview_mib,
        long_term_idle_days: value.long_term_idle_days,
        temporary_idle_hours: value.temporary_idle_hours,
        temporary_session_only: value.temporary_session_only,
    }
}
fn temporary_bytes(root: &FilePath, depth: usize) -> domain::Result<u64> {
    if depth == 0 || !root.exists() {
        return Ok(0);
    }
    let mut bytes = 0;
    let files = match std::fs::read_dir(root) {
        Ok(files) => files,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(domain::Error::io(e)),
    };
    for entry in files {
        let entry = entry.map_err(domain::Error::io)?;
        let kind = match entry.file_type() {
            Ok(kind) => kind,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(domain::Error::io(e)),
        };
        if kind.is_symlink() {
            continue;
        }
        if kind.is_file() {
            match entry.metadata() {
                Ok(m) => bytes += m.len(),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(domain::Error::io(e)),
            };
        } else if kind.is_dir() {
            bytes += temporary_bytes(&entry.path(), depth - 1)?;
        }
    }
    Ok(bytes)
}
fn settings_status(s: &AppState) -> domain::Result<SettingsStatus> {
    use std::sync::atomic::Ordering;
    let config = s.queries.cache.config()?;
    for pid in s.store.owned_projects()? {
        s.queries.cache.track(&s.store, &pid)?;
    }
    let projects = s.queries.cache.projects()?;
    let indexes = s.queries.browse_index.storage()?.0;
    let rating_bytes = s.queries.rating_cache.storage_bytes()?;
    let preview = s.previews.cache.metrics();
    let mut preview_bytes = preview.bytes;
    for name in [
        "cache.sqlite",
        "cache.sqlite-wal",
        "cache.sqlite-shm",
        "cache-settings.json",
    ] {
        if let Ok(metadata) = std::fs::metadata(FilePath::new(&preview.directory).join(name)) {
            preview_bytes += metadata.len();
        }
    }
    let member_bytes = projects.iter().map(|p| p.bytes).sum::<u64>();
    let ranked = s.queries.ranked_indexes.metrics()?;
    let fixed_bases = s
        .queries
        .rating_cache
        .entries()?
        .iter()
        .filter(|b| b.fixed)
        .map(|b| b.bytes)
        .sum::<u64>();
    Ok(SettingsStatus {
        maintenance: s
            .queries
            .cache
            .maintenance
            .lock()
            .map_err(|_| domain::Error::new("INTERNAL_ERROR", "清理状态不可用"))?
            .clone(),
        cache: cache_settings(config.clone()),
        query_limits: resources::query_limits(s)?,
        storage: CacheStorageOverview {
            total_bytes: (member_bytes + indexes + rating_bytes + preview_bytes + ranked.bytes)
                .to_string(),
            total_quota_bytes: (u64::from(config.total_mib) << 20).to_string(),
            long_term_bytes: (projects.iter().map(|p| p.long_term_bytes).sum::<u64>()
                + indexes
                + rating_bytes)
                .to_string(),
            temporary_bytes: (projects.iter().map(|p| p.temporary_bytes).sum::<u64>()
                + ranked.bytes)
                .to_string(),
            preview_bytes: preview_bytes.to_string(),
            source_index_bytes: indexes.to_string(),
            rating_basis_bytes: rating_bytes.to_string(),
            project_member_bytes: (member_bytes + ranked.bytes).to_string(),
            ranked_index_bytes: ranked.bytes.to_string(),
            reusable_bytes: projects
                .iter()
                .map(|p| p.free_bytes)
                .sum::<u64>()
                .to_string(),
            fixed_member_bytes: (projects.iter().map(|p| p.fixed_bytes).sum::<u64>()
                + fixed_bases
                + indexes)
                .to_string(),
            working_temporary_bytes: (temporary_bytes(&s.store.root().join("query-temp"), 3)?
                + s.queries.rating_cache.working_bytes()?
                + ranked.working_bytes)
                .to_string(),
            protected_results: projects.iter().map(|p| p.protected).sum(),
            active_views: projects
                .iter()
                .map(|p| s.queries.cache.live(&p.id).len() as u64)
                .sum(),
            long_term_results: projects.iter().map(|p| p.long_term_families).sum(),
            temporary_results: projects.iter().map(|p| p.temporary_families).sum(),
            cleanup_pending: s.queries.cache.busy.load(Ordering::Acquire)
                || s.queries.cache.requested.load(Ordering::Acquire)
                || projects.iter().any(|p| p.cleanup_pending),
        },
    })
}
#[utoipa::path(get,path="/v1/settings",operation_id="read_settings",responses((status=200,body=SettingsStatus)))]
pub(super) async fn read(State(s): State<AppState>) -> ApiResult<SettingsStatus> {
    Ok(Json(blocking(move || settings_status(&s)).await?))
}
#[utoipa::path(put,path="/v1/settings/cache",operation_id="configure_cache_settings",request_body=CacheSettings,responses((status=200,body=SettingsStatus)))]
pub(super) async fn configure(
    State(s): State<AppState>,
    Body(body): Body<CacheSettings>,
) -> ApiResult<SettingsStatus> {
    Ok(Json(
        blocking(move || {
            let _gate = s.queries.cache.lock()?;
            let config = cache_config(body);
            config.validate()?;
            let previous = s.queries.cache.config()?;
            s.queries.cache.configure(config.clone())?;
            if let Err(error) = s
                .previews
                .cache
                .set_quota(u64::from(config.preview_mib) << 20)
            {
                let _ = s.queries.cache.configure(previous);
                return Err(error);
            }
            settings_status(&s)
        })
        .await?,
    ))
}
fn tier(value: &str) -> domain::Result<domain::QueryCacheTier> {
    match value {
        "long_term" => Ok(domain::QueryCacheTier::LongTerm),
        "temporary" => Ok(domain::QueryCacheTier::Temporary),
        _ => Err(domain::Error::invalid("缓存类别必须是长期或临时")),
    }
}
#[utoipa::path(post,path="/v1/settings/cache/clear",operation_id="clear_cache_tier",request_body=ClearCacheTier,responses((status=200,body=SettingsStatus)))]
pub(super) async fn clear(
    State(s): State<AppState>,
    Body(body): Body<ClearCacheTier>,
) -> ApiResult<SettingsStatus> {
    let target = body.tier.as_deref().map(tier).transpose()?;
    s.queries.cache.request_clear(target)?;
    Ok(Json(blocking(move || settings_status(&s)).await?))
}
#[utoipa::path(get,path="/v1/projects/{project_id}/cache-entries",operation_id="list_cache_entries",params(("project_id"=String,Path),("cursor"=Option<String>,Query),("limit"=Option<usize>,Query)),responses((status=200,body=CacheEntries)))]
pub(super) async fn entries(
    State(s): State<AppState>,
    Path(pid): Path<String>,
    Query(q): Query<QueryListParams>,
) -> ApiResult<CacheEntries> {
    Ok(Json(
        blocking(move || {
            let limit = q.limit.unwrap_or(32).clamp(1, 127);
            let mut rows = s
                .store
                .query_cache_entries(&pid, q.cursor.as_deref(), limit + 1)?;
            let more = rows.len() > limit;
            rows.truncate(limit);
            let cursor = if more {
                rows.last().map(|r| r.family_id.clone())
            } else {
                None
            };
            let active = s
                .store
                .active_query_families(&pid, &s.queries.cache.live(&pid))?;
            let items = rows
                .into_iter()
                .map(|r| CacheEntry {
                    in_use: active.contains(&r.family_id),
                    project_id: r.project_id,
                    family_id: r.family_id,
                    result_id: r.result_id,
                    spec: r.spec.into(),
                    tier: r.tier.as_str().into(),
                    fixed: r.fixed,
                    session_only: r.session_only,
                    last_used_millis: r.last_used_millis.to_string(),
                    members: r.members,
                    estimated_bytes: r.estimated_bytes.map(|n| n.to_string()),
                    protected_results: r.protected_results,
                })
                .collect();
            Ok(CacheEntries {
                items,
                next_cursor: cursor,
                cleanups: s
                    .store
                    .query_cleanup_status(&pid)?
                    .into_iter()
                    .map(|t| CacheCleanup {
                        family_id: t.family_id,
                        result_id: t.result_id,
                        spec: t.spec.map(Into::into),
                        state: t.state,
                        total: t.total,
                        processed: t.processed,
                        removed: t.removed,
                        started_millis: t.started_millis,
                        updated_millis: t.updated_millis,
                        error: t.error,
                    })
                    .collect(),
            })
        })
        .await?,
    ))
}
#[utoipa::path(put,path="/v1/projects/{project_id}/query-results/{result_id}/retention",operation_id="set_result_retention",params(("project_id"=String,Path),("result_id"=String,Path)),request_body=SetResultRetention,responses((status=200,body=QueryResult)))]
pub(super) async fn retention(
    State(s): State<AppState>,
    Extension(session): Extension<ClientSession>,
    Path((pid, rid)): Path<(String, String)>,
    Body(body): Body<SetResultRetention>,
) -> ApiResult<QueryResult> {
    Ok(Json(
        blocking(move || {
            let _gate = s.queries.cache.lock()?;
            let selected = tier(&body.tier)?;
            let config = s.queries.cache.config()?;
            let session_id = s.queries.cache.session(&pid, session.0.as_deref())?;
            let result = s.store.set_cache_retention(
                &pid,
                &rid,
                selected,
                body.fixed,
                config.temporary_session_only,
            )?;
            s.store
                .bind_cache_session(&pid, &rid, &session_id, config.temporary_session_only)?;
            s.queries.cache.track_committed(&s.store, &pid);
            Ok(result.into())
        })
        .await?,
    ))
}
#[utoipa::path(post,path="/v1/projects/{project_id}/cache-session",operation_id="keep_cache_session",params(("project_id"=String,Path)),responses((status=200,body=OkResponse)))]
pub(super) async fn heartbeat(
    State(s): State<AppState>,
    Extension(session): Extension<ClientSession>,
    Path(pid): Path<String>,
) -> ApiResult<OkResponse> {
    s.queries.cache.session(&pid, session.0.as_deref())?;
    Ok(Json(OkResponse { ok: true }))
}
fn build_status(value: crate::query_jobs::RatingBuildStatus) -> RatingBuild {
    RatingBuild {
        source_id: value.source_id,
        state: value.state,
        current_rating: value.current_rating,
        completed: value.completed,
        error: value.error,
    }
}
#[utoipa::path(post,path="/v1/projects/{project_id}/query-results/{result_id}/cache-release",operation_id="release_cache_entry",params(("project_id"=String,Path),("result_id"=String,Path)),responses((status=200,body=QueryResult)))]
pub(super) async fn release_entry(
    State(s): State<AppState>,
    Path((pid, rid)): Path<(String, String)>,
) -> ApiResult<QueryResult> {
    Ok(Json(
        blocking(move || {
            let _gate = s.queries.cache.lock()?;
            let result = s
                .store
                .release_cache_entry(&pid, &rid, &s.queries.cache.live(&pid))?;
            s.queries.cache.phase("queued", Some(&pid));
            s.queries
                .cache
                .requested
                .store(true, std::sync::atomic::Ordering::Release);
            s.queries.cache.track_committed(&s.store, &pid);
            Ok(result.into())
        })
        .await?,
    ))
}
#[utoipa::path(get,path="/v1/cache/rating-bases",operation_id="list_rating_bases",responses((status=200,body=RatingBases)))]
pub(super) async fn bases(State(s): State<AppState>) -> ApiResult<RatingBases> {
    Ok(Json(
        blocking(move || {
            let items = s
                .queries
                .rating_cache
                .entries()?
                .into_iter()
                .map(|r| RatingBasis {
                    source_id: r.source_id,
                    rating: r.rating,
                    generation: r.generation,
                    sequence: r.sequence,
                    records: r.records,
                    bytes: r.bytes.to_string(),
                    last_used_millis: r.last_used_millis.to_string(),
                    incremental: r.incremental,
                    fixed: r.fixed,
                    active: r.active,
                })
                .collect();
            Ok(RatingBases {
                items,
                builds: s
                    .queries
                    .rating_builds()?
                    .into_iter()
                    .map(build_status)
                    .collect(),
            })
        })
        .await?,
    ))
}
#[utoipa::path(post,path="/v1/projects/{project_id}/sources/{source_id}/rating-bases",operation_id="prebuild_rating_bases",params(("project_id"=String,Path),("source_id"=String,Path)),responses((status=200,body=RatingBuild)))]
pub(super) async fn prebuild(
    State(s): State<AppState>,
    Path((pid, sid)): Path<(String, String)>,
) -> ApiResult<RatingBuild> {
    Ok(Json(
        blocking(move || {
            let source = s.store.source(&pid, &sid)?;
            Ok(build_status(s.queries.start_rating_build(&source)?))
        })
        .await?,
    ))
}
#[utoipa::path(post,path="/v1/cache/rating-bases/{source_id}/cancel",operation_id="cancel_rating_basis_build",params(("source_id"=String,Path)),responses((status=200,body=OkResponse)))]
pub(super) async fn cancel_build(
    State(s): State<AppState>,
    Path(sid): Path<String>,
) -> ApiResult<OkResponse> {
    s.queries.cancel_rating_build(&sid)?;
    Ok(Json(OkResponse { ok: true }))
}
#[utoipa::path(put,path="/v1/cache/rating-bases/{source_id}/{rating}",operation_id="set_rating_basis_retention",params(("source_id"=String,Path),("rating"=String,Path)),request_body=SetRatingRetention,responses((status=200,body=OkResponse)))]
pub(super) async fn fix_basis(
    State(s): State<AppState>,
    Path((sid, rating)): Path<(String, String)>,
    Body(body): Body<SetRatingRetention>,
) -> ApiResult<OkResponse> {
    Ok(Json(
        blocking(move || {
            s.queries
                .rating_cache
                .set_fixed(&sid, &rating, body.fixed)?;
            Ok(OkResponse { ok: true })
        })
        .await?,
    ))
}
#[utoipa::path(post,path="/v1/cache/rating-bases/{source_id}/{rating}/release",operation_id="release_rating_basis",params(("source_id"=String,Path),("rating"=String,Path)),responses((status=200,body=OkResponse)))]
pub(super) async fn release_basis(
    State(s): State<AppState>,
    Path((sid, rating)): Path<(String, String)>,
) -> ApiResult<OkResponse> {
    Ok(Json(
        blocking(move || {
            if !s.queries.rating_cache.remove(&sid, &rating, false)? {
                return Err(domain::Error::new(
                    "CACHE_IN_USE",
                    "基础缓存正在使用或已固定，请稍后重试或取消固定",
                ));
            }
            Ok(OkResponse { ok: true })
        })
        .await?,
    ))
}
