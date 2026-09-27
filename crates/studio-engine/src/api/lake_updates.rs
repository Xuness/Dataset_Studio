use super::*;
use domain::lake_updates::LakeUpdateOperation as Op;
use serde_json::json;
use utoipa::ToSchema;

async fn invoke<T>(s: AppState, op: Op, args: serde_json::Value) -> ApiResult<T>
where
    T: DeserializeOwned + Send + 'static,
{
    Ok(Json(
        blocking(move || {
            let value = s.lake_updates.execute(op, args)?;
            serde_json::from_value(value).map_err(|_| {
                domain::Error::new("UPDATE_PROTOCOL", "更新运行器返回了不兼容的数据结构")
            })
        })
        .await?,
    ))
}
#[utoipa::path(get,path="/v1/lake-updates/status",responses((status=200,body=LakeUpdateServiceStatus)),operation_id="lake_updates_status")]
pub(super) async fn status(State(s): State<AppState>) -> ApiResult<LakeUpdateServiceStatus> {
    if !s.lake_updates.configured() {
        return Ok(Json(LakeUpdateServiceStatus {
            configured: false,
            protocol_version: 1,
            worker_recent: false,
            credentials: vec![],
            activity: LakeUpdateActivity::default(),
            preparation_count: 0,
            preparation_attention_count: 0,
            preparations: vec![],
        }));
    }
    let (value, pending, attention, preparations) = blocking(move || {
        let value = s.lake_updates.execute(Op::Status, json!({}))?;
        let (pending, attention, preparations) = s.store.lake_input_activity()?;
        let rows = preparations
            .into_iter()
            .map(|r| serde_json::from_value(json!(r)).map_err(domain::Error::io))
            .collect::<domain::Result<Vec<LakeUpdatePreparation>>>()?;
        Ok((value, pending, attention, rows))
    })
    .await?;
    Ok(Json(LakeUpdateServiceStatus {
        configured: true,
        protocol_version: 1,
        worker_recent: value["worker_recent"].as_bool().unwrap_or(false),
        credentials: serde_json::from_value(value["credentials"].clone())
            .map_err(|_| Failure(domain::Error::new("UPDATE_PROTOCOL", "凭据状态格式不兼容")))?,
        activity: serde_json::from_value(value["activity"].clone())
            .map_err(|_| Failure(domain::Error::new("UPDATE_PROTOCOL", "更新活动格式不兼容")))?,
        preparation_count: pending,
        preparation_attention_count: attention,
        preparations,
    }))
}
#[utoipa::path(put,path="/v1/lake-updates/runtime",request_body=ConfigureLakeUpdates,responses((status=200,body=OkResponse)),operation_id="lake_updates_configure")]
pub(super) async fn configure(
    State(s): State<AppState>,
    Body(body): Body<ConfigureLakeUpdates>,
) -> ApiResult<OkResponse> {
    blocking(move || {
        s.lake_updates
            .configure(domain::lake_updates::LakeUpdateRuntime {
                python: body.python.into(),
                state_root: body.state_root.into(),
            })
    })
    .await?;
    Ok(Json(OkResponse { ok: true }))
}
#[utoipa::path(get,path="/v1/lake-updates/capabilities",responses((status=200,body=LakeUpdateCapabilities)),operation_id="lake_updates_capabilities")]
pub(super) async fn capabilities(State(s): State<AppState>) -> ApiResult<LakeUpdateCapabilities> {
    invoke(s, Op::Capabilities, json!({})).await
}
#[utoipa::path(get,path="/v1/lake-updates/pipeline",responses((status=200,body=LakePipelineSettings)),operation_id="lake_updates_pipeline")]
pub(super) async fn pipeline(State(s): State<AppState>) -> ApiResult<LakePipelineSettings> {
    invoke(s, Op::PipelineGet, json!({})).await
}
#[utoipa::path(put,path="/v1/lake-updates/pipeline",request_body=SaveLakePipelineSettings,responses((status=200,body=LakePipelineSettings)),operation_id="lake_updates_save_pipeline")]
pub(super) async fn save_pipeline(
    State(s): State<AppState>,
    Body(body): Body<SaveLakePipelineSettings>,
) -> ApiResult<LakePipelineSettings> {
    invoke(s, Op::PipelineSet, json!(body)).await
}
#[utoipa::path(get,path="/v1/lake-updates/lakes",responses((status=200,body=UpdateLakes)),operation_id="lake_updates_lakes")]
pub(super) async fn lakes(State(s): State<AppState>) -> ApiResult<UpdateLakes> {
    invoke(s, Op::Lakes, json!({})).await
}
#[utoipa::path(post,path="/v1/lake-updates/lakes",request_body=RegisterUpdateLake,responses((status=200,body=UpdateLake)),operation_id="lake_updates_register")]
pub(super) async fn register(
    State(s): State<AppState>,
    Body(body): Body<RegisterUpdateLake>,
) -> ApiResult<UpdateLake> {
    invoke(s, Op::Register, json!(body)).await
}
#[utoipa::path(put,path="/v1/lake-updates/credentials",request_body=SetLakeCredentials,responses((status=200,body=LakeCredentialStatus)),operation_id="lake_updates_credentials")]
pub(super) async fn credentials(
    State(s): State<AppState>,
    body: std::result::Result<Json<SetLakeCredentials>, JsonRejection>,
) -> ApiResult<LakeCredentialStatus> {
    let body = body
        .map_err(|_| Failure(domain::Error::invalid("凭据输入格式无效")))?
        .0;
    let mut value = json!(body);
    let site = value
        .as_object_mut()
        .and_then(|o| o.remove("site"))
        .ok_or_else(|| Failure(domain::Error::invalid("缺少站点")))?;
    invoke(s, Op::CredentialSet, json!({"site":site,"value":value})).await
}
#[utoipa::path(post,path="/v1/lake-updates/credentials/{site}/clear",params(("site"=String,Path)),responses((status=200,body=LakeCredentialStatus)),operation_id="lake_updates_clear_credentials")]
pub(super) async fn clear_credentials(
    State(s): State<AppState>,
    Path(site): Path<String>,
) -> ApiResult<LakeCredentialStatus> {
    invoke(s, Op::CredentialDelete, json!({"site":site})).await
}
#[utoipa::path(post,path="/v1/lake-updates/probes/{site}",params(("site"=String,Path)),responses((status=200,body=LakeApiProbe)),operation_id="lake_updates_probe")]
pub(super) async fn probe(
    State(s): State<AppState>,
    Path(site): Path<String>,
) -> ApiResult<LakeApiProbe> {
    invoke(s, Op::Probe, json!({"site":site})).await
}
#[utoipa::path(post,path="/v1/lake-updates/preview",request_body=LakeUpdateDefinition,responses((status=200,body=LakeUpdatePreview)),operation_id="lake_updates_preview")]
pub(super) async fn preview(
    State(s): State<AppState>,
    Body(body): Body<LakeUpdateDefinition>,
) -> ApiResult<LakeUpdatePreview> {
    invoke(s, Op::Preview, json!({"definition":body})).await
}
#[derive(Deserialize)]
pub(super) struct ListQuery {
    after: Option<String>,
    limit: Option<u32>,
    lake_id: Option<String>,
    status: Option<String>,
}
#[utoipa::path(get,path="/v1/lake-updates/jobs",params(("after"=Option<String>,Query),("limit"=Option<u32>,Query),("lake_id"=Option<String>,Query),("status"=Option<String>,Query)),responses((status=200,body=LakeUpdateJobs)),operation_id="lake_updates_jobs")]
pub(super) async fn jobs(
    State(s): State<AppState>,
    Query(q): Query<ListQuery>,
) -> ApiResult<LakeUpdateJobs> {
    invoke(
        s,
        Op::Jobs,
        json!({"after":q.after.unwrap_or_default(),"limit":q.limit.unwrap_or(50),"lake_id":q.lake_id,"status":q.status}),
    )
    .await
}
#[utoipa::path(post,path="/v1/lake-updates/jobs",request_body=CreateLakeUpdate,responses((status=200,body=LakeUpdateJob)),operation_id="lake_updates_create")]
pub(super) async fn create(
    State(s): State<AppState>,
    Body(body): Body<CreateLakeUpdate>,
) -> ApiResult<LakeUpdateJob> {
    invoke(s, Op::Create, json!(body)).await
}
#[utoipa::path(get,path="/v1/lake-updates/jobs/{id}",params(("id"=String,Path)),responses((status=200,body=LakeUpdateJob)),operation_id="lake_updates_job")]
pub(super) async fn job(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<LakeUpdateJob> {
    invoke(s, Op::Job, json!({"id":id})).await
}
#[utoipa::path(post,path="/v1/lake-updates/jobs/{id}/actions",params(("id"=String,Path)),request_body=LakeUpdateActionRequest,responses((status=200,body=LakeUpdateJob)),operation_id="lake_updates_action")]
pub(super) async fn action(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<LakeUpdateActionRequest>,
) -> ApiResult<LakeUpdateJob> {
    invoke(s, Op::Action, json!({"id":id,"action":body.action})).await
}
#[derive(Deserialize)]
pub(super) struct ItemQuery {
    after: Option<u64>,
    limit: Option<u32>,
    status: Option<String>,
    reason: Option<String>,
}
#[utoipa::path(get,path="/v1/lake-updates/jobs/{id}/items",params(("id"=String,Path),("after"=Option<u64>,Query),("limit"=Option<u32>,Query),("status"=Option<String>,Query),("reason"=Option<String>,Query)),responses((status=200,body=LakeUpdateItems)),operation_id="lake_updates_items")]
pub(super) async fn items(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<ItemQuery>,
) -> ApiResult<LakeUpdateItems> {
    invoke(
        s,
        Op::Items,
        json!({"id":id,"after":q.after.unwrap_or(0),"limit":q.limit.unwrap_or(100),"status":q.status,"reason":q.reason}),
    )
    .await
}
#[utoipa::path(get,path="/v1/lake-updates/jobs/{id}/coverage",params(("id"=String,Path)),responses((status=200,body=LakeUpdateCoverage)),operation_id="lake_updates_coverage")]
pub(super) async fn coverage(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<LakeUpdateCoverage> {
    invoke(s, Op::Coverage, json!({"id":id})).await
}
#[utoipa::path(get,path="/v1/lake-updates/schedules",responses((status=200,body=LakeUpdateSchedules)),operation_id="lake_updates_schedules")]
pub(super) async fn schedules(State(s): State<AppState>) -> ApiResult<LakeUpdateSchedules> {
    invoke(s, Op::Schedules, json!({})).await
}
#[utoipa::path(post,path="/v1/lake-updates/schedules",request_body=SaveLakeUpdateSchedule,responses((status=200,body=LakeUpdateScheduleSaved)),operation_id="lake_updates_schedule")]
pub(super) async fn schedule(
    State(s): State<AppState>,
    Body(body): Body<SaveLakeUpdateSchedule>,
) -> ApiResult<LakeUpdateScheduleSaved> {
    invoke(s, Op::ScheduleSet, json!(body)).await
}
#[derive(Deserialize, ToSchema)]
pub(super) struct DeleteSchedule {
    revision: u64,
}
#[utoipa::path(post,path="/v1/lake-updates/schedules/{id}/remove",params(("id"=String,Path)),request_body=DeleteSchedule,responses((status=200,body=OkResponse)),operation_id="lake_updates_remove_schedule")]
pub(super) async fn remove_schedule(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<DeleteSchedule>,
) -> ApiResult<OkResponse> {
    blocking(move || {
        s.lake_updates.execute(
            Op::ScheduleDelete,
            json!({"id":id,"revision":body.revision}),
        )
    })
    .await?;
    Ok(Json(OkResponse { ok: true }))
}
pub(super) fn routes() -> axum::Router<AppState> {
    axum::Router::new()
        .route(
            "/preparations",
            get(super::lake_inputs::list).post(super::lake_inputs::create),
        )
        .route(
            "/preparations/{id}/actions",
            post(super::lake_inputs::action),
        )
        .route("/inputs", post(create_input))
        .route("/inputs/{id}", get(input))
        .route("/inputs/{id}/append", post(append_input))
        .route("/inputs/{id}/seal", post(seal_input))
        .route("/status", get(status))
        .route("/runtime", axum::routing::put(configure))
        .route("/pipeline", get(pipeline).put(save_pipeline))
        .route("/capabilities", get(capabilities))
        .route("/lakes", get(lakes).post(register))
        .route("/credentials", axum::routing::put(credentials))
        .route("/credentials/{site}/clear", post(clear_credentials))
        .route("/probes/{site}", post(probe))
        .route("/preview", post(preview))
        .route("/jobs", get(jobs).post(create))
        .route("/jobs/{id}", get(job))
        .route("/jobs/{id}/actions", post(action))
        .route("/jobs/{id}/items", get(items))
        .route("/jobs/{id}/coverage", get(coverage))
        .route("/schedules", get(schedules).post(schedule))
        .route("/schedules/{id}/remove", post(remove_schedule))
}

#[utoipa::path(post,path="/v1/lake-updates/inputs",operation_id="lake_updates_create_input",request_body=CreateLakeUpdateInput,responses((status=200,body=LakeUpdateInput)))]
pub(super) async fn create_input(
    State(s): State<AppState>,
    Body(body): Body<CreateLakeUpdateInput>,
) -> ApiResult<LakeUpdateInput> {
    invoke(s, Op::InputCreate, json!(body)).await
}
#[utoipa::path(get,path="/v1/lake-updates/inputs/{id}",operation_id="lake_updates_input",params(("id"=String,Path)),responses((status=200,body=LakeUpdateInput)))]
pub(super) async fn input(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<LakeUpdateInput> {
    invoke(s, Op::Input, json!({"id":id})).await
}
#[utoipa::path(post,path="/v1/lake-updates/inputs/{id}/append",operation_id="lake_updates_append_input",params(("id"=String,Path)),request_body=AppendLakeUpdateInput,responses((status=200,body=LakeUpdateInput)))]
pub(super) async fn append_input(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<AppendLakeUpdateInput>,
) -> ApiResult<LakeUpdateInput> {
    invoke(
        s,
        Op::InputAppend,
        json!({"id":id,"post_ids":body.post_ids,"object_sha256s":body.object_sha256s}),
    )
    .await
}
#[utoipa::path(post,path="/v1/lake-updates/inputs/{id}/seal",operation_id="lake_updates_seal_input",params(("id"=String,Path)),responses((status=200,body=LakeUpdateInput)))]
pub(super) async fn seal_input(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<LakeUpdateInput> {
    invoke(s, Op::InputSeal, json!({"id":id})).await
}
