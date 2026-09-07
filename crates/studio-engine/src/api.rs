use axum::{
    Json,
    extract::{FromRequest, Path, Query, Request, State, rejection::JsonRejection},
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
use studio_application::{MetadataAdapter, ProjectRepository, SourceAdapter};
use studio_domain as domain;
use studio_protocol::*;
use studio_sources::SourceRouter;
use studio_storage::SqliteStore;
use utoipa::OpenApi;

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<SqliteStore>,
    pub connection: EngineConnection,
    pub io: Arc<tokio::sync::Semaphore>,
    pub metadata_io: Arc<tokio::sync::Semaphore>,
    pub metadata: Arc<studio_sources::MetadataReader>,
    pub shutdown: tokio::sync::watch::Sender<bool>,
}
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
            | "PROJECT_ID_CONFLICT"
            | "IDEMPOTENCY_CONFLICT" => StatusCode::CONFLICT,
            "SOURCE_BUSY" | "SOURCE_UNAVAILABLE" | "METADATA_RUNTIME_UNAVAILABLE" => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            "SOURCE_TIMEOUT" => StatusCode::GATEWAY_TIMEOUT,
            "SOURCE_RESOURCE_LIMIT" | "METADATA_LIMIT" => StatusCode::PAYLOAD_TOO_LARGE,
            "INVALID_INPUT"
            | "SOURCE_ID_MISMATCH"
            | "SOURCE_PATH_INVALID"
            | "FORMAT_UNSUPPORTED"
            | "METADATA_UNSUPPORTED"
            | "METADATA_RUNTIME_UNSUPPORTED"
            | "SOURCE_FORMAT_UNSUPPORTED" => StatusCode::BAD_REQUEST,
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
        blocking(move || s.store.open(body.directory.into()))
            .await?
            .into(),
    ))
}
#[utoipa::path(get,path="/v1/projects/{project_id}",params(("project_id"=String,Path)),responses((status=200,body=Project)))]
async fn project(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult<Project> {
    Ok(Json(blocking(move || s.store.project(&id)).await?.into()))
}

fn metadata_permit(
    s: &AppState,
) -> std::result::Result<tokio::sync::OwnedSemaphorePermit, Failure> {
    s.metadata_io.clone().try_acquire_owned().map_err(|_| {
        Failure(domain::Error::new(
            "SOURCE_BUSY",
            "另一个元数据请求正在读取，请稍后重试",
        ))
    })
}
#[utoipa::path(get,path="/v1/projects/{project_id}/sources/{source_id}/assets/{asset_id}/metadata",params(("project_id"=String,Path),("source_id"=String,Path),("asset_id"=String,Path),("cursor"=Option<String>,Query),("limit"=Option<usize>,Query),("version"=Option<String>,Query)),responses((status=200,body=MetadataOverview),(status=409,body=ApiError),(status=503,body=ApiError)))]
async fn metadata(
    State(s): State<AppState>,
    Path((pid, sid, aid)): Path<(String, String, String)>,
    Query(q): Query<MetadataQuery>,
) -> ApiResult<MetadataOverview> {
    let permit = metadata_permit(&s)?;
    Ok(Json(
        blocking(move || {
            let _permit = permit;
            let source = s.store.source(&pid, &sid)?;
            s.metadata.metadata(&source, &aid, q.into()).map(Into::into)
        })
        .await?,
    ))
}
#[utoipa::path(get,path="/v1/projects/{project_id}/sources/{source_id}/assets/{asset_id}/records/{record_id}/observations",params(("project_id"=String,Path),("source_id"=String,Path),("asset_id"=String,Path),("record_id"=String,Path),("cursor"=Option<String>,Query),("limit"=Option<usize>,Query),("version"=Option<String>,Query)),responses((status=200,body=ObservationPage),(status=409,body=ApiError),(status=503,body=ApiError)))]
async fn observations(
    State(s): State<AppState>,
    Path((pid, sid, aid, rid)): Path<(String, String, String, String)>,
    Query(q): Query<MetadataQuery>,
) -> ApiResult<ObservationPage> {
    let permit = metadata_permit(&s)?;
    Ok(Json(
        blocking(move || {
            let _permit = permit;
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
    Path((pid, sid, aid, rid, oid)): Path<(String, String, String, String, String)>,
    Query(q): Query<RawMetadataQuery>,
) -> ApiResult<RawMetadata> {
    let permit = metadata_permit(&s)?;
    Ok(Json(
        blocking(move || {
            let _permit = permit;
            let source = s.store.source(&pid, &sid)?;
            s.metadata
                .raw_metadata(&source, &aid, &rid, &oid, &q.version)
                .map(Into::into)
        })
        .await?,
    ))
}
#[utoipa::path(get,path="/v1/projects/{project_id}/sources",params(("project_id"=String,Path)),responses((status=200,body=Sources)))]
async fn sources(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult<Sources> {
    Ok(Json(Sources {
        items: blocking(move || {
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
}
fn encode_cursor(cursor: &Cursor) -> domain::Result<String> {
    Ok(URL_SAFE_NO_PAD.encode(serde_json::to_vec(cursor).map_err(domain::Error::io)?))
}
fn browse_sync(store: &SqliteStore, id: &str, query: BrowseQuery) -> domain::Result<AssetPage> {
    let limit = query.limit.unwrap_or(48).clamp(1, 128);
    let mut sources = store.sources(id)?;
    if let Some(source_id) = &query.source_id {
        sources.retain(|s| &s.id == source_id);
        if sources.is_empty() {
            return Err(domain::Error::new("NOT_FOUND", "数据源不在当前项目中"));
        }
    }
    let scope = hex::encode(Sha256::digest(
        serde_json::to_vec(&(id, &sources, &query.collection_id)).map_err(domain::Error::io)?,
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
    if let Some(collection) = query.collection_id {
        if !store.collections(id)?.iter().any(|c| c.id == collection) {
            return Err(domain::Error::new("NOT_FOUND", "工作集不存在"));
        }
        let mut keys =
            store.collection_keys(id, &collection, cursor.last_key.as_ref(), limit + 1)?;
        has_more = keys.len() > limit;
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
    })
}
#[utoipa::path(get,path="/v1/projects/{project_id}/assets",params(("project_id"=String,Path),("source_id"=Option<String>,Query),("collection_id"=Option<String>,Query),("cursor"=Option<String>,Query),("limit"=Option<usize>,Query)),responses((status=200,body=AssetPage)))]
async fn assets(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<BrowseQuery>,
) -> ApiResult<AssetPage> {
    let _permit =
        s.io.acquire()
            .await
            .map_err(|e| Failure(domain::Error::io(e)))?;
    Ok(Json(
        blocking(move || browse_sync(&s.store, &id, query)).await?,
    ))
}
#[derive(Deserialize)]
struct MediaQuery {
    edge: Option<u32>,
}
#[utoipa::path(get,path="/v1/projects/{project_id}/sources/{source_id}/assets/{asset_id}/media",params(("project_id"=String,Path),("source_id"=String,Path),("asset_id"=String,Path),("edge"=Option<u32>,Query)),responses((status=200,description="Authenticated image bytes",content_type="image/jpeg")))]
async fn media(
    State(s): State<AppState>,
    Path((pid, sid, aid)): Path<(String, String, String)>,
    Query(q): Query<MediaQuery>,
) -> std::result::Result<Response, Failure> {
    let _permit =
        s.io.acquire()
            .await
            .map_err(|e| Failure(domain::Error::io(e)))?;
    let result = blocking(move || {
        let source = s.store.source(&pid, &sid)?;
        let media = SourceRouter.read(&source, &aid)?;
        studio_sources::thumbnail(media, q.edge.unwrap_or(360))
    })
    .await?;
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, result.content_type),
            (
                axum::http::header::CACHE_CONTROL,
                "private, max-age=86400".into(),
            ),
        ],
        result.bytes,
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
    Path(id): Path<String>,
    Body(body): Body<CreateCollection>,
) -> ApiResult<Collection> {
    Ok(Json(
        blocking(move || s.store.save_collection(&id, &body.name))
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
    Path(id): Path<String>,
    Body(body): Body<SubmitJob>,
) -> ApiResult<Job> {
    Ok(Json(
        blocking(move || {
            s.store.submit_job(
                &id,
                &body.idempotency_key,
                body.selection_revision,
                body.delay_ms,
            )
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
            s.store
                .update_job(&pid, &jid, "cancelled", job.completed, None, None)
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
        Ok(s.store
            .directory(&pid)?
            .join("artifacts")
            .join(format!("{}.jsonl", job.id)))
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
    loop{if *shutdown.borrow(){break;}
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
        project,
        sources,
        attach_source,
        assets,
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
        events
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
        .route("/v1/health", get(health))
        .route("/v1/shutdown", post(shutdown))
        .route("/v1/projects", get(projects).post(create_project))
        .route("/v1/projects/open", post(open_project))
        .route("/v1/projects/{id}", get(project))
        .route(
            "/v1/projects/{id}/sources",
            get(sources).post(attach_source),
        )
        .route("/v1/projects/{id}/assets", get(assets))
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
}
