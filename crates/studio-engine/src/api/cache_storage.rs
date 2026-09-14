use super::*;
use std::sync::atomic::Ordering;

#[utoipa::path(get,path="/v1/cache/projects",operation_id="cache_projects",responses((status=200,body=CacheProjects)))]
pub(super) async fn projects(State(s): State<AppState>) -> ApiResult<CacheProjects> {
    Ok(Json(
        blocking(move || {
            for pid in s.store.owned_projects()? {
                s.queries.cache.track(&s.store, &pid)?;
            }
            let catalog = s.queries.cache.projects()?;
            let ranked = s.queries.ranked_indexes.inventory()?;
            let projects = s.store.list()?;
            for project in &projects {
                if !catalog.iter().any(|c| c.id == project.id) && project.issue.is_none() {
                    s.queries
                        .cache
                        .request_audit(&project.id, project.directory.clone())?;
                }
            }
            let items = projects
                .into_iter()
                .map(|p| {
                    let cached = catalog.iter().find(|c| c.id == p.id);
                    let rank_bytes: u64 = ranked
                        .iter()
                        .filter(|r| r.meta.scope.project_id == p.id)
                        .map(|r| r.bytes)
                        .sum();
                    let bytes = cached.map_or(0, |c| c.bytes);
                    CacheProject {
                        project_id: p.id,
                        name: p.name,
                        state: p.state.into(),
                        directory: p.directory.to_string_lossy().into_owned(),
                        member_bytes: bytes.to_string(),
                        long_term_bytes: cached.map_or(0, |c| c.long_term_bytes).to_string(),
                        temporary_bytes: (cached.map_or(0, |c| c.temporary_bytes) + rank_bytes)
                            .to_string(),
                        ranked_index_bytes: rank_bytes.to_string(),
                        total_bytes: (bytes + rank_bytes).to_string(),
                        issue: p.issue,
                    }
                })
                .collect();
            Ok(CacheProjects { items })
        })
        .await?,
    ))
}

fn cleanup(task: studio_storage::QueryCleanup) -> CacheCleanup {
    CacheCleanup {
        family_id: task.family_id,
        result_id: task.result_id,
        spec: task.spec.map(Into::into),
        state: task.state,
        total: task.total,
        processed: task.processed,
        removed: task.removed,
        started_millis: task.started_millis,
        updated_millis: task.updated_millis,
        error: task.error,
    }
}
#[utoipa::path(get,path="/v1/cache/projects/{project_id}",operation_id="project_cache_inventory",params(("project_id"=String,Path),("cursor"=Option<String>,Query),("limit"=Option<usize>,Query)),responses((status=200,body=ProjectCacheInventory)))]
pub(super) async fn inventory(
    State(s): State<AppState>,
    Path(pid): Path<String>,
    Query(q): Query<QueryListParams>,
) -> ApiResult<ProjectCacheInventory> {
    Ok(Json(
        blocking(move || {
            let limit = q.limit.unwrap_or(32).clamp(1, 127);
            let mut inventory = s.store.cache_inventory(
                &pid,
                q.cursor.as_deref(),
                limit + 1,
                &s.queries.cache.live(&pid),
            )?;
            if inventory.members.iter().any(|m| m.bytes.is_none()) {
                s.queries
                    .cache
                    .request_audit(&pid, inventory.directory.clone())?;
            }
            let more = inventory.members.len() > limit;
            inventory.members.truncate(limit);
            let next_cursor = if more {
                inventory.members.last().map(|m| m.family_id.clone())
            } else {
                None
            };
            let members = inventory
                .members
                .into_iter()
                .map(|item| {
                    let reason = if item.in_use {
                        Some("正在使用".into())
                    } else if item.reference_count > 0 {
                        Some("被项目引用，请先处理引用对象".into())
                    } else if item.result.cache.fixed {
                        Some("已固定，请先取消固定".into())
                    } else {
                        None
                    };
                    CacheMemberItem {
                        family_id: item.family_id,
                        result_id: item.result.id,
                        spec: item.result.spec.into(),
                        cached: item.cached,
                        tier: item.result.cache.tier.as_str().into(),
                        fixed: item.result.cache.fixed,
                        session_only: item.result.cache.session_only,
                        members: item.members,
                        estimated_bytes: item.bytes.map(|n| n.to_string()),
                        last_used_millis: item.last_used_millis.to_string(),
                        in_use: item.in_use,
                        references: item.references,
                        reference_count: item.reference_count,
                        can_release: reason.is_none(),
                        reason,
                    }
                })
                .collect();
            let ranked_indexes = s
                .queries
                .ranked_indexes
                .inventory()?
                .into_iter()
                .filter(|r| r.meta.scope.project_id == pid)
                .map(|r| CacheRankedItem {
                    label: s
                        .store
                        .cache_scope_label(&pid, &r.meta.scope)
                        .unwrap_or_else(|_| "已移除范围的排名索引".into()),
                    key: r.meta.key,
                    path: r.path.to_string_lossy().into_owned(),
                    members: r.meta.count,
                    bytes: r.bytes.to_string(),
                    last_used_millis: r.last_used_millis.to_string(),
                    in_use: r.in_use,
                })
                .collect();
            Ok(ProjectCacheInventory {
                project_id: pid,
                member_path: inventory
                    .directory
                    .join("project.sqlite")
                    .to_string_lossy()
                    .into_owned(),
                members,
                next_cursor,
                ranked_indexes,
                cleanups: inventory.cleanups.into_iter().map(cleanup).collect(),
            })
        })
        .await?,
    ))
}

fn request_maintenance(s: &AppState, pid: &str) {
    s.queries.cache.phase("queued", Some(pid));
    s.queries.cache.requested.store(true, Ordering::Release);
    if s.store.view_is_open(pid) {
        s.queries.cache.track_committed(&s.store, pid);
    }
}
#[utoipa::path(post,path="/v1/cache/projects/{project_id}/members/{result_id}/release",operation_id="release_project_cache_member",params(("project_id"=String,Path),("result_id"=String,Path)),responses((status=200,body=OkResponse)))]
pub(super) async fn release_member(
    State(s): State<AppState>,
    Path((pid, rid)): Path<(String, String)>,
) -> ApiResult<OkResponse> {
    Ok(Json(
        blocking(move || {
            let _gate = s.queries.cache.lock()?;
            s.store.cache_project_action(&pid, |store| {
                store.release_cache_entry(&pid, &rid, &s.queries.cache.live(&pid))
            })?;
            request_maintenance(&s, &pid);
            Ok(OkResponse { ok: true })
        })
        .await?,
    ))
}
#[utoipa::path(put,path="/v1/cache/projects/{project_id}/members/{result_id}/retention",operation_id="retain_project_cache_member",params(("project_id"=String,Path),("result_id"=String,Path)),request_body=SetResultRetention,responses((status=200,body=OkResponse)))]
pub(super) async fn retention(
    State(s): State<AppState>,
    Extension(session): Extension<ClientSession>,
    Path((pid, rid)): Path<(String, String)>,
    Body(body): Body<SetResultRetention>,
) -> ApiResult<OkResponse> {
    Ok(Json(
        blocking(move || {
            let tier = match body.tier.as_str() {
                "long_term" => domain::QueryCacheTier::LongTerm,
                "temporary" => domain::QueryCacheTier::Temporary,
                _ => return Err(domain::Error::invalid("缓存类别必须是长期或临时")),
            };
            let _gate = s.queries.cache.lock()?;
            let config = s.queries.cache.config()?;
            let session_id = if s.store.view_is_open(&pid) {
                Some(s.queries.cache.session(&pid, session.0.as_deref())?)
            } else {
                None
            };
            s.store.cache_project_action(&pid, |store| {
                store.set_cache_retention(
                    &pid,
                    &rid,
                    tier,
                    body.fixed,
                    config.temporary_session_only,
                )?;
                if let Some(id) = &session_id {
                    store.bind_cache_session(&pid, &rid, id, config.temporary_session_only)?;
                }
                Ok(())
            })?;
            request_maintenance(&s, &pid);
            Ok(OkResponse { ok: true })
        })
        .await?,
    ))
}
#[utoipa::path(post,path="/v1/cache/projects/{project_id}/ranked/{key}/release",operation_id="release_project_ranked_index",params(("project_id"=String,Path),("key"=String,Path)),responses((status=200,body=OkResponse)))]
pub(super) async fn release_ranked(
    State(s): State<AppState>,
    Path((pid, key)): Path<(String, String)>,
) -> ApiResult<OkResponse> {
    Ok(Json(
        blocking(move || {
            domain::validate_id(&pid)?;
            s.queries.ranked_indexes.release(&pid, &key)?;
            Ok(OkResponse { ok: true })
        })
        .await?,
    ))
}
