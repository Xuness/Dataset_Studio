use super::*;

fn validate_input(
    s: &AppState,
    pid: &str,
    spec: &domain::QuerySpec,
    versions: &[domain::QuerySourceVersion],
) -> domain::Result<()> {
    s.store.query_input_count(pid, spec)?;
    if let Some(scope) = &spec.input_scope {
        validate_scope(s, pid, scope)?;
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
    Path(pid): Path<String>,
    Body(body): Body<RunQuery>,
) -> ApiResult<QueryResult> {
    Ok(Json(
        blocking(move || {
            let spec = domain::QuerySpec::from(body.spec).normalize()?;
            let versions = s.queries.versions(&s.store, &pid, &spec)?;
            validate_input(&s, &pid, &spec, &versions)?;
            let _cache_gate = s
                .queries
                .cache
                .gate
                .lock()
                .map_err(|_| domain::Error::new("INTERNAL_ERROR", "查询缓存锁不可用"))?;
            let result = s.store.create_cached_result(
                &pid,
                None,
                spec,
                versions,
                s.queries.cache.config()?.quota_mib > 0,
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
    for id in &spec.source_ids {
        s.queries
            .reader
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
            let mut directory = s.queries.reader.fields(&s.store.source(&pid, &sid)?)?;
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
            validate_input(&s, &pid, &query.spec, &versions)?;
            let _cache_gate = s
                .queries
                .cache
                .gate
                .lock()
                .map_err(|_| domain::Error::new("INTERNAL_ERROR", "查询缓存锁不可用"))?;
            let result = s.store.create_cached_result(
                &pid,
                Some((&qid, query.revision)),
                query.spec,
                versions,
                s.queries.cache.config()?.quota_mib > 0,
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
    Path((pid, rid)): Path<(String, String)>,
) -> ApiResult<QueryResult> {
    Ok(Json(
        blocking(move || s.store.query_result(&pid, &rid).map(Into::into)).await?,
    ))
}
#[utoipa::path(get,path="/v1/projects/{project_id}/query-results/{result_id}/validity",params(("project_id"=String,Path),("result_id"=String,Path)),responses((status=200,body=ResultValidity)))]
pub(super) async fn validity(
    State(s): State<AppState>,
    Extension(read_context): Extension<RequestReadContext>,
    Path((pid, rid)): Path<(String, String)>,
) -> ApiResult<ResultValidity> {
    Ok(Json(
        blocking(move || {
            let _permit = read_permit(&s, domain::ReadClass::Index, &read_context)?;
            let result = s.store.query_result(&pid, &rid)?;
            let issue = s
                .queries
                .validate_result(&s.store, &result)
                .err()
                .map(|e| e.to_string());
            Ok(ResultValidity {
                result_id: rid,
                current: issue.is_none(),
                issue,
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
    Path((pid, rid)): Path<(String, String)>,
) -> ApiResult<QueryResult> {
    Ok(Json(
        blocking(move || s.store.release_result(&pid, &rid).map(Into::into)).await?,
    ))
}
#[utoipa::path(post,path="/v1/projects/{project_id}/query-results/{result_id}/leases/{lease_id}",params(("project_id"=String,Path),("result_id"=String,Path),("lease_id"=String,Path)),responses((status=200,body=OkResponse)))]
pub(super) async fn lease_result(
    State(s): State<AppState>,
    Path((pid, rid, lid)): Path<(String, String, String)>,
) -> ApiResult<OkResponse> {
    Ok(Json(
        blocking(move || {
            let _gate = s
                .queries
                .cache
                .gate
                .lock()
                .map_err(|_| domain::Error::new("INTERNAL_ERROR", "查询缓存锁不可用"))?;
            let result = s.store.query_result(&pid, &rid)?;
            if result.state != domain::ResultState::Ready {
                return Err(domain::Error::new(
                    "RESULT_NOT_READY",
                    "查询结果已回收或尚未完成",
                ));
            }
            s.queries.cache.lease(&pid, &rid, &lid)?;
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
    Extension(read_context): Extension<RequestReadContext>,
    Path((pid, rid)): Path<(String, String)>,
    Query(q): Query<QueryListParams>,
) -> ApiResult<ResultAssets> {
    Ok(Json(
        blocking(move || {
            let _permit = read_permit(&s, domain::ReadClass::Index, &read_context)?;
            let result =
                {
                    let _gate =
                        s.queries.cache.gate.lock().map_err(|_| {
                            domain::Error::new("INTERNAL_ERROR", "查询缓存锁不可用")
                        })?;
                    let result = s.store.query_result(&pid, &rid)?;
                    if result.state == domain::ResultState::Ready {
                        s.queries.cache.recent(&pid, &rid);
                    }
                    result
                };
            s.queries.validate_result(&s.store, &result)?;
            let order = q.order.map(Into::into).unwrap_or(result.spec.order);
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
            let page = s.store.result_page_ordered(
                &pid,
                &rid,
                after.as_ref(),
                q.limit.unwrap_or(48),
                order,
            )?;
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
                for item in SourceRouter.freeze(&source, &keys)? {
                    if !result.source_versions.iter().any(|v| {
                        v.source_id == source_id && v.catalog_revision == item.source_revision
                    }) {
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
            enrich_summaries(&s, &pid, &read_context, &mut items)?;
            let next_cursor = page
                .next
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
                .transpose()?;
            Ok(ResultAssets {
                result_id: rid.clone(),
                count: result.count.unwrap_or(0),
                page: AssetPage {
                    items,
                    next_cursor,
                    revision: rid,
                    preparing: None,
                    result_id: None,
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
) -> domain::Result<()> {
    scope.validate_project(pid)?;
    if let domain::ScopeTarget::QueryResult { result_id } = &scope.target {
        s.queries
            .validate_result(&s.store, &s.store.query_result(pid, result_id)?)?;
    }
    Ok(())
}
pub(super) fn source_capture(
    s: &AppState,
    pid: &str,
    scope: &domain::ScopeRef,
) -> domain::Result<(domain::QuerySpec, Vec<domain::QuerySourceVersion>)> {
    scope.validate_project(pid)?;
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
    let versions = s.queries.versions(&s.store, pid, &spec)?;
    if versions[0].catalog_revision != *revision {
        return Err(domain::Error::new(
            "SOURCE_CHANGED",
            "来源版本已变化，请刷新后重试",
        ));
    }
    Ok((spec, versions))
}
#[utoipa::path(post,path="/v1/projects/{project_id}/scopes/capture",params(("project_id"=String,Path)),request_body=CaptureScope,responses((status=200,body=QueryResult)))]
pub(super) async fn capture(
    State(s): State<AppState>,
    Path(pid): Path<String>,
    Body(body): Body<CaptureScope>,
) -> ApiResult<QueryResult> {
    Ok(Json(
        blocking(move || {
            let (spec, versions) = source_capture(&s, &pid, &body.scope.into())?;
            let _cache_gate = s
                .queries
                .cache
                .gate
                .lock()
                .map_err(|_| domain::Error::new("INTERNAL_ERROR", "查询缓存锁不可用"))?;
            let result = s.store.create_cached_result(
                &pid,
                None,
                spec,
                versions,
                s.queries.cache.config()?.quota_mib > 0,
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
            validate_scope(&s, &pid, &scope)?;
            s.store
                .change_selection_scope(&pid, body.expected_revision, &scope, body.operation.into())
                .map(Into::into)
        })
        .await?,
    ))
}
