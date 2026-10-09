use super::*;

fn validate_input(
    s: &AppState,
    pid: &str,
    spec: &domain::QuerySpec,
    versions: &[domain::QuerySourceVersion],
    read: &SourceRead,
) -> domain::Result<()> {
    s.store.query_input_count(pid, spec)?;
    if let Some(scope) = &spec.input_scope {
        if !spec.uses_only_fixed_project_data() {
            validate_scope(s, pid, scope, read)?;
        }
        if let domain::ScopeTarget::Source {
            source_id,
            revision,
        } = &scope.target
            && !versions
                .iter()
                .any(|v| v.source_id == *source_id && v.catalog_revision == *revision)
        {
            return Err(domain::Error::new(
                "SOURCE_CHANGED",
                "浏览来源已更新，请刷新后筛选",
            ));
        }
    }
    Ok(())
}

#[utoipa::path(post,path="/v1/projects/{project_id}/query-results",operation_id="run_query",params(("project_id"=String,Path)),request_body=RunQuery,responses((status=200,body=QueryResult)))]
pub(super) async fn run(
    State(s): State<AppState>,
    Extension(session): Extension<ClientSession>,
    Extension(context): Extension<RequestReadContext>,
    Path(pid): Path<String>,
    Body(body): Body<RunQuery>,
) -> ApiResult<QueryResult> {
    Ok(Json(
        blocking(move || {
            let spec = domain::QuerySpec::from(body.spec).normalize()?;
            let versions = s.queries.versions(&s.store, &pid, &spec)?;
            let read = read_permit(&s, domain::ReadClass::Index, &context)?;
            if let Some(result) = s.store.create_ranking_result(
                &pid,
                &spec,
                None,
                &versions,
                context.cancelled.clone(),
            )? {
                query_views::retain_created(&s, &pid, &result, true, &read)?;
                s.queries.cache.recent(&pid, &result.id);
                return Ok(result.into());
            }
            validate_input(&s, &pid, &spec, &versions, &read)?;
            if versions
                .iter()
                .any(|v| v.consistency == "retained_online_snapshot")
            {
                return query_views::create_fixed(&s, &pid, spec, versions, &read).map(Into::into);
            }
            let _cache_gate = s.queries.cache.lock()?;
            let result = s.store.create_result_with_cache(
                &pid,
                None,
                spec,
                versions,
                &s.queries.cache.request(&pid, session.0.as_deref())?,
            )?;
            s.queries.cache.recent(&pid, &result.id);
            s.queries.cache.track_committed(&s.store, &pid);
            Ok(result.into())
        })
        .await?,
    ))
}
use studio_application::{QueryAdapter, QueryRepository, ScopeRepository};

fn validate_spec(
    s: &AppState,
    pid: &str,
    spec: domain::QuerySpec,
) -> domain::Result<domain::QuerySpec> {
    let spec = spec.normalize()?;
    s.store.validate_derived(pid, &spec)?;
    let native = studio_storage::native_spec(&spec);
    let read = s.sources.inspect()?;
    let reader = read.query(domain::METADATA_MEMORY_BYTES, false);
    for id in &spec.source_ids {
        reader
            .fields(&s.store.source(pid, id)?)?
            .validate(&native)?;
    }
    Ok(spec)
}
#[utoipa::path(get,path="/v1/projects/{project_id}/sources/{source_id}/fields",params(("project_id"=String,Path),("source_id"=String,Path)),responses((status=200,body=FieldDirectory)))]
pub(super) async fn fields(
    State(s): State<AppState>,
    Extension(read_context): Extension<RequestReadContext>,
    Path((pid, sid)): Path<(String, String)>,
) -> ApiResult<FieldDirectory> {
    Ok(Json(
        blocking(move || {
            let _permit = read_permit(&s, domain::ReadClass::Index, &read_context)?;
            let mut directory = _permit
                .query(domain::METADATA_MEMORY_BYTES, false)
                .fields(&s.store.source(&pid, &sid)?)?;
            directory.fields.extend(s.store.derived_fields(&pid, &sid)?);
            Ok(directory.into())
        })
        .await?,
    ))
}
#[utoipa::path(get,path="/v1/projects/{project_id}/queries",params(("project_id"=String,Path),("cursor"=Option<String>,Query),("limit"=Option<usize>,Query)),responses((status=200,body=QueryDefinitions)))]
pub(super) async fn definitions(
    State(s): State<AppState>,
    Path(pid): Path<String>,
    Query(q): Query<QueryListParams>,
) -> ApiResult<QueryDefinitions> {
    Ok(Json(
        blocking(move || {
            let limit = q.limit.unwrap_or(50).clamp(1, 100);
            let mut items = s
                .store
                .query_definitions(&pid, q.cursor.as_deref(), limit + 1)?;
            let more = items.len() > limit;
            items.truncate(limit);
            Ok(QueryDefinitions {
                next_cursor: if more {
                    items.last().map(|q| q.id.clone())
                } else {
                    None
                },
                items: items.into_iter().map(Into::into).collect(),
            })
        })
        .await?,
    ))
}
#[utoipa::path(post,path="/v1/projects/{project_id}/queries",params(("project_id"=String,Path)),request_body=SaveQuery,responses((status=200,body=QueryDefinition)))]
pub(super) async fn create_definition(
    State(s): State<AppState>,
    Path(pid): Path<String>,
    Body(body): Body<SaveQuery>,
) -> ApiResult<QueryDefinition> {
    Ok(Json(
        blocking(move || {
            if body.expected_revision.is_some() {
                return Err(domain::Error::invalid("新查询不能指定旧版本"));
            }
            let spec = validate_spec(&s, &pid, body.spec.into())?;
            s.store
                .save_query(&pid, &body.name, spec, None)
                .map(Into::into)
        })
        .await?,
    ))
}
#[utoipa::path(get,path="/v1/projects/{project_id}/queries/{query_id}",params(("project_id"=String,Path),("query_id"=String,Path)),responses((status=200,body=QueryDefinition)))]
pub(super) async fn definition(
    State(s): State<AppState>,
    Path((pid, qid)): Path<(String, String)>,
) -> ApiResult<QueryDefinition> {
    Ok(Json(
        blocking(move || s.store.query_definition(&pid, &qid).map(Into::into)).await?,
    ))
}
#[utoipa::path(patch,path="/v1/projects/{project_id}/queries/{query_id}",params(("project_id"=String,Path),("query_id"=String,Path)),request_body=SaveQuery,responses((status=200,body=QueryDefinition)))]
pub(super) async fn update_definition(
    State(s): State<AppState>,
    Path((pid, qid)): Path<(String, String)>,
    Body(body): Body<SaveQuery>,
) -> ApiResult<QueryDefinition> {
    Ok(Json(
        blocking(move || {
            let revision = body
                .expected_revision
                .ok_or_else(|| domain::Error::invalid("修改查询需要当前版本"))?;
            let spec = validate_spec(&s, &pid, body.spec.into())?;
            s.store
                .save_query(&pid, &body.name, spec, Some((&qid, revision)))
                .map(Into::into)
        })
        .await?,
    ))
}
#[utoipa::path(post,path="/v1/projects/{project_id}/queries/{query_id}/results",params(("project_id"=String,Path),("query_id"=String,Path)),request_body=BuildQuery,responses((status=200,body=QueryResult)))]
pub(super) async fn build(
    State(s): State<AppState>,
    Extension(session): Extension<ClientSession>,
    Extension(read_context): Extension<RequestReadContext>,
    Path((pid, qid)): Path<(String, String)>,
    Body(body): Body<BuildQuery>,
) -> ApiResult<QueryResult> {
    Ok(Json(
        blocking(move || {
            let _permit = read_permit(&s, domain::ReadClass::Index, &read_context)?;
            let query = s.store.query_definition(&pid, &qid)?;
            if query.revision != body.expected_revision {
                return Err(domain::Error::new("REVISION_CONFLICT", "查询定义已变化"));
            }
            let versions = s.queries.versions(&s.store, &pid, &query.spec)?;
            if let Some(result) = s.store.create_ranking_result(
                &pid,
                &query.spec,
                Some((&qid, query.revision)),
                &versions,
                read_context.cancelled.clone(),
            )? {
                query_views::retain_created(&s, &pid, &result, true, &_permit)?;
                s.queries.cache.recent(&pid, &result.id);
                return Ok(result.into());
            }
            validate_input(&s, &pid, &query.spec, &versions, &_permit)?;
            if versions
                .iter()
                .any(|v| v.consistency == "retained_online_snapshot")
            {
                let result = s.store.create_snapshot_result_from(
                    &pid,
                    Some((&qid, query.revision)),
                    query.spec,
                    versions,
                    false,
                )?;
                query_views::retain_created(&s, &pid, &result, true, &_permit)?;
                return Ok(result.into());
            }
            let _cache_gate = s.queries.cache.lock()?;
            let result = s.store.create_result_with_cache(
                &pid,
                Some((&qid, query.revision)),
                query.spec,
                versions,
                &s.queries.cache.request(&pid, session.0.as_deref())?,
            )?;
            s.queries.cache.recent(&pid, &result.id);
            s.queries.cache.track_committed(&s.store, &pid);
            Ok(result.into())
        })
        .await?,
    ))
}
#[utoipa::path(get,path="/v1/projects/{project_id}/query-results",params(("project_id"=String,Path),("cursor"=Option<String>,Query),("limit"=Option<usize>,Query)),responses((status=200,body=QueryResults)))]
pub(super) async fn results(
    State(s): State<AppState>,
    Path(pid): Path<String>,
    Query(q): Query<QueryListParams>,
) -> ApiResult<QueryResults> {
    Ok(Json(
        blocking(move || {
            let limit = q.limit.unwrap_or(50).clamp(1, 100);
            let mut items = s
                .store
                .query_results(&pid, q.cursor.as_deref(), limit + 1)?;
            let more = items.len() > limit;
            items.truncate(limit);
            Ok(QueryResults {
                next_cursor: if more {
                    items.last().map(|q| q.id.clone())
                } else {
                    None
                },
                items: items.into_iter().map(Into::into).collect(),
            })
        })
        .await?,
    ))
}
#[utoipa::path(get,path="/v1/projects/{project_id}/query-results/{result_id}",params(("project_id"=String,Path),("result_id"=String,Path)),responses((status=200,body=QueryResult)))]
pub(super) async fn result(
    State(s): State<AppState>,
    Extension(session): Extension<ClientSession>,
    Path((pid, rid)): Path<(String, String)>,
) -> ApiResult<QueryResult> {
    Ok(Json(
        blocking(move || session_result(&s, &pid, &rid, session.0.as_deref()).map(Into::into))
            .await?,
    ))
}
fn session_result(
    s: &AppState,
    pid: &str,
    rid: &str,
    session: Option<&str>,
) -> domain::Result<domain::QueryResult> {
    s.queries.cache.session(pid, session)?;
    s.store
        .query_result_for_session(pid, rid, &s.queries.cache.live_sessions(pid)?)
}
#[utoipa::path(get,path="/v1/projects/{project_id}/query-results/{result_id}/validity",params(("project_id"=String,Path),("result_id"=String,Path)),responses((status=200,body=ResultValidity)))]
pub(super) async fn validity(
    State(s): State<AppState>,
    Extension(session): Extension<ClientSession>,
    Extension(read_context): Extension<RequestReadContext>,
    Path((pid, rid)): Path<(String, String)>,
) -> ApiResult<ResultValidity> {
    Ok(Json(
        blocking(move || {
            let _permit = read_permit(&s, domain::ReadClass::Index, &read_context)?;
            let result = session_result(&s, &pid, &rid, session.0.as_deref())?;
            let issue = s
                .queries
                .validate_result(&s.store, &result, &_permit)
                .err()
                .map(|e| e.to_string());
            Ok(ResultValidity {
                result_id: rid,
                current: issue.is_none(),
                issue,
                newer_available: result.state == domain::ResultState::Ready
                    && (s
                        .queries
                        .versions(&s.store, &pid, &result.spec)
                        .is_ok_and(|v| v != result.source_versions)
                        || result.spec.input_scope.as_ref().is_some_and(|scope| {
                            if let domain::ScopeTarget::Workset {
                                collection_id,
                                revision,
                            } = &scope.target
                            {
                                s.store
                                    .collection(&pid, collection_id)
                                    .is_ok_and(|c| c.revision != revision.unwrap_or(0))
                            } else {
                                false
                            }
                        })),
            })
        })
        .await?,
    ))
}
#[utoipa::path(post,path="/v1/projects/{project_id}/query-results/{result_id}/cancel",params(("project_id"=String,Path),("result_id"=String,Path)),responses((status=200,body=QueryResult)))]
pub(super) async fn cancel(
    State(s): State<AppState>,
    Path((pid, rid)): Path<(String, String)>,
) -> ApiResult<QueryResult> {
    Ok(Json(
        blocking(move || {
            s.queries.cancel(&rid);
            let result = s.store.cancel_result(&pid, &rid)?;
            Ok(result.into())
        })
        .await?,
    ))
}
#[utoipa::path(post,path="/v1/projects/{project_id}/query-results/{result_id}/release",params(("project_id"=String,Path),("result_id"=String,Path)),responses((status=200,body=QueryResult)))]
pub(super) async fn release(
    State(s): State<AppState>,
    Extension(context): Extension<RequestReadContext>,
    Path((pid, rid)): Path<(String, String)>,
) -> ApiResult<QueryResult> {
    Ok(Json(
        blocking(move || {
            let read = read_permit(&s, domain::ReadClass::Index, &context)?;
            let result = s.store.release_result(&pid, &rid)?;
            query_views::release_versions(&s, &pid, &result, &read)?;
            Ok(result.into())
        })
        .await?,
    ))
}
#[utoipa::path(post,path="/v1/projects/{project_id}/query-results/{result_id}/leases/{lease_id}",params(("project_id"=String,Path),("result_id"=String,Path),("lease_id"=String,Path)),responses((status=200,body=OkResponse)))]
pub(super) async fn lease_result(
    State(s): State<AppState>,
    Extension(session): Extension<ClientSession>,
    Extension(context): Extension<RequestReadContext>,
    Path((pid, rid, lid)): Path<(String, String, String)>,
) -> ApiResult<OkResponse> {
    Ok(Json(
        blocking(move || {
            if s.store.query_storage_kind(&pid, &rid)? == "view" {
                domain::validate_id(&lid)?;
                let result = s.store.touch_query_view(&pid, &rid)?;
                let read = read_permit(&s, domain::ReadClass::Index, &context)?;
                query_views::retain(&s, &pid, &result, false, &read)?;
                return Ok(OkResponse { ok: true });
            }
            let _gate = s.queries.cache.lock()?;
            let result = session_result(&s, &pid, &rid, session.0.as_deref())?;
            if result.state != domain::ResultState::Ready {
                return Err(domain::Error::new(
                    "RESULT_NOT_READY",
                    "查询结果已回收或尚未完成",
                ));
            }
            let session_id = s.queries.cache.session(&pid, session.0.as_deref())?;
            s.queries.cache.lease(&pid, &rid, &lid, &session_id)?;
            s.store.bind_cache_session(
                &pid,
                &rid,
                &session_id,
                s.queries.cache.config()?.temporary_session_only,
            )?;
            s.store.touch_query_cache(&pid, &rid)?;
            Ok(OkResponse { ok: true })
        })
        .await?,
    ))
}
#[utoipa::path(post,path="/v1/projects/{project_id}/query-results/{result_id}/leases/{lease_id}/release",params(("project_id"=String,Path),("result_id"=String,Path),("lease_id"=String,Path)),responses((status=200,body=OkResponse)))]
pub(super) async fn release_result_lease(
    State(s): State<AppState>,
    Path((pid, rid, lid)): Path<(String, String, String)>,
) -> ApiResult<OkResponse> {
    for value in [&pid, &rid, &lid] {
        domain::validate_id(value)?;
    }
    s.queries.cache.release(&pid, &rid, &lid);
    Ok(Json(OkResponse { ok: true }))
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResultCursor {
    project_id: String,
    result_id: String,
    after: domain::AssetKey,
    #[serde(default)]
    order: Option<domain::QueryOrder>,
}
#[utoipa::path(get,path="/v1/projects/{project_id}/query-results/{result_id}/assets",params(("project_id"=String,Path),("result_id"=String,Path),("cursor"=Option<String>,Query),("order"=Option<QueryOrder>,Query),("limit"=Option<usize>,Query)),responses((status=200,body=ResultAssets)))]
pub(super) async fn result_assets(
    State(s): State<AppState>,
    Extension(session): Extension<ClientSession>,
    Extension(read_context): Extension<RequestReadContext>,
    Path((pid, rid)): Path<(String, String)>,
    Query(q): Query<QueryListParams>,
) -> ApiResult<ResultAssets> {
    Ok(Json(
        blocking(move || {
            let _permit = read_permit(&s, domain::ReadClass::Index, &read_context)?;
            let peek = s.store.query_result(&pid, &rid)?;
            if peek.cache.mode == "view" {
                return query_views::assets(&s, &pid, &rid, q, &read_context, &_permit);
            }
            let result = {
                let _gate = s.queries.cache.lock()?;
                let result = session_result(&s, &pid, &rid, session.0.as_deref())?;
                if result.state == domain::ResultState::Ready {
                    s.queries.cache.recent(&pid, &rid);
                }
                result
            };
            s.queries.validate_result(&s.store, &result, &_permit)?;
            let order = q.order.map(Into::into).unwrap_or(result.spec.order);
            let mut projected_next = None;
            let live_post_positions = order.by_post()
                && (s.store.query_storage_kind(&pid, &rid)? == "ranking"
                    || result
                        .spec
                        .source_ids
                        .iter()
                        .map(|id| s.store.source(&pid, id))
                        .collect::<domain::Result<Vec<_>>>()?
                        .iter()
                        .any(|source| source.kind == "pixiv"));
            let page = if live_post_positions {
                let signature = hex::encode(Sha256::digest(
                    serde_json::to_vec(&(
                        "source-post-view-v2",
                        &pid,
                        &rid,
                        order,
                        &result.source_versions,
                    ))
                    .map_err(domain::Error::io)?,
                ));
                let revisions = result
                    .source_versions
                    .iter()
                    .map(|v| (v.source_id.clone(), v.catalog_revision.clone()))
                    .collect::<BTreeMap<_, _>>();
                let mut cursor = if let Some(raw) = q.cursor {
                    if raw.len() > 16384 {
                        return Err(domain::Error::invalid("结果游标过长"));
                    }
                    let cursor: super::Cursor = URL_SAFE_NO_PAD
                        .decode(raw)
                        .ok()
                        .and_then(|v| serde_json::from_slice(&v).ok())
                        .ok_or_else(|| domain::Error::invalid("结果游标无效"))?;
                    if cursor.scope != signature || cursor.revisions != revisions {
                        return Err(domain::Error::invalid("结果游标不属于当前成员、版本或排序"));
                    }
                    cursor
                } else {
                    super::Cursor {
                        scope: signature,
                        revisions,
                        ..Default::default()
                    }
                };
                let scope = domain::ScopeRef {
                    project_id: pid.clone(),
                    target: domain::ScopeTarget::QueryResult {
                        result_id: rid.clone(),
                    },
                };
                let page = scoped_browse::page(
                    &s,
                    &_permit,
                    &pid,
                    &scope,
                    &mut cursor,
                    order,
                    q.limit.unwrap_or(48).clamp(1, 128),
                )?
                .ok_or_else(|| domain::Error::new("QUERY_UNSUPPORTED", "当前来源不支持帖子排序"))?;
                let next = page.more.then(|| encode_cursor(&cursor)).transpose()?;
                if page.preparing.is_some() {
                    return Ok(ResultAssets {
                        result_id: rid.clone(),
                        count: result.count,
                        page: AssetPage {
                            items: Vec::new(),
                            next_cursor: next,
                            revision: rid,
                            preparing: page.preparing,
                            result_id: None,
                            scan: page.scan,
                            start_cursor: None,
                        },
                    });
                }
                projected_next = Some(next);
                domain::ResultPage {
                    keys: page.keys,
                    next: None,
                }
            } else {
                let after = q
                    .cursor
                    .map(|value| -> domain::Result<_> {
                        if value.len() > 1024 {
                            return Err(domain::Error::invalid("结果游标过长"));
                        }
                        let cursor: ResultCursor = URL_SAFE_NO_PAD
                            .decode(value)
                            .ok()
                            .and_then(|b| serde_json::from_slice(&b).ok())
                            .ok_or_else(|| domain::Error::invalid("结果游标无效"))?;
                        if cursor.project_id != pid || cursor.result_id != rid {
                            return Err(domain::Error::invalid("游标不属于当前结果"));
                        }
                        if cursor.order.unwrap_or(result.spec.order) != order {
                            return Err(domain::Error::invalid("分页排序已变化，请返回第一页"));
                        }
                        Ok(cursor.after)
                    })
                    .transpose()?;
                s.store.result_page_ordered(
                    &pid,
                    &rid,
                    after.as_ref(),
                    q.limit.unwrap_or(48),
                    order,
                )?
            };
            let mut groups = BTreeMap::<String, Vec<domain::AssetKey>>::new();
            for key in &page.keys {
                groups
                    .entry(key.source_id.clone())
                    .or_default()
                    .push(key.clone());
            }
            let mut assets = std::collections::HashMap::new();
            for (source_id, keys) in groups {
                let source = s.store.source(&pid, &source_id)?;
                for item in _permit.freeze_at(
                    &source,
                    &keys,
                    result
                        .source_versions
                        .iter()
                        .find(|v| v.source_id == source_id)
                        .filter(|v| {
                            v.consistency == "retained_online_snapshot"
                                || !result.spec.uses_only_fixed_project_data()
                        })
                        .map(|v| v.catalog_revision.as_str()),
                )? {
                    if !result.spec.uses_only_fixed_project_data()
                        && !result.source_versions.iter().any(|v| {
                            v.source_id == source_id && v.catalog_revision == item.source_revision
                        })
                    {
                        return Err(domain::Error::new("SOURCE_CHANGED", "分页期间来源已变化"));
                    }
                    assets.insert(item.asset.key.clone(), item.asset);
                }
            }
            let membership = s.store.contains(&pid, &page.keys)?;
            let mut items = page
                .keys
                .into_iter()
                .zip(membership)
                .map(|(key, selected)| {
                    assets
                        .remove(&key)
                        .map(|a| Asset::from_domain(a, selected))
                        .ok_or_else(|| domain::Error::new("SOURCE_CHANGED", "结果成员已不可用"))
                })
                .collect::<domain::Result<Vec<_>>>()?;
            enrich_summaries_at(
                &s,
                &pid,
                &read_context,
                &_permit,
                &mut items,
                &result
                    .source_versions
                    .iter()
                    .filter(|v| {
                        v.consistency == "retained_online_snapshot"
                            || !result.spec.uses_only_fixed_project_data()
                    })
                    .cloned()
                    .collect::<Vec<_>>(),
            )?;
            ranking_browse::annotate(
                &s,
                &pid,
                &domain::ScopeRef {
                    project_id: pid.clone(),
                    target: domain::ScopeTarget::QueryResult {
                        result_id: rid.clone(),
                    },
                },
                &read_context,
                &mut items,
            )?;
            let next_cursor = if let Some(next) = projected_next {
                next
            } else {
                page.next
                    .map(|after| {
                        serde_json::to_vec(&ResultCursor {
                            project_id: pid,
                            result_id: rid.clone(),
                            after,
                            order: Some(order),
                        })
                        .map(|b| URL_SAFE_NO_PAD.encode(b))
                        .map_err(domain::Error::io)
                    })
                    .transpose()?
            };
            Ok(ResultAssets {
                result_id: rid.clone(),
                count: result.count,
                page: AssetPage {
                    items,
                    next_cursor,
                    revision: rid,
                    preparing: None,
                    result_id: None,
                    scan: None,
                    start_cursor: None,
                },
            })
        })
        .await?,
    ))
}
pub(super) fn validate_scope(
    s: &AppState,
    pid: &str,
    scope: &domain::ScopeRef,
    read: &SourceRead,
) -> domain::Result<()> {
    scope.validate_project(pid)?;
    if let domain::ScopeTarget::QueryResult { result_id } = &scope.target {
        s.queries
            .validate_result(&s.store, &s.store.query_result(pid, result_id)?, read)?;
    }
    Ok(())
}
pub(super) fn requires_capture(
    s: &AppState,
    pid: &str,
    scope: &domain::ScopeRef,
) -> domain::Result<bool> {
    Ok(match &scope.target {
        domain::ScopeTarget::Source { .. } => true,
        domain::ScopeTarget::QueryResult { result_id } => {
            s.store.query_result(pid, result_id)?.cache.mode == "view"
        }
        _ => false,
    })
}
pub(super) fn source_capture(
    s: &AppState,
    pid: &str,
    scope: &domain::ScopeRef,
    read: &SourceRead,
) -> domain::Result<(domain::QuerySpec, Vec<domain::QuerySourceVersion>)> {
    scope.validate_project(pid)?;
    if let domain::ScopeTarget::QueryResult { result_id } = &scope.target {
        let result = s.store.touch_query_view(pid, result_id)?;
        s.queries.validate_result(&s.store, &result, read)?;
        return Ok((result.spec, result.source_versions));
    }
    let domain::ScopeTarget::Source {
        source_id,
        revision,
    } = &scope.target
    else {
        return Err(domain::Error::invalid(
            "只有来源范围需要后台捕获；其他范围已有成员引用",
        ));
    };
    let spec = domain::QuerySpec {
        version: 1,
        source_ids: vec![source_id.clone()],
        conditions: vec![],
        observation_rule: domain::ObservationRule::CurrentPost,
        order: domain::QueryOrder::AssetKeyAsc,
        input_scope: None,
    };
    let reader = read.query(domain::METADATA_MEMORY_BYTES, false);
    let versions =
        vec![reader.read_version_at(&s.store.source(pid, source_id)?, Some(revision), false)?];
    Ok((spec, versions))
}
#[utoipa::path(post,path="/v1/projects/{project_id}/scopes/capture",params(("project_id"=String,Path)),request_body=CaptureScope,responses((status=200,body=QueryResult)))]
pub(super) async fn capture(
    State(s): State<AppState>,
    Extension(session): Extension<ClientSession>,
    Extension(context): Extension<RequestReadContext>,
    Path(pid): Path<String>,
    Body(body): Body<CaptureScope>,
) -> ApiResult<QueryResult> {
    Ok(Json(
        blocking(move || {
            let read = read_permit(&s, domain::ReadClass::Index, &context)?;
            let (spec, versions) = source_capture(&s, &pid, &body.scope.into(), &read)?;
            if versions.iter().any(|v| {
                v.consistency == "retained_online_snapshot" || v.consistency == "immutable_demo"
            }) {
                return query_views::create_fixed(&s, &pid, spec, versions, &read).map(Into::into);
            }
            let _cache_gate = s.queries.cache.lock()?;
            let result = s.store.create_result_with_cache(
                &pid,
                None,
                spec,
                versions,
                &s.queries.cache.request(&pid, session.0.as_deref())?,
            )?;
            s.queries.cache.recent(&pid, &result.id);
            s.queries.cache.track_committed(&s.store, &pid);
            Ok(result.into())
        })
        .await?,
    ))
}
#[utoipa::path(post,path="/v1/projects/{project_id}/selection/scope",params(("project_id"=String,Path)),request_body=ChangeSelectionScope,responses((status=200,body=Selection)))]
pub(super) async fn select_scope(
    State(s): State<AppState>,
    Extension(read_context): Extension<RequestReadContext>,
    Path(pid): Path<String>,
    Body(body): Body<ChangeSelectionScope>,
) -> ApiResult<Selection> {
    Ok(Json(
        blocking(move || {
            let _permit = read_permit(&s, domain::ReadClass::Index, &read_context)?;
            let scope = body.scope.into();
            validate_scope(&s, &pid, &scope, &_permit)?;
            s.store
                .change_selection_scope(&pid, body.expected_revision, &scope, body.operation.into())
                .map(Into::into)
        })
        .await?,
    ))
}
