use axum::{
    Json,
    extract::{Extension, FromRequest, Path, Query, Request, State, rejection::JsonRejection},
    http::{HeaderValue, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, convert::Infallible, sync::Arc};
use studio_application::{
    MetadataAdapter, ProjectRepository, QueryRepository, ScopeRepository, SourceAdapter,
};
use studio_domain as domain;
use studio_protocol::*;
use studio_sources::SourceRouter;
use studio_storage::SqliteStore;
use utoipa::OpenApi;
mod query;
mod resources;
mod settings;
mod source_locations;
mod tools;

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<SqliteStore>,
    pub connection: EngineConnection,
    pub resources: Arc<dyn studio_application::ReadResources>,
    pub previews: Arc<crate::previews::PreviewService>,
    pub metadata: Arc<studio_sources::MetadataReader>,
    pub queries: Arc<crate::query_jobs::QueryRunner>,
    pub shutdown: tokio::sync::watch::Sender<bool>,
}
#[derive(Clone)]
pub struct RequestReadContext {
    pub cancelled: studio_application::ReadCancellation,
    pub priority: domain::ReadPriority,
}
#[derive(Clone)]
pub struct ClientSession(pub Option<String>);
pub struct Failure(domain::Error);
impl From<domain::Error> for Failure {
    fn from(error: domain::Error) -> Self {
        Self(error)
    }
}
impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let code = self.0.code;
        let status = match code {
            "NOT_FOUND" => StatusCode::NOT_FOUND,
            "UNAUTHORIZED" => StatusCode::UNAUTHORIZED,
            "REVISION_CONFLICT"
            | "SOURCE_CHANGED"
            | "PROJECT_BUSY"
            | "PROJECT_CLOSED"
            | "RESULT_NOT_READY"
            | "RESULT_IN_USE"
            | "ARTIFACT_IN_USE"
            | "ARTIFACT_NOT_READY"
            | "DRAFT_CONFLICT"
            | "DRAFT_VERSION_UNSUPPORTED"
            | "SCOPE_PROJECT_MISMATCH"
            | "SOURCE_LOCATION_CONFLICT"
            | "PROJECT_ID_CONFLICT"
            | "IDEMPOTENCY_CONFLICT" => StatusCode::CONFLICT,
            "SOURCE_BUSY"
            | "SOURCE_UNAVAILABLE"
            | "METADATA_RUNTIME_UNAVAILABLE"
            | "READ_BUDGET_EXCEEDED"
            | "CACHE_BUSY" => StatusCode::SERVICE_UNAVAILABLE,
            "SOURCE_TIMEOUT" => StatusCode::GATEWAY_TIMEOUT,
            "SOURCE_RESOURCE_LIMIT" | "METADATA_LIMIT" => StatusCode::PAYLOAD_TOO_LARGE,
            "CANCELLED" => StatusCode::CONFLICT,
            "INVALID_INPUT"
            | "SOURCE_ID_MISMATCH"
            | "SOURCE_PATH_INVALID"
            | "FORMAT_UNSUPPORTED"
            | "METADATA_UNSUPPORTED"
            | "METADATA_RUNTIME_UNSUPPORTED"
            | "SOURCE_FORMAT_UNSUPPORTED" => StatusCode::BAD_REQUEST,
            "QUERY_UNSUPPORTED"
            | "QUERY_VERSION_UNSUPPORTED"
            | "SCOPE_REQUIRES_CAPTURE"
            | "OPERATOR_UNAVAILABLE"
            | "PARAMETERS_VERSION_UNSUPPORTED" => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let id = domain::new_id();
        tracing::warn!(request_id=%id,code,message=%self.0.message,"request failed");
        let mut response = (
            status,
            Json(ApiError {
                code: code.into(),
                message: self.0.message,
                request_id: id.clone(),
            }),
        )
            .into_response();
        if let Ok(value) = HeaderValue::from_str(&id) {
            response.headers_mut().insert("x-request-id", value);
        }
        response
    }
}
type ApiResult<T> = std::result::Result<Json<T>, Failure>;
pub struct Body<T>(T);
impl<T: DeserializeOwned, S: Send + Sync> FromRequest<S> for Body<T> {
    type Rejection = Failure;
    async fn from_request(req: Request, state: &S) -> std::result::Result<Self, Self::Rejection> {
        let Json(value) = Json::<T>::from_request(req, state)
            .await
            .map_err(|e: JsonRejection| Failure(domain::Error::invalid(e.body_text())))?;
        Ok(Self(value))
    }
}
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> domain::Result<T> + Send + 'static,
) -> std::result::Result<T, Failure> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| Failure(domain::Error::io(e)))?
        .map_err(Failure)
}

#[utoipa::path(get,path="/v1/health",responses((status=200,body=Health)))]
async fn health(State(s): State<AppState>) -> Json<Health> {
    Json(Health {
        api_version: API_VERSION,
        version: env!("CARGO_PKG_VERSION").into(),
        instance_id: s.connection.instance_id,
    })
}
#[utoipa::path(get,path="/v1/projects",responses((status=200,body=Projects)))]
async fn projects(State(s): State<AppState>) -> ApiResult<Projects> {
    Ok(Json(Projects {
        items: blocking(move || s.store.list())
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    }))
}
#[utoipa::path(post,path="/v1/projects",request_body=CreateProject,responses((status=200,body=Project),(status=400,body=ApiError)))]
async fn create_project(
    State(s): State<AppState>,
    Body(body): Body<CreateProject>,
) -> ApiResult<Project> {
    Ok(Json(
        blocking(move || {
            s.store
                .create(&body.name, body.parent_directory.map(Into::into))
        })
        .await?
        .into(),
    ))
}
#[utoipa::path(post,path="/v1/projects/open",request_body=OpenProject,responses((status=200,body=Project)))]
async fn open_project(
    State(s): State<AppState>,
    Body(body): Body<OpenProject>,
) -> ApiResult<Project> {
    Ok(Json(
        blocking(move || {
            let _gate = s
                .queries
                .cache
                .gate
                .lock()
                .map_err(|_| domain::Error::new("INTERNAL_ERROR", "查询缓存锁不可用"))?;
            let project = s.store.open(body.directory.into())?;
            s.queries.cache.track_committed(&s.store, &project.id);
            Ok(project)
        })
        .await?
        .into(),
    ))
}
#[utoipa::path(post,path="/v1/projects/{project_id}/open",params(("project_id"=String,Path)),responses((status=200,body=Project)))]
async fn open_recent(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult<Project> {
    Ok(Json(
        blocking(move || {
            let _gate = s
                .queries
                .cache
                .gate
                .lock()
                .map_err(|_| domain::Error::new("INTERNAL_ERROR", "查询缓存锁不可用"))?;
            let project = s.store.open_recent(&id)?;
            s.queries.cache.track_committed(&s.store, &id);
            Ok(project)
        })
        .await?
        .into(),
    ))
}
#[utoipa::path(post,path="/v1/projects/{project_id}/close",params(("project_id"=String,Path)),responses((status=200,body=ProjectClose)))]
async fn close_project(
    State(s): State<AppState>,
    Extension(session): Extension<ClientSession>,
    Path(id): Path<String>,
) -> ApiResult<ProjectClose> {
    Ok(Json(
        blocking(move || {
            let _gate = s
                .queries
                .cache
                .gate
                .lock()
                .map_err(|_| domain::Error::new("INTERNAL_ERROR", "查询缓存锁不可用"))?;
            if s.store.directory(&id).is_ok() {
                s.queries.cache.track_committed(&s.store, &id);
            }
            s.queries.cache.end_session(&id, session.0.as_deref());
            let result =
                if s.store.view_is_open(&id) && !s.queries.cache.live_sessions(&id)?.is_empty() {
                    domain::ProjectClose {
                        project_id: id,
                        state: domain::ProjectState::Open,
                    }
                } else {
                    s.store.close(&id)?
                };
            Ok(result)
        })
        .await?
        .into(),
    ))
}
#[utoipa::path(get,path="/v1/projects/{project_id}",params(("project_id"=String,Path)),responses((status=200,body=Project)))]
async fn project(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult<Project> {
    Ok(Json(blocking(move || s.store.project(&id)).await?.into()))
}
#[utoipa::path(get,path="/v1/projects/{project_id}/sources/{source_id}/assets/{asset_id}",operation_id="asset_detail",params(("project_id"=String,Path),("source_id"=String,Path),("asset_id"=String,Path)),responses((status=200,body=Asset)))]
async fn asset_detail(
    State(s): State<AppState>,
    Extension(read_context): Extension<RequestReadContext>,
    Path((pid, sid, aid)): Path<(String, String, String)>,
) -> ApiResult<Asset> {
    Ok(Json(
        blocking(move || {
            let _permit = read_permit(&s, domain::ReadClass::Index, &read_context)?;
            let key = domain::AssetKey {
                source_id: sid.clone(),
                asset_id: aid,
            };
            let source = s.store.source(&pid, &sid)?;
            let item = SourceRouter
                .freeze(&source, std::slice::from_ref(&key))?
                .into_iter()
                .next()
                .ok_or_else(|| domain::Error::new("NOT_FOUND", "焦点对象已不可用"))?;
            let selected = s
                .store
                .contains(&pid, &[key])?
                .first()
                .copied()
                .unwrap_or(false);
            let mut items = vec![Asset::from_domain(item.asset, selected)];
            enrich_summaries(&s, &pid, &read_context, &mut items)?;
            Ok(items.remove(0))
        })
        .await?,
    ))
}

fn read_permit(
    s: &AppState,
    class: domain::ReadClass,
    context: &RequestReadContext,
) -> domain::Result<Box<dyn studio_application::ReadLease>> {
    s.resources.acquire(
        domain::ReadRequest {
            class,
            priority: context.priority,
            bytes: if class == domain::ReadClass::NativeQuery {
                domain::METADATA_MEMORY_BYTES
            } else {
                32 << 20
            },
        },
        &context.cancelled,
    )
}

fn enrich_summaries(
    s: &AppState,
    pid: &str,
    context: &RequestReadContext,
    items: &mut [Asset],
) -> domain::Result<()> {
    let mut groups = BTreeMap::<String, Vec<String>>::new();
    for item in items.iter() {
        groups
            .entry(item.key.source_id.clone())
            .or_default()
            .push(item.key.asset_id.clone());
    }
    for (sid, ids) in groups {
        let source = s.store.source(pid, &sid)?;
        if source.kind != "danbooru" {
            continue;
        }
        let result = (|| {
            let _permit = read_permit(s, domain::ReadClass::NativeQuery, context)?;
            s.metadata
                .summaries(&source, &ids, context.cancelled.clone())
        })();
        match result {
            Ok(summaries) => {
                let mut summaries = summaries
                    .into_iter()
                    .map(|v| (v.asset_id.clone(), v))
                    .collect::<std::collections::HashMap<_, _>>();
                for item in items.iter_mut().filter(|item| item.key.source_id == sid) {
                    item.summary = summaries.remove(&item.key.asset_id).map(Into::into);
                }
            }
            Err(e) if e.code == "CANCELLED" => return Err(e),
            Err(e) => {
                for item in items.iter_mut().filter(|item| item.key.source_id == sid) {
                    item.summary = Some(AssetSummary {
                        status: "unavailable".into(),
                        post_ids: Vec::new(),
                        post_count: None,
                        version: None,
                        issue: Some(e.to_string()),
                    });
                }
            }
        }
    }
    Ok(())
}
#[utoipa::path(get,path="/v1/projects/{project_id}/sources/{source_id}/assets/{asset_id}/metadata",params(("project_id"=String,Path),("source_id"=String,Path),("asset_id"=String,Path),("cursor"=Option<String>,Query),("limit"=Option<usize>,Query),("version"=Option<String>,Query)),responses((status=200,body=MetadataOverview),(status=409,body=ApiError),(status=503,body=ApiError)))]
async fn metadata(
    State(s): State<AppState>,
    Extension(read_context): Extension<RequestReadContext>,
    Path((pid, sid, aid)): Path<(String, String, String)>,
    Query(q): Query<MetadataQuery>,
) -> ApiResult<MetadataOverview> {
    Ok(Json(
        blocking(move || {
            let _permit = read_permit(&s, domain::ReadClass::NativeQuery, &read_context)?;
            let source = s.store.source(&pid, &sid)?;
            s.metadata.metadata(&source, &aid, q.into()).map(Into::into)
        })
        .await?,
    ))
}
#[utoipa::path(get,path="/v1/projects/{project_id}/sources/{source_id}/assets/{asset_id}/records/{record_id}/observations",params(("project_id"=String,Path),("source_id"=String,Path),("asset_id"=String,Path),("record_id"=String,Path),("cursor"=Option<String>,Query),("limit"=Option<usize>,Query),("version"=Option<String>,Query)),responses((status=200,body=ObservationPage),(status=409,body=ApiError),(status=503,body=ApiError)))]
async fn observations(
    State(s): State<AppState>,
    Extension(read_context): Extension<RequestReadContext>,
    Path((pid, sid, aid, rid)): Path<(String, String, String, String)>,
    Query(q): Query<MetadataQuery>,
) -> ApiResult<ObservationPage> {
    Ok(Json(
        blocking(move || {
            let _permit = read_permit(&s, domain::ReadClass::NativeQuery, &read_context)?;
            let source = s.store.source(&pid, &sid)?;
            s.metadata
                .observations(&source, &aid, &rid, q.into())
                .map(Into::into)
        })
        .await?,
    ))
}
#[utoipa::path(get,path="/v1/projects/{project_id}/sources/{source_id}/assets/{asset_id}/records/{record_id}/observations/{observation_id}/raw",params(("project_id"=String,Path),("source_id"=String,Path),("asset_id"=String,Path),("record_id"=String,Path),("observation_id"=String,Path),("version"=String,Query)),responses((status=200,body=RawMetadata),(status=409,body=ApiError),(status=503,body=ApiError)))]
async fn raw_metadata(
    State(s): State<AppState>,
    Extension(read_context): Extension<RequestReadContext>,
    Path((pid, sid, aid, rid, oid)): Path<(String, String, String, String, String)>,
    Query(q): Query<RawMetadataQuery>,
) -> ApiResult<RawMetadata> {
    Ok(Json(
        blocking(move || {
            let _permit = read_permit(&s, domain::ReadClass::NativeQuery, &read_context)?;
            let source = s.store.source(&pid, &sid)?;
            s.metadata
                .raw_metadata(&source, &aid, &rid, &oid, &q.version)
                .map(Into::into)
        })
        .await?,
    ))
}
#[utoipa::path(get,path="/v1/projects/{project_id}/sources",params(("project_id"=String,Path)),responses((status=200,body=Sources)))]
async fn sources(
    State(s): State<AppState>,
    Extension(read_context): Extension<RequestReadContext>,
    Path(id): Path<String>,
) -> ApiResult<Sources> {
    Ok(Json(Sources {
        items: blocking(move || {
            let _permit = read_permit(&s, domain::ReadClass::Index, &read_context)?;
            Ok(s.store
                .sources(&id)?
                .into_iter()
                .map(|source| {
                    let probe = SourceRouter.probe(&source);
                    match probe {
                        Ok(p) => Source {
                            id: source.id,
                            name: source.name,
                            kind: source.kind,
                            revision: Some(p.revision),
                            enumeration: p.enumeration,
                            count: p.count,
                            available: true,
                            issue: None,
                        },
                        Err(e) => Source {
                            id: source.id,
                            name: source.name,
                            kind: source.kind,
                            revision: None,
                            enumeration: "unavailable".into(),
                            count: None,
                            available: false,
                            issue: Some(e.to_string()),
                        },
                    }
                })
                .collect())
        })
        .await?,
    }))
}
#[utoipa::path(post,path="/v1/projects/{project_id}/sources",params(("project_id"=String,Path)),request_body=AttachSource,responses((status=200,body=Source)))]
async fn attach_source(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<AttachSource>,
) -> ApiResult<Source> {
    Ok(Json(
        blocking(move || {
            let mut source = domain::Source {
                id: String::new(),
                name: domain::validate_name(&body.name)?,
                kind: body.kind,
                index_root: body.index_root.map(Into::into),
                media_root: body.media_root.map(Into::into),
            };
            let probe = SourceRouter.probe(&source)?;
            domain::validate_id(&probe.id)?;
            source.id = probe.id.clone();
            s.store.attach(&id, source.clone())?;
            Ok(Source {
                id: source.id,
                name: source.name,
                kind: source.kind,
                revision: Some(probe.revision),
                enumeration: probe.enumeration,
                count: probe.count,
                available: true,
                issue: None,
            })
        })
        .await?,
    ))
}
#[derive(Default, Serialize, Deserialize)]
struct Cursor {
    scope: String,
    source: usize,
    after: Option<String>,
    last_key: Option<domain::AssetKey>,
    revisions: BTreeMap<String, String>,
    #[serde(default)]
    source_afters: BTreeMap<String, String>,
    #[serde(default)]
    sorted_result_id: Option<String>,
}
fn encode_cursor(cursor: &Cursor) -> domain::Result<String> {
    Ok(URL_SAFE_NO_PAD.encode(serde_json::to_vec(cursor).map_err(domain::Error::io)?))
}
fn browse_sync(s: &AppState, id: &str, query: BrowseQuery) -> domain::Result<AssetPage> {
    let store = &s.store;
    let order = query
        .order
        .map(Into::into)
        .unwrap_or(domain::QueryOrder::AssetKeyAsc);
    let selected_scope = query.selection.unwrap_or(false);
    if usize::from(query.source_id.is_some())
        + usize::from(query.collection_id.is_some())
        + usize::from(selected_scope)
        > 1
    {
        return Err(domain::Error::invalid("一次只能浏览一个数据范围"));
    }
    let selection_revision = if selected_scope {
        Some(store.selection(id)?.revision)
    } else {
        None
    };
    let limit = query.limit.unwrap_or(48).clamp(1, 128);
    let mut sources = store.sources(id)?;
    if let Some(source_id) = &query.source_id {
        sources.retain(|s| &s.id == source_id);
        if sources.is_empty() {
            return Err(domain::Error::new("NOT_FOUND", "数据源不在当前项目中"));
        }
    }
    let scope = hex::encode(Sha256::digest(
        serde_json::to_vec(&(
            id,
            &sources,
            &query.collection_id,
            selection_revision,
            order,
        ))
        .map_err(domain::Error::io)?,
    ));
    let mut cursor = match query.cursor {
        Some(raw) => {
            if raw.len() > 16384 {
                return Err(domain::Error::invalid("分页游标过长"));
            }
            let bytes = URL_SAFE_NO_PAD
                .decode(raw)
                .map_err(|_| domain::Error::invalid("无效的分页游标"))?;
            let c: Cursor = serde_json::from_slice(&bytes)
                .map_err(|_| domain::Error::invalid("无效的分页游标"))?;
            if c.scope != scope {
                return Err(domain::Error::new(
                    "SOURCE_CHANGED",
                    "数据范围已变化，请重新查询",
                ));
            }
            c
        }
        None => Cursor {
            scope,
            ..Default::default()
        },
    };
    let mut items = Vec::new();
    let mut has_more = false;
    if query.collection_id.is_some() || selected_scope {
        if let Some(collection) = &query.collection_id
            && !store.collections(id)?.iter().any(|c| &c.id == collection)
        {
            return Err(domain::Error::new("NOT_FOUND", "工作集不存在"));
        }
        let sorted = if order != domain::QueryOrder::AssetKeyAsc {
            let input_scope = domain::ScopeRef {
                project_id: id.into(),
                target: if let Some(collection) = &query.collection_id {
                    domain::ScopeTarget::Workset {
                        collection_id: collection.clone(),
                    }
                } else {
                    domain::ScopeTarget::Selection {
                        revision: selection_revision.unwrap_or(0),
                    }
                },
            };
            let prepared = cursor
                .sorted_result_id
                .as_ref()
                .map(|rid| store.query_result(id, rid))
                .transpose()?;
            let source_ids = if let Some(result) = &prepared {
                if result.spec.input_scope.as_ref() != Some(&input_scope)
                    || !result.spec.conditions.is_empty()
                {
                    return Err(domain::Error::invalid("排序游标不属于当前范围"));
                }
                result.spec.source_ids.clone()
            } else {
                store.scope_source_ids(id, &input_scope)?
            };
            if source_ids.is_empty() {
                return Ok(AssetPage {
                    items: vec![],
                    next_cursor: None,
                    revision: cursor.scope,
                    preparing: None,
                    result_id: None,
                });
            }
            let spec = domain::QuerySpec {
                version: 3,
                source_ids,
                conditions: vec![],
                observation_rule: domain::ObservationRule::AnyObservation,
                order,
                input_scope: Some(input_scope),
            }
            .normalize()?;
            let result =
                {
                    let _gate =
                        s.queries.cache.gate.lock().map_err(|_| {
                            domain::Error::new("INTERNAL_ERROR", "查询缓存锁不可用")
                        })?;
                    let result = if let Some(result) = prepared {
                        result
                    } else {
                        let versions = s.queries.versions(store, id, &spec)?;
                        store.browse_result(
                            id,
                            spec,
                            versions,
                            s.queries.cache.config()?.query_enabled(),
                        )?
                    };
                    s.queries.cache.recent(id, &result.id);
                    result
                };
            if matches!(
                result.state,
                domain::ResultState::Queued | domain::ResultState::Running
            ) {
                return Ok(AssetPage {
                    items: vec![],
                    next_cursor: None,
                    revision: cursor.scope,
                    preparing: Some("正在准备范围排序".into()),
                    result_id: Some(result.id),
                });
            }
            s.queries.validate_result(store, &result)?;
            cursor.sorted_result_id = Some(result.id.clone());
            let page = store.result_page_ordered(
                id,
                &result.id,
                cursor.last_key.as_ref(),
                limit,
                order,
            )?;
            has_more = page.next.is_some();
            Some(page.keys)
        } else {
            None
        };
        let sorted_page = sorted.is_some();
        let mut keys = if let Some(keys) = sorted {
            keys
        } else if let Some(collection) = &query.collection_id {
            store.collection_keys(id, collection, cursor.last_key.as_ref(), limit + 1)?
        } else {
            store.selection_keys(id, cursor.last_key.as_ref(), limit + 1)?
        };
        if !sorted_page {
            has_more = keys.len() > limit;
        }
        keys.truncate(limit);
        cursor.last_key = keys.last().cloned();
        for key in keys {
            let source = store.source(id, &key.source_id)?;
            let frozen = SourceRouter.freeze(&source, &[key])?;
            for item in frozen {
                if let Some(old) = cursor.revisions.get(&source.id)
                    && old != &item.source_revision
                {
                    return Err(domain::Error::new(
                        "SOURCE_CHANGED",
                        "来源已更新，请重新查询",
                    ));
                }
                cursor
                    .revisions
                    .insert(source.id.clone(), item.source_revision);
                items.push(item.asset);
            }
        }
    } else if order != domain::QueryOrder::AssetKeyAsc {
        if sources.len() > 8 {
            return Err(domain::Error::invalid(
                "帖子 ID 排序最多合并 8 个来源，请缩小浏览范围",
            ));
        }
        if order.by_post() {
            let mut ready = true;
            for source in &sources {
                ready &= s.queries.prepare_browse_index(source)?;
            }
            if !ready {
                return Ok(AssetPage {
                    items: vec![],
                    next_cursor: None,
                    revision: cursor.scope,
                    preparing: Some("正在更新帖子排序索引".into()),
                    result_id: None,
                });
            }
        }
        let mut candidates = Vec::new();
        let mut source_more = false;
        for source in &sources {
            let after = cursor.source_afters.get(&source.id).map(String::as_str);
            let (mut rows, revision) = if source.kind == "danbooru" && order.by_post() {
                let index = s.queries.browse_index.reader(source)?;
                let revision = format!(
                    "catalog-v1:{}:{}",
                    index.stamp.generation, index.stamp.sequence
                );
                (index.page(&source.id, order, after, limit + 1)?, revision)
            } else {
                let page = SourceRouter.page_ordered(
                    source,
                    after,
                    limit + 1,
                    None,
                    order.descending(),
                )?;
                source_more |= page.next.is_some();
                let rows = page.items.into_iter().map(|a| (a.key, None)).collect();
                (rows, page.revision)
            };
            if cursor
                .revisions
                .get(&source.id)
                .is_some_and(|old| old != &revision)
            {
                return Err(domain::Error::new(
                    "SOURCE_CHANGED",
                    "浏览来源已更新，请返回第一页",
                ));
            }
            cursor.revisions.insert(source.id.clone(), revision);
            candidates.append(&mut rows);
        }
        candidates.sort_by(|(a, ap), (b, bp)| {
            let ids = match (ap, bp) {
                (Some(a), Some(b)) => {
                    if order.descending() {
                        b.cmp(a)
                    } else {
                        a.cmp(b)
                    }
                }
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(_), None) => std::cmp::Ordering::Less,
                _ => std::cmp::Ordering::Equal,
            };
            ids.then_with(|| {
                if order.descending() {
                    (&b.source_id, &b.asset_id).cmp(&(&a.source_id, &a.asset_id))
                } else {
                    (&a.source_id, &a.asset_id).cmp(&(&b.source_id, &b.asset_id))
                }
            })
        });
        has_more = candidates.len() > limit || source_more;
        candidates.truncate(limit);
        let mut resolved = std::collections::HashMap::new();
        for source in &sources {
            let keys = candidates
                .iter()
                .filter(|(k, _)| k.source_id == source.id)
                .map(|(k, _)| k.clone())
                .collect::<Vec<_>>();
            if let Some(last) = keys.last() {
                cursor
                    .source_afters
                    .insert(source.id.clone(), last.asset_id.clone());
            }
            for item in SourceRouter.freeze(source, &keys)? {
                if cursor.revisions.get(&source.id) != Some(&item.source_revision) {
                    return Err(domain::Error::new(
                        "SOURCE_CHANGED",
                        "排序读取期间来源已变化",
                    ));
                }
                resolved.insert(item.asset.key.clone(), item.asset);
            }
            if source.kind == "danbooru" {
                studio_sources::BrowseIndex::verify_revision(
                    source,
                    &cursor.revisions[&source.id],
                )?;
            }
        }
        for (key, _) in candidates {
            items.push(
                resolved
                    .remove(&key)
                    .ok_or_else(|| domain::Error::new("SOURCE_CHANGED", "排序成员已不可用"))?,
            );
        }
    } else {
        while items.len() < limit && cursor.source < sources.len() {
            let source = &sources[cursor.source];
            let page = SourceRouter.page(
                source,
                cursor.after.as_deref(),
                limit - items.len(),
                cursor.revisions.get(&source.id).map(String::as_str),
            )?;
            cursor.revisions.insert(source.id.clone(), page.revision);
            items.extend(page.items);
            if let Some(next) = page.next {
                cursor.after = Some(next);
                has_more = true;
                break;
            } else {
                cursor.source += 1;
                cursor.after = None;
            }
        }
        has_more |= cursor.source < sources.len();
    }
    let membership =
        store.contains(id, &items.iter().map(|a| a.key.clone()).collect::<Vec<_>>())?;
    if selection_revision.is_some_and(|revision| {
        store
            .selection(id)
            .map(|s| s.revision != revision)
            .unwrap_or(true)
    }) {
        return Err(domain::Error::new(
            "SOURCE_CHANGED",
            "选择范围已变化，请从第一页重新读取",
        ));
    }
    let revision = hex::encode(Sha256::digest(
        serde_json::to_vec(&cursor.revisions).map_err(domain::Error::io)?,
    ));
    Ok(AssetPage {
        items: items
            .into_iter()
            .zip(membership)
            .map(|(a, selected)| Asset::from_domain(a, selected))
            .collect(),
        next_cursor: if has_more {
            Some(encode_cursor(&cursor)?)
        } else {
            None
        },
        revision,
        preparing: None,
        result_id: cursor.sorted_result_id,
    })
}
#[utoipa::path(get,path="/v1/projects/{project_id}/assets",params(("project_id"=String,Path),("source_id"=Option<String>,Query),("collection_id"=Option<String>,Query),("selection"=Option<bool>,Query),("cursor"=Option<String>,Query),("order"=Option<QueryOrder>,Query),("limit"=Option<usize>,Query)),responses((status=200,body=AssetPage)))]
async fn assets(
    State(s): State<AppState>,
    Extension(read_context): Extension<RequestReadContext>,
    Path(id): Path<String>,
    Query(query): Query<BrowseQuery>,
) -> ApiResult<AssetPage> {
    Ok(Json(
        blocking(move || {
            let _permit = read_permit(&s, domain::ReadClass::Index, &read_context)?;
            let mut page = browse_sync(&s, &id, query)?;
            enrich_summaries(&s, &id, &read_context, &mut page.items)?;
            Ok(page)
        })
        .await?,
    ))
}
#[derive(Deserialize)]
struct MediaQuery {
    edge: Option<u32>,
    request_id: Option<String>,
    priority: Option<domain::ReadPriority>,
    max_source_bytes: Option<u64>,
}
#[utoipa::path(get,path="/v1/projects/{project_id}/sources/{source_id}/assets/{asset_id}/media",params(("project_id"=String,Path),("source_id"=String,Path),("asset_id"=String,Path),("edge"=Option<u32>,Query),("request_id"=Option<String>,Query),("priority"=Option<String>,Query,description="interactive, background or prefetch"),("max_source_bytes"=Option<u64>,Query,description="Cold generation input byte limit, at most 64 MiB")),responses((status=200,description="Authenticated image bytes; x-studio-cache, x-studio-freshness and x-studio-verified-ms report cache verification",content_type="image/jpeg")))]
async fn media(
    State(s): State<AppState>,
    Path((pid, sid, aid)): Path<(String, String, String)>,
    Query(q): Query<MediaQuery>,
) -> std::result::Result<Response, Failure> {
    let ticket = s
        .previews
        .ticket(&pid, &q.request_id.unwrap_or_else(domain::new_id))?;
    let result = s
        .previews
        .get(
            s.store,
            &ticket,
            pid,
            domain::AssetKey {
                source_id: sid,
                asset_id: aid,
            },
            crate::previews::PreviewOptions {
                edge: q.edge.unwrap_or(360),
                priority: q.priority.unwrap_or(domain::ReadPriority::Interactive),
                max_source_bytes: q.max_source_bytes.unwrap_or(64 << 20),
            },
        )
        .await?;
    Ok((
        [
            ("content-type", result.media.content_type.clone()),
            ("cache-control", "no-store".into()),
            ("x-studio-cache", result.cache.into()),
            ("x-studio-freshness", result.freshness.into()),
            ("x-studio-verified-ms", result.verified_ms.to_string()),
        ],
        result.media.bytes.clone(),
    )
        .into_response())
}
#[utoipa::path(get,path="/v1/projects/{project_id}/selection",params(("project_id"=String,Path)),responses((status=200,body=Selection)))]
async fn selection(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult<Selection> {
    Ok(Json(blocking(move || s.store.selection(&id)).await?.into()))
}
#[utoipa::path(patch,path="/v1/projects/{project_id}/selection",params(("project_id"=String,Path)),request_body=ChangeSelection,responses((status=200,body=Selection)))]
async fn change_selection(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<ChangeSelection>,
) -> ApiResult<Selection> {
    Ok(Json(
        blocking(move || {
            let add = body
                .add
                .into_iter()
                .map(Into::into)
                .collect::<Vec<domain::AssetKey>>();
            let remove = body.remove.into_iter().map(Into::into).collect::<Vec<_>>();
            if add.len() + remove.len() > 1000 {
                return Err(domain::Error::invalid("单次选择变更过大"));
            }
            let mut groups = BTreeMap::<String, Vec<domain::AssetKey>>::new();
            for key in &add {
                groups
                    .entry(key.source_id.clone())
                    .or_default()
                    .push(key.clone());
            }
            for (sid, keys) in groups {
                SourceRouter.freeze(&s.store.source(&id, &sid)?, &keys)?;
            }
            s.store
                .change_selection(&id, body.expected_revision, &add, &remove, body.clear)
        })
        .await?
        .into(),
    ))
}
#[utoipa::path(get,path="/v1/projects/{project_id}/collections",params(("project_id"=String,Path)),responses((status=200,body=Collections)))]
async fn collections(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult<Collections> {
    Ok(Json(Collections {
        items: blocking(move || s.store.collections(&id))
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    }))
}
#[utoipa::path(post,path="/v1/projects/{project_id}/collections",params(("project_id"=String,Path)),request_body=CreateCollection,responses((status=200,body=Collection)))]
async fn create_collection(
    State(s): State<AppState>,
    Extension(read_context): Extension<RequestReadContext>,
    Path(id): Path<String>,
    Body(body): Body<CreateCollection>,
) -> ApiResult<Collection> {
    Ok(Json(
        blocking(move || {
            let _permit = read_permit(&s, domain::ReadClass::Index, &read_context)?;
            if let Some(scope) = body.scope {
                let scope = scope.into();
                query::validate_scope(&s, &id, &scope)?;
                s.store.save_scope_collection(&id, &body.name, &scope)
            } else {
                s.store.save_collection(&id, &body.name)
            }
        })
        .await?
        .into(),
    ))
}
#[utoipa::path(get,path="/v1/projects/{project_id}/jobs",params(("project_id"=String,Path)),responses((status=200,body=Jobs)))]
async fn list_jobs(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult<Jobs> {
    Ok(Json(Jobs {
        items: blocking(move || s.store.jobs(&id))
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    }))
}
#[utoipa::path(post,path="/v1/projects/{project_id}/jobs",params(("project_id"=String,Path)),request_body=SubmitJob,responses((status=200,body=Job)))]
async fn submit_job(
    State(s): State<AppState>,
    Extension(read_context): Extension<RequestReadContext>,
    Path(id): Path<String>,
    Body(body): Body<SubmitJob>,
) -> ApiResult<Job> {
    Ok(Json(
        blocking(move || {
            let _permit = read_permit(&s, domain::ReadClass::Index, &read_context)?;
            match (body.scope, body.selection_revision) {
                (Some(scope), None) => {
                    let scope: domain::ScopeRef = scope.into();
                    if let Some(job) = s.store.retry_scope_job(
                        &id,
                        &body.idempotency_key,
                        &scope,
                        body.delay_ms,
                    )? {
                        return Ok(job);
                    }
                    query::validate_scope(&s, &id, &scope)?;
                    let capture = if matches!(scope.target, domain::ScopeTarget::Source { .. }) {
                        Some(query::source_capture(&s, &id, &scope)?)
                    } else {
                        None
                    };
                    s.store.submit_scope_job(
                        &id,
                        &body.idempotency_key,
                        &scope,
                        body.delay_ms,
                        capture,
                    )
                }
                (None, Some(revision)) => {
                    s.store
                        .submit_job(&id, &body.idempotency_key, revision, body.delay_ms)
                }
                _ => Err(domain::Error::invalid(
                    "任务需要一个显式输入范围或旧选择版本",
                )),
            }
        })
        .await?
        .into(),
    ))
}
#[utoipa::path(post,path="/v1/projects/{project_id}/jobs/{job_id}/cancel",params(("project_id"=String,Path),("job_id"=String,Path)),responses((status=200,body=Job)))]
async fn cancel_job(
    State(s): State<AppState>,
    Path((pid, jid)): Path<(String, String)>,
) -> ApiResult<Job> {
    Ok(Json(
        blocking(move || {
            let job = s.store.job(&pid, &jid)?;
            let cancelled =
                s.store
                    .update_job(&pid, &jid, "cancelled", job.completed, None, None)?;
            if cancelled.status == "cancelled" {
                crate::jobs::cancel(&pid, &jid);
            }
            if cancelled.status == "cancelled"
                && let Some(result) = s.store.job_owned_result(&pid, &jid)?
            {
                s.store.cancel_result(&pid, &result)?;
                s.queries.cancel(&result);
            }
            Ok(cancelled)
        })
        .await?
        .into(),
    ))
}
#[utoipa::path(get,path="/v1/projects/{project_id}/jobs/{job_id}/artifact",params(("project_id"=String,Path),("job_id"=String,Path)),responses((status=200,description="Published NDJSON manifest",content_type="application/x-ndjson")))]
async fn artifact(
    State(s): State<AppState>,
    Path((pid, jid)): Path<(String, String)>,
) -> std::result::Result<Response, Failure> {
    let path = blocking(move || {
        let job = s.store.job(&pid, &jid)?;
        if job.status != "succeeded" || job.artifact.is_none() {
            return Err(domain::Error::new("NOT_FOUND", "任务尚未发布成果"));
        }
        let _lease = s.store.operation_lease(&pid)?;
        let item = crate::artifacts::verify(&s.store, &pid, &jid)?;
        if item.state != domain::ArtifactState::Ready {
            return Err(domain::Error::new(
                "ARTIFACT_NOT_READY",
                "成果已释放或尚不可读",
            ));
        }
        crate::artifacts::controlled_path(&s.store, &pid, &format!("artifacts/{}.jsonl", job.id))
    })
    .await?;
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|e| Failure(domain::Error::io(e)))?;
    let length = file
        .metadata()
        .await
        .map_err(|e| Failure(domain::Error::io(e)))?
        .len();
    let stream = tokio_util::io::ReaderStream::new(file);
    let mut response = (
        [
            (axum::http::header::CONTENT_TYPE, "application/x-ndjson"),
            (
                axum::http::header::CONTENT_DISPOSITION,
                "attachment; filename=dataset-manifest.jsonl",
            ),
        ],
        axum::body::Body::from_stream(stream),
    )
        .into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_LENGTH,
        HeaderValue::from(length),
    );
    Ok(response)
}
#[derive(Deserialize)]
struct EventQuery {
    #[serde(default)]
    after: Option<u64>,
}
#[utoipa::path(get,path="/v1/projects/{project_id}/events",params(("project_id"=String,Path),("after"=Option<u64>,Query)),responses((status=200,body=ProjectEvent,content_type="text/event-stream")))]
async fn events(
    State(s): State<AppState>,
    Path(pid): Path<String>,
    Query(q): Query<EventQuery>,
) -> std::result::Result<impl IntoResponse, Failure> {
    s.store.project(&pid)?;
    let initial = q.after.unwrap_or(s.store.latest_event(&pid)?);
    let shutdown = s.shutdown.subscribe();
    let stream = async_stream::stream! {let mut after=initial;
    if q.after.is_none(){let sync=ProjectEvent{sequence:after,project_id:pid.clone(),kind:"project.sync".into(),resource_id:pid.clone()};yield Ok::<_,Infallible>(Event::default().id(after.to_string()).event("project").data(serde_json::to_string(&sync).unwrap_or_default()));}
    loop{if *shutdown.borrow() || !s.store.view_is_open(&pid){break;}
        let store=s.store.clone();let id=pid.clone();let rows=tokio::task::spawn_blocking(move||store.events(&id,after)).await;
        match rows{Ok(Ok(rows))=>{for row in rows{after=row.sequence;let dto:ProjectEvent=row.into();yield Ok::<_,Infallible>(Event::default().id(after.to_string()).event("project").data(serde_json::to_string(&dto).unwrap_or_default()));}},_=>break}
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }};
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
#[utoipa::path(post,path="/v1/shutdown",responses((status=200,body=OkResponse)))]
async fn shutdown(State(s): State<AppState>) -> Json<OkResponse> {
    let _ = s.shutdown.send(true);
    Json(OkResponse { ok: true })
}
#[derive(OpenApi)]
#[openapi(
    paths(
        health,
        shutdown,
        projects,
        create_project,
        open_project,
        open_recent,
        close_project,
        project,
        sources,
        attach_source,
        assets,
        asset_detail,
        media,
        metadata,
        observations,
        raw_metadata,
        selection,
        change_selection,
        collections,
        create_collection,
        list_jobs,
        submit_job,
        cancel_job,
        artifact,
        events,
        query::fields,
        query::definitions,
        query::create_definition,
        query::definition,
        query::update_definition,
        query::build,
        query::results,
        query::run,
        query::result,
        query::validity,
        query::cancel,
        query::release,
        query::lease_result,
        query::release_result_lease,
        query::result_assets,
        query::capture,
        query::select_scope,
        source_locations::relink,
        tools::operators,
        tools::submit,
        tools::validate_scope,
        tools::run,
        tools::retry,
        tools::artifacts,
        tools::artifact,
        tools::verify,
        tools::rows,
        tools::release,
        tools::draft,
        tools::save_draft,
        tools::preference,
        tools::save_preference,
        resources::status,
        resources::configure,
        resources::configure_query,
        resources::configure_query_cache,
        resources::clear_query_cache,
        resources::clear,
        resources::cancel,
        settings::read,
        settings::configure,
        settings::clear,
        settings::entries,
        settings::retention,
        settings::release_entry,
        settings::heartbeat,
        settings::bases,
        settings::prebuild,
        settings::cancel_build,
        settings::fix_basis,
        settings::release_basis
    ),
    components(schemas(
        EngineConnection,
        ApiError,
        ProjectEvent,
        OkResponse,
        MetadataQuery,
        RawMetadataQuery
    )),
    info(title = "Dataset Studio Engine", version = "1.0.0")
)]
pub struct ApiDoc;
pub fn routes() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/v1/settings", get(settings::read))
        .route(
            "/v1/projects/{project_id}/query-results/{result_id}/cache-release",
            post(settings::release_entry),
        )
        .route(
            "/v1/settings/cache",
            axum::routing::put(settings::configure),
        )
        .route("/v1/settings/cache/clear", post(settings::clear))
        .route(
            "/v1/projects/{project_id}/cache-entries",
            get(settings::entries),
        )
        .route(
            "/v1/projects/{project_id}/query-results/{result_id}/retention",
            axum::routing::put(settings::retention),
        )
        .route(
            "/v1/projects/{project_id}/cache-session",
            post(settings::heartbeat),
        )
        .route("/v1/cache/rating-bases", get(settings::bases))
        .route(
            "/v1/projects/{project_id}/sources/{source_id}/rating-bases",
            post(settings::prebuild),
        )
        .route(
            "/v1/cache/rating-bases/{source_id}/cancel",
            post(settings::cancel_build),
        )
        .route(
            "/v1/cache/rating-bases/{source_id}/{rating}",
            axum::routing::put(settings::fix_basis),
        )
        .route(
            "/v1/cache/rating-bases/{source_id}/{rating}/release",
            post(settings::release_basis),
        )
        .route("/v1/resources", get(resources::status))
        .route(
            "/v1/resources/query-cache",
            axum::routing::put(resources::configure_query_cache),
        )
        .route(
            "/v1/resources/query-cache/clear",
            post(resources::clear_query_cache),
        )
        .route(
            "/v1/resources/query",
            axum::routing::put(resources::configure_query),
        )
        .route(
            "/v1/resources/cache",
            axum::routing::put(resources::configure),
        )
        .route("/v1/resources/cache/clear", post(resources::clear))
        .route(
            "/v1/projects/{pid}/read-requests/{rid}/cancel",
            post(resources::cancel),
        )
        .route("/v1/operators", get(tools::operators))
        .route(
            "/v1/preferences/{key}",
            get(tools::preference).put(tools::save_preference),
        )
        .route("/v1/projects/{pid}/tools/jobs", post(tools::submit))
        .route(
            "/v1/projects/{pid}/tools/validate-scope",
            post(tools::validate_scope),
        )
        .route("/v1/projects/{pid}/jobs/{jid}/run", get(tools::run))
        .route("/v1/projects/{pid}/jobs/{jid}/retry", post(tools::retry))
        .route("/v1/projects/{pid}/artifacts", get(tools::artifacts))
        .route("/v1/projects/{pid}/artifacts/{aid}", get(tools::artifact))
        .route("/v1/projects/{pid}/artifacts/{aid}/rows", get(tools::rows))
        .route(
            "/v1/projects/{pid}/artifacts/{aid}/verify",
            post(tools::verify),
        )
        .route(
            "/v1/projects/{pid}/artifacts/{aid}/release",
            post(tools::release),
        )
        .route(
            "/v1/projects/{pid}/drafts/{module}/{instance}",
            get(tools::draft).put(tools::save_draft),
        )
        .route("/v1/health", get(health))
        .route("/v1/shutdown", post(shutdown))
        .route("/v1/projects", get(projects).post(create_project))
        .route("/v1/projects/open", post(open_project))
        .route("/v1/projects/{id}", get(project))
        .route("/v1/projects/{id}/open", post(open_recent))
        .route("/v1/projects/{id}/close", post(close_project))
        .route(
            "/v1/projects/{id}/sources",
            get(sources).post(attach_source),
        )
        .route("/v1/projects/{id}/assets", get(assets))
        .route(
            "/v1/projects/{pid}/sources/{sid}/assets/{aid}",
            get(asset_detail),
        )
        .route(
            "/v1/projects/{pid}/sources/{sid}/assets/{aid}/media",
            get(media),
        )
        .route(
            "/v1/projects/{pid}/sources/{sid}/assets/{aid}/metadata",
            get(metadata),
        )
        .route(
            "/v1/projects/{pid}/sources/{sid}/assets/{aid}/records/{rid}/observations",
            get(observations),
        )
        .route(
            "/v1/projects/{pid}/sources/{sid}/assets/{aid}/records/{rid}/observations/{oid}/raw",
            get(raw_metadata),
        )
        .route(
            "/v1/projects/{id}/selection",
            get(selection).patch(change_selection),
        )
        .route(
            "/v1/projects/{id}/collections",
            get(collections).post(create_collection),
        )
        .route("/v1/projects/{id}/jobs", get(list_jobs).post(submit_job))
        .route("/v1/projects/{pid}/jobs/{jid}/cancel", post(cancel_job))
        .route("/v1/projects/{pid}/jobs/{jid}/artifact", get(artifact))
        .route("/v1/projects/{id}/events", get(events))
        .route(
            "/v1/projects/{pid}/sources/{sid}/fields",
            get(query::fields),
        )
        .route(
            "/v1/projects/{pid}/queries",
            get(query::definitions).post(query::create_definition),
        )
        .route(
            "/v1/projects/{pid}/queries/{qid}",
            get(query::definition).patch(query::update_definition),
        )
        .route(
            "/v1/projects/{pid}/queries/{qid}/results",
            post(query::build),
        )
        .route(
            "/v1/projects/{pid}/query-results",
            get(query::results).post(query::run),
        )
        .route("/v1/projects/{pid}/query-results/{rid}", get(query::result))
        .route(
            "/v1/projects/{pid}/query-results/{rid}/leases/{lid}",
            post(query::lease_result),
        )
        .route(
            "/v1/projects/{pid}/query-results/{rid}/leases/{lid}/release",
            post(query::release_result_lease),
        )
        .route(
            "/v1/projects/{pid}/query-results/{rid}/validity",
            get(query::validity),
        )
        .route(
            "/v1/projects/{pid}/query-results/{rid}/assets",
            get(query::result_assets),
        )
        .route(
            "/v1/projects/{pid}/query-results/{rid}/cancel",
            post(query::cancel),
        )
        .route(
            "/v1/projects/{pid}/query-results/{rid}/release",
            post(query::release),
        )
        .route("/v1/projects/{pid}/scopes/capture", post(query::capture))
        .route(
            "/v1/projects/{pid}/selection/scope",
            post(query::select_scope),
        )
        .route(
            "/v1/projects/{pid}/sources/{sid}/relink",
            post(source_locations::relink),
        )
}
