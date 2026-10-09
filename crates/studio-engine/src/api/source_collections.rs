use super::*;
use domain::source_collections::CollectionOperation as Op;
use serde_json::{Value, json};

async fn invoke<T: DeserializeOwned + Send + 'static>(
    s: AppState,
    op: Op,
    args: Value,
) -> ApiResult<T> {
    Ok(Json(
        blocking(move || {
            serde_json::from_value(s.lake_updates.execute_collection(op, args)?).map_err(|_| {
                domain::Error::new("COLLECTION_PROTOCOL", "采集运行器返回了不兼容的数据结构")
            })
        })
        .await?,
    ))
}
fn args_with_id(id: String, value: impl Serialize) -> domain::Result<Value> {
    let mut value = serde_json::to_value(value).map_err(domain::Error::io)?;
    value["id"] = json!(id);
    Ok(value)
}
#[utoipa::path(get,path="/v1/source-collections/status",responses((status=200,body=CollectionServiceStatus)),operation_id="collections_status")]
pub(super) async fn status(State(s): State<AppState>) -> ApiResult<CollectionServiceStatus> {
    let configured = s.lake_updates.configured();
    let (value, runtime) = blocking(move || {
        let value = if configured {
            s.lake_updates
                .execute_collection(Op::Status, json!({}))
                .unwrap_or_else(|_| json!({}))
        } else {
            json!({})
        };
        let runtime =
            serde_json::from_value(json!(s.lake_updates.health())).map_err(domain::Error::io)?;
        Ok((value, runtime))
    })
    .await?;
    Ok(Json(CollectionServiceStatus {
        configured,
        protocol_version: 1,
        collection_contract_version: 2,
        runtime,
        counts: serde_json::from_value(value.get("counts").cloned().unwrap_or(json!({})))
            .map_err(domain::Error::io)?,
        active: serde_json::from_value(value.get("active").cloned().unwrap_or(json!([])))
            .map_err(domain::Error::io)?,
    }))
}
#[utoipa::path(get,path="/v1/source-collections/capabilities",responses((status=200,body=CollectionCapabilities)),operation_id="collections_capabilities")]
pub(super) async fn capabilities(State(s): State<AppState>) -> ApiResult<CollectionCapabilities> {
    invoke(s, Op::Capabilities, json!({})).await
}
#[utoipa::path(get,path="/v1/source-collections/lakes",params(CollectionPageQuery),responses((status=200,body=CollectionLakes)),operation_id="collections_lakes")]
pub(super) async fn lakes(
    State(s): State<AppState>,
    Query(q): Query<CollectionPageQuery>,
) -> ApiResult<CollectionLakes> {
    invoke(s, Op::Lakes, json!(q)).await
}
#[utoipa::path(post,path="/v1/source-collections/lakes",request_body=CreateCollectionLake,responses((status=200,body=CollectionLake)),operation_id="collections_create_lake")]
pub(super) async fn create_lake(
    State(s): State<AppState>,
    Body(body): Body<CreateCollectionLake>,
) -> ApiResult<CollectionLake> {
    invoke(s, Op::LakeCreate, json!(body)).await
}
#[utoipa::path(get,path="/v1/source-collections/accounts",params(CollectionPageQuery),responses((status=200,body=CollectionAccounts)),operation_id="collections_accounts")]
pub(super) async fn accounts(
    State(s): State<AppState>,
    Query(q): Query<CollectionPageQuery>,
) -> ApiResult<CollectionAccounts> {
    invoke(s, Op::Accounts, json!(q)).await
}
#[utoipa::path(post,path="/v1/source-collections/lakes/register",request_body=CreateCollectionLake,responses((status=200,body=CollectionLake)),operation_id="collections_register_lake")]
pub(super) async fn register_lake(
    State(s): State<AppState>,
    Body(body): Body<CreateCollectionLake>,
) -> ApiResult<CollectionLake> {
    invoke(s, Op::LakeRegister, json!(body)).await
}
#[utoipa::path(put,path="/v1/source-collections/accounts/{id}",params(("id"=String,Path)),request_body=SaveCollectionAccount,responses((status=200,body=CollectionAccount)),operation_id="collections_save_account")]
pub(super) async fn save_account(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<SaveCollectionAccount>,
) -> ApiResult<CollectionAccount> {
    if id != body.account_id {
        return Err(domain::Error::invalid("账号路径与请求身份不一致").into());
    }
    let value = json!(body);
    if serde_json::to_vec(&value).map_err(domain::Error::io)?.len() > 65536 {
        return Err(domain::Error::new("COLLECTION_LIMIT", "凭据请求超过 64 KiB").into());
    }
    invoke(s, Op::AccountSave, value).await
}
#[utoipa::path(post,path="/v1/source-collections/accounts/{id}/authenticate",params(("id"=String,Path)),request_body=SaveCollectionAccount,responses((status=200,body=CollectionAccountProbe)),operation_id="collections_authenticate_account")]
pub(super) async fn authenticate_account(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<SaveCollectionAccount>,
) -> ApiResult<CollectionAccountProbe> {
    if id != body.account_id || !matches!(body.mode, CollectionAccountMode::Session) {
        return Err(domain::Error::invalid("请使用相同身份的 Pixiv 登录会话").into());
    }
    let value = json!(body);
    if serde_json::to_vec(&value).map_err(domain::Error::io)?.len() > 65536 {
        return Err(domain::Error::new("COLLECTION_LIMIT", "凭据请求超过 64 KiB").into());
    }
    invoke(s, Op::AccountAuthenticate, value).await
}
#[utoipa::path(post,path="/v1/source-collections/accounts/{id}/probe",params(("id"=String,Path)),request_body=CollectionRevisionCommand,responses((status=200,body=CollectionAccountProbe)),operation_id="collections_probe_account")]
pub(super) async fn probe_account(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<CollectionRevisionCommand>,
) -> ApiResult<CollectionAccountProbe> {
    invoke(s, Op::AccountProbe, args_with_id(id, body)?).await
}
#[utoipa::path(post,path="/v1/source-collections/accounts/{id}/clear",params(("id"=String,Path)),request_body=CollectionRevisionCommand,responses((status=200,body=CollectionAccount)),operation_id="collections_clear_account")]
pub(super) async fn clear_account(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<CollectionRevisionCommand>,
) -> ApiResult<CollectionAccount> {
    invoke(s, Op::AccountClear, args_with_id(id, body)?).await
}
#[utoipa::path(post,path="/v1/source-collections/jobs/preview",request_body=CollectionJobDefinition,responses((status=200,body=CollectionPreview)),operation_id="collections_preview")]
pub(super) async fn preview(
    State(s): State<AppState>,
    Body(body): Body<CollectionJobDefinition>,
) -> ApiResult<CollectionPreview> {
    invoke(s, Op::Preview, json!({"definition":body})).await
}
#[utoipa::path(post,path="/v1/source-collections/jobs",request_body=CreateCollectionJob,responses((status=200,body=CollectionJobResult)),operation_id="collections_create_job")]
pub(super) async fn create(
    State(s): State<AppState>,
    Body(body): Body<CreateCollectionJob>,
) -> ApiResult<CollectionJobResult> {
    invoke(s, Op::Create, json!(body)).await
}
#[utoipa::path(get,path="/v1/source-collections/jobs",params(CollectionJobsQuery),responses((status=200,body=CollectionJobs)),operation_id="collections_jobs")]
pub(super) async fn jobs(
    State(s): State<AppState>,
    Query(q): Query<CollectionJobsQuery>,
) -> ApiResult<CollectionJobs> {
    invoke(s, Op::Jobs, json!(q)).await
}
#[utoipa::path(get,path="/v1/source-collections/jobs/{id}",params(("id"=String,Path)),responses((status=200,body=CollectionJob)),operation_id="collections_job")]
pub(super) async fn job(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<CollectionJob> {
    invoke(s, Op::Job, json!({"id":id})).await
}
#[utoipa::path(get,path="/v1/source-collections/jobs/{id}/tasks",params(("id"=String,Path),CollectionTasksQuery),responses((status=200,body=CollectionTasks)),operation_id="collections_tasks")]
pub(super) async fn tasks(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<CollectionTasksQuery>,
) -> ApiResult<CollectionTasks> {
    invoke(s, Op::Tasks, args_with_id(id, q)?).await
}
#[utoipa::path(get,path="/v1/source-collections/jobs/{id}/coverage",params(("id"=String,Path)),responses((status=200,body=CollectionCoverage)),operation_id="collections_coverage")]
pub(super) async fn coverage(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<CollectionCoverage> {
    invoke(s, Op::Coverage, json!({"id":id})).await
}
#[utoipa::path(post,path="/v1/source-collections/jobs/{id}/actions",params(("id"=String,Path)),request_body=CollectionJobAction,responses((status=200,body=CollectionJobResult)),operation_id="collections_action")]
pub(super) async fn action(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<CollectionJobAction>,
) -> ApiResult<CollectionJobResult> {
    invoke(s, Op::Action, args_with_id(id, body)?).await
}
#[utoipa::path(get,path="/v1/source-collections/pipeline",responses((status=200,body=CollectionPipelineSettings)),operation_id="collections_pipeline")]
pub(super) async fn pipeline(State(s): State<AppState>) -> ApiResult<CollectionPipelineSettings> {
    invoke(s, Op::PipelineGet, json!({})).await
}
#[utoipa::path(put,path="/v1/source-collections/pipeline",request_body=SaveCollectionPipeline,responses((status=200,body=CollectionPipelineSettings)),operation_id="collections_save_pipeline")]
pub(super) async fn save_pipeline(
    State(s): State<AppState>,
    Body(body): Body<SaveCollectionPipeline>,
) -> ApiResult<CollectionPipelineSettings> {
    invoke(s, Op::PipelineSet, json!(body)).await
}

#[utoipa::path(get,path="/v1/source-collections/schedules",params(CollectionSchedulesQuery),responses((status=200,body=CollectionSchedules)),operation_id="collections_schedules")]
pub(super) async fn schedules(
    State(s): State<AppState>,
    Query(q): Query<CollectionSchedulesQuery>,
) -> ApiResult<CollectionSchedules> {
    invoke(s, Op::Schedules, json!(q)).await
}
#[utoipa::path(put,path="/v1/source-collections/schedules/{id}",params(("id"=String,Path)),request_body=SaveCollectionSchedule,responses((status=200,body=CollectionSchedule)),operation_id="collections_save_schedule")]
pub(super) async fn save_schedule(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<SaveCollectionSchedule>,
) -> ApiResult<CollectionSchedule> {
    if id != body.id {
        return Err(domain::Error::invalid("计划路径与请求身份不一致").into());
    }
    invoke(s, Op::ScheduleSave, json!(body)).await
}
#[utoipa::path(post,path="/v1/source-collections/schedules/{id}/remove",params(("id"=String,Path)),request_body=CollectionRevisionCommand,responses((status=200,body=CollectionScheduleRemoved)),operation_id="collections_remove_schedule")]
pub(super) async fn remove_schedule(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<CollectionRevisionCommand>,
) -> ApiResult<CollectionScheduleRemoved> {
    invoke(s, Op::ScheduleRemove, args_with_id(id, body)?).await
}
#[utoipa::path(get,path="/v1/source-collections/workspace/lakes",params(LakeWorkspaceLakesQuery),responses((status=200,body=LakeWorkspaceLakes)),operation_id="lake_workspace_lakes")]
pub(super) async fn workspace_lakes(
    State(s): State<AppState>,
    Query(q): Query<LakeWorkspaceLakesQuery>,
) -> ApiResult<LakeWorkspaceLakes> {
    invoke(s, Op::WorkspaceLakes, json!(q)).await
}
#[utoipa::path(get,path="/v1/source-collections/workspace/jobs",params(LakeWorkspaceJobsQuery),responses((status=200,body=LakeWorkspaceJobs)),operation_id="lake_workspace_jobs")]
pub(super) async fn workspace_jobs(
    State(s): State<AppState>,
    Query(q): Query<LakeWorkspaceJobsQuery>,
) -> ApiResult<LakeWorkspaceJobs> {
    invoke(s, Op::WorkspaceJobs, json!(q)).await
}
#[utoipa::path(get,path="/v1/source-collections/workspace/schedules",params(LakeWorkspaceSchedulesQuery),responses((status=200,body=LakeWorkspaceSchedules)),operation_id="lake_workspace_schedules")]
pub(super) async fn workspace_schedules(
    State(s): State<AppState>,
    Query(q): Query<LakeWorkspaceSchedulesQuery>,
) -> ApiResult<LakeWorkspaceSchedules> {
    invoke(s, Op::WorkspaceSchedules, json!(q)).await
}

async fn relation<T: DeserializeOwned + Send + 'static>(
    s: AppState,
    read: RequestReadContext,
    path: (String, String, String),
    q: SourceRelationQuery,
    kind: domain::SourceRelationKind,
) -> ApiResult<T> {
    Ok(Json(
        blocking(move || {
            let source = s.store.source(&path.0, &path.1)?;
            let permit = read_permit(&s, domain::ReadClass::NativeQuery, &read)?;
            let value = permit.source_relation(
                &source,
                domain::SourceRelationRequest {
                    kind,
                    id: path.2,
                    version: q.version,
                    cursor: q.cursor,
                    limit: q.limit,
                    manifest_id: q.manifest_id,
                    recipe_id: q.recipe_id,
                },
            )?;
            serde_json::from_value(value)
                .map_err(|_| domain::Error::new("SOURCE_FORMAT_ERROR", "作品或作者关联格式不兼容"))
        })
        .await?,
    ))
}
#[utoipa::path(get,path="/v1/projects/{project_id}/sources/{source_id}/works/{id}",params(("project_id"=String,Path),("source_id"=String,Path),("id"=String,Path),SourceRelationQuery),responses((status=200,body=SourceWorkDetail)),operation_id="source_work")]
pub(super) async fn work(
    State(s): State<AppState>,
    Extension(read): Extension<RequestReadContext>,
    Path(path): Path<(String, String, String)>,
    Query(q): Query<SourceRelationQuery>,
) -> ApiResult<SourceWorkDetail> {
    relation(s, read, path, q, domain::SourceRelationKind::Work).await
}
#[utoipa::path(get,path="/v1/projects/{project_id}/sources/{source_id}/works/{id}/media",params(("project_id"=String,Path),("source_id"=String,Path),("id"=String,Path),SourceRelationQuery),responses((status=200,body=WorkMediaPage)),operation_id="source_work_media")]
pub(super) async fn work_media(
    State(s): State<AppState>,
    Extension(read): Extension<RequestReadContext>,
    Path(path): Path<(String, String, String)>,
    Query(q): Query<SourceRelationQuery>,
) -> ApiResult<WorkMediaPage> {
    relation(s, read, path, q, domain::SourceRelationKind::WorkMedia).await
}
#[utoipa::path(get,path="/v1/projects/{project_id}/sources/{source_id}/authors/{id}",params(("project_id"=String,Path),("source_id"=String,Path),("id"=String,Path),SourceRelationQuery),responses((status=200,body=SourceAuthorDetail)),operation_id="source_author")]
pub(super) async fn author(
    State(s): State<AppState>,
    Extension(read): Extension<RequestReadContext>,
    Path(path): Path<(String, String, String)>,
    Query(q): Query<SourceRelationQuery>,
) -> ApiResult<SourceAuthorDetail> {
    relation(s, read, path, q, domain::SourceRelationKind::Author).await
}
#[utoipa::path(get,path="/v1/projects/{project_id}/sources/{source_id}/authors/{id}/works",params(("project_id"=String,Path),("source_id"=String,Path),("id"=String,Path),SourceRelationQuery),responses((status=200,body=SourceAuthorWorks)),operation_id="source_author_works")]
pub(super) async fn author_works(
    State(s): State<AppState>,
    Extension(read): Extension<RequestReadContext>,
    Path(path): Path<(String, String, String)>,
    Query(q): Query<SourceRelationQuery>,
) -> ApiResult<SourceAuthorWorks> {
    relation(s, read, path, q, domain::SourceRelationKind::AuthorWorks).await
}

pub(super) fn routes() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/status", get(status))
        .route("/capabilities", get(capabilities))
        .route("/lakes", get(lakes).post(create_lake))
        .route("/lakes/register", post(register_lake))
        .route("/accounts", get(accounts))
        .route("/accounts/{id}", axum::routing::put(save_account))
        .route("/accounts/{id}/authenticate", post(authenticate_account))
        .route("/accounts/{id}/probe", post(probe_account))
        .route("/accounts/{id}/clear", post(clear_account))
        .route("/jobs/preview", post(preview))
        .route("/jobs", get(jobs).post(create))
        .route("/jobs/{id}", get(job))
        .route("/jobs/{id}/tasks", get(tasks))
        .route("/jobs/{id}/coverage", get(coverage))
        .route("/jobs/{id}/actions", post(action))
        .route("/pipeline", get(pipeline).put(save_pipeline))
        .route("/schedules", get(schedules))
        .route("/schedules/{id}", axum::routing::put(save_schedule))
        .route("/schedules/{id}/remove", post(remove_schedule))
        .route("/workspace/lakes", get(workspace_lakes))
        .route("/workspace/jobs", get(workspace_jobs))
        .route("/workspace/schedules", get(workspace_schedules))
}
pub(super) fn source_routes() -> axum::Router<AppState> {
    axum::Router::new()
        .route(
            "/v1/projects/{project_id}/sources/{source_id}/works/{id}",
            get(work),
        )
        .route(
            "/v1/projects/{project_id}/sources/{source_id}/works/{id}/media",
            get(work_media),
        )
        .route(
            "/v1/projects/{project_id}/sources/{source_id}/authors/{id}",
            get(author),
        )
        .route(
            "/v1/projects/{project_id}/sources/{source_id}/authors/{id}/works",
            get(author_works),
        )
}
