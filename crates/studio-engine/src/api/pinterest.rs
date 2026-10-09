use super::*;
use axum::routing::put;
use domain::pinterest::PinterestOperation as Op;
use serde_json::{Value, json};

#[utoipa::path(get,path="/v1/pinterest-collections/status",responses((status=200,body=PinterestStatus)),operation_id="pinterest_status")]
pub(super) async fn status(State(s): State<AppState>) -> ApiResult<PinterestStatus> {
    invoke(s, Op::Status, json!({})).await
}

async fn invoke<T: DeserializeOwned + Send + 'static>(
    s: AppState,
    op: Op,
    args: Value,
) -> ApiResult<T> {
    Ok(Json(
        blocking(move || {
            serde_json::from_value(s.lake_updates.execute_pinterest(op, args)?).map_err(|_| {
                domain::Error::new(
                    "COLLECTION_PROTOCOL",
                    "Pinterest 运行器返回了不兼容的数据结构",
                )
            })
        })
        .await?,
    ))
}
#[utoipa::path(get,path="/v1/pinterest-collections/capabilities",responses((status=200,body=PinterestCapabilities)),operation_id="pinterest_capabilities")]
pub(super) async fn capabilities(State(s): State<AppState>) -> ApiResult<PinterestCapabilities> {
    invoke(s, Op::Capabilities, json!({})).await
}
#[utoipa::path(get,path="/v1/pinterest-collections/lakes",params(CollectionPageQuery),responses((status=200,body=CollectionLakes)),operation_id="pinterest_lakes")]
pub(super) async fn lakes(
    State(s): State<AppState>,
    Query(q): Query<CollectionPageQuery>,
) -> ApiResult<CollectionLakes> {
    invoke(s, Op::Lakes, json!(q)).await
}
#[utoipa::path(post,path="/v1/pinterest-collections/lakes",request_body=CreateCollectionLake,responses((status=200,body=CollectionLake)),operation_id="pinterest_create_lake")]
pub(super) async fn create_lake(
    State(s): State<AppState>,
    Body(body): Body<CreateCollectionLake>,
) -> ApiResult<CollectionLake> {
    invoke(s, Op::LakeCreate, json!(body)).await
}
#[utoipa::path(post,path="/v1/pinterest-collections/lakes/register",request_body=CreateCollectionLake,responses((status=200,body=CollectionLake)),operation_id="pinterest_register_lake")]
pub(super) async fn register_lake(
    State(s): State<AppState>,
    Body(body): Body<CreateCollectionLake>,
) -> ApiResult<CollectionLake> {
    invoke(s, Op::LakeRegister, json!(body)).await
}
#[utoipa::path(post,path="/v1/pinterest-collections/jobs/preview",request_body=PinterestDefinition,responses((status=200,body=PinterestPreview)),operation_id="pinterest_preview")]
pub(super) async fn preview(
    State(s): State<AppState>,
    Body(body): Body<PinterestDefinition>,
) -> ApiResult<PinterestPreview> {
    invoke(s, Op::Preview, json!({"definition":body})).await
}
#[utoipa::path(post,path="/v1/pinterest-collections/jobs",request_body=CreatePinterestJob,responses((status=200,body=PinterestJob)),operation_id="pinterest_create")]
pub(super) async fn create(
    State(s): State<AppState>,
    Body(body): Body<CreatePinterestJob>,
) -> ApiResult<PinterestJob> {
    invoke(s, Op::Create, json!(body)).await
}
#[utoipa::path(get,path="/v1/pinterest-collections/jobs",params(CollectionJobsQuery),responses((status=200,body=PinterestJobs)),operation_id="pinterest_jobs")]
pub(super) async fn jobs(
    State(s): State<AppState>,
    Query(q): Query<CollectionJobsQuery>,
) -> ApiResult<PinterestJobs> {
    invoke(s, Op::Jobs, json!(q)).await
}
#[utoipa::path(get,path="/v1/pinterest-collections/jobs/{id}",params(("id"=String,Path)),responses((status=200,body=PinterestJob)),operation_id="pinterest_job")]
pub(super) async fn job(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<PinterestJob> {
    invoke(s, Op::Job, json!({"job_id":id})).await
}
#[utoipa::path(get,path="/v1/pinterest-collections/jobs/{id}/items",params(("id"=String,Path),PinterestItemsQuery),responses((status=200,body=PinterestItems)),operation_id="pinterest_items")]
pub(super) async fn items(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<PinterestItemsQuery>,
) -> ApiResult<PinterestItems> {
    let mut args = json!(q);
    args["job_id"] = json!(id);
    invoke(s, Op::Items, args).await
}
#[utoipa::path(get,path="/v1/pinterest-collections/jobs/{id}/streams",params(("id"=String,Path),PinterestItemsQuery),responses((status=200,body=PinterestStreams)),operation_id="pinterest_streams")]
pub(super) async fn streams(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<PinterestItemsQuery>,
) -> ApiResult<PinterestStreams> {
    let mut args = json!(q);
    args["job_id"] = json!(id);
    invoke(s, Op::Streams, args).await
}
#[utoipa::path(post,path="/v1/pinterest-collections/jobs/{id}/actions",params(("id"=String,Path)),request_body=PinterestJobAction,responses((status=200,body=PinterestJob)),operation_id="pinterest_action")]
pub(super) async fn action(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<PinterestJobAction>,
) -> ApiResult<PinterestJob> {
    let mut args = json!(body);
    args["job_id"] = json!(id);
    invoke(s, Op::Action, args).await
}
pub(super) fn routes() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/status", get(status))
        .route("/capabilities", get(capabilities))
        .route("/lakes", get(lakes).post(create_lake))
        .route("/lakes/register", post(register_lake))
        .route("/jobs/preview", post(preview))
        .route("/jobs", get(jobs).post(create))
        .route("/jobs/{id}", get(job))
        .route("/jobs/{id}/items", get(items))
        .route("/jobs/{id}/streams", get(streams))
        .route("/jobs/{id}/actions", post(action))
        .route("/schedules", get(schedules))
        .route("/schedules/{id}", put(save_schedule))
        .route("/schedules/{id}/remove", post(remove_schedule))
}

#[utoipa::path(get,path="/v1/pinterest-collections/schedules",params(CollectionSchedulesQuery),responses((status=200,body=PinterestSchedules)),operation_id="pinterest_schedules")]
pub(super) async fn schedules(
    State(s): State<AppState>,
    Query(q): Query<CollectionSchedulesQuery>,
) -> ApiResult<PinterestSchedules> {
    invoke(s, Op::Schedules, json!(q)).await
}
#[utoipa::path(put,path="/v1/pinterest-collections/schedules/{id}",params(("id"=String,Path)),request_body=SavePinterestSchedule,responses((status=200,body=PinterestSchedule)),operation_id="pinterest_save_schedule")]
pub(super) async fn save_schedule(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<SavePinterestSchedule>,
) -> ApiResult<PinterestSchedule> {
    if id != body.id {
        return Err(domain::Error::invalid("Pinterest schedule identity differs").into());
    }
    invoke(s, Op::ScheduleSave, json!(body)).await
}
#[utoipa::path(post,path="/v1/pinterest-collections/schedules/{id}/remove",params(("id"=String,Path)),request_body=CollectionRevisionCommand,responses((status=200,body=CollectionScheduleRemoved)),operation_id="pinterest_remove_schedule")]
pub(super) async fn remove_schedule(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<CollectionRevisionCommand>,
) -> ApiResult<CollectionScheduleRemoved> {
    let mut args = json!(body);
    args["id"] = json!(id);
    invoke(s, Op::ScheduleRemove, args).await
}
