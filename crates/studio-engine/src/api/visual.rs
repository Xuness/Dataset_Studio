use super::*;
use studio_protocol::visual::*;

fn wire<T: DeserializeOwned>(value: impl Serialize) -> domain::Result<T> {
    serde_json::from_value(serde_json::to_value(value).map_err(domain::Error::io)?)
        .map_err(domain::Error::io)
}
#[utoipa::path(get,path="/runtime",responses((status=200,body=VisualRuntimeStatus)),operation_id="visual_runtime")]
async fn runtime(State(s): State<AppState>) -> ApiResult<VisualRuntimeStatus> {
    Ok(Json(wire(s.visual.runtime()?)?))
}
#[utoipa::path(put,path="/runtime",request_body=VisualRuntimeConfig,responses((status=200,body=VisualRuntimeStatus)),operation_id="visual_configure")]
async fn configure(
    State(s): State<AppState>,
    Body(value): Body<VisualRuntimeConfig>,
) -> ApiResult<VisualRuntimeStatus> {
    Ok(Json(wire(
        blocking(move || s.visual.configure(wire(value)?)).await?,
    )?))
}
#[utoipa::path(get,path="/models",responses((status=200,body=VisualModelDescription)),operation_id="visual_models")]
async fn models(State(s): State<AppState>) -> ApiResult<VisualModelDescription> {
    Ok(Json(wire(blocking(move || s.visual.models()).await?)?))
}
#[utoipa::path(get,path="/recipes",responses((status=200,body=VisualRecipes)),operation_id="visual_recipes")]
async fn recipes(State(s): State<AppState>) -> ApiResult<VisualRecipes> {
    Ok(Json(VisualRecipes {
        items: wire(blocking(move || s.visual.recipes()).await?)?,
    }))
}
#[utoipa::path(get,path="/sources",responses((status=200,body=VisualSources)),operation_id="visual_sources")]
async fn sources(State(s): State<AppState>) -> ApiResult<VisualSources> {
    Ok(Json(VisualSources {
        items: wire(blocking(move || s.visual.sources()).await?)?,
    }))
}
#[derive(Deserialize)]
struct Page {
    after: Option<u64>,
    limit: Option<u32>,
}
#[utoipa::path(get,path="/runs",params(("after"=Option<u64>,Query),("limit"=Option<u32>,Query)),responses((status=200,body=VisualRuns)),operation_id="visual_runs")]
async fn runs(State(s): State<AppState>, Query(page): Query<Page>) -> ApiResult<VisualRuns> {
    let limit = page.limit.unwrap_or(50).clamp(1, 99);
    let mut items = blocking(move || s.visual.runs(page.after.unwrap_or(0), limit + 1)).await?;
    let more = items.len() > limit as usize;
    items.truncate(limit as usize);
    let next_cursor = more.then(|| items.last().unwrap().ordinal);
    Ok(Json(VisualRuns {
        items: wire(items)?,
        next_cursor,
    }))
}
#[utoipa::path(post,path="/runs",request_body=VisualRunRequest,responses((status=200,body=VisualRun)),operation_id="visual_create")]
async fn create(
    State(s): State<AppState>,
    Body(value): Body<VisualRunRequest>,
) -> ApiResult<VisualRun> {
    Ok(Json(wire(
        blocking(move || s.visual.create(wire(value)?)).await?,
    )?))
}
#[utoipa::path(get,path="/runs/{id}",params(("id"=String,Path)),responses((status=200,body=VisualRun)),operation_id="visual_run")]
async fn run(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult<VisualRun> {
    Ok(Json(wire(blocking(move || s.visual.run(&id)).await?)?))
}
#[utoipa::path(post,path="/runs/{id}/actions",params(("id"=String,Path)),request_body=VisualAction,responses((status=200,body=VisualRun)),operation_id="visual_action")]
async fn action(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Body(value): Body<VisualAction>,
) -> ApiResult<VisualRun> {
    Ok(Json(wire(
        blocking(move || s.visual.action(&id, &value.action)).await?,
    )?))
}
#[utoipa::path(get,path="/coverage",responses((status=200,body=VisualCoverageResponse)),operation_id="visual_coverage")]
async fn coverage(State(s): State<AppState>) -> ApiResult<VisualCoverageResponse> {
    Ok(Json(VisualCoverageResponse {
        items: wire(blocking(move || s.visual.coverage()).await?)?,
    }))
}
#[derive(Deserialize)]
struct FailurePage {
    after: Option<i64>,
    limit: Option<u32>,
}
#[utoipa::path(get,path="/runs/{id}/failures",params(("id"=String,Path),("after"=Option<i64>,Query),("limit"=Option<u32>,Query)),responses((status=200,body=VisualFailures)),operation_id="visual_failures")]
async fn failures(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Query(page): Query<FailurePage>,
) -> ApiResult<VisualFailures> {
    let items = blocking(move || {
        s.visual
            .failures(&id, page.after.unwrap_or(0), page.limit.unwrap_or(50))
    })
    .await?;
    let next_cursor = items.last().map(|r| r.object_id);
    Ok(Json(VisualFailures {
        items: wire(items)?,
        next_cursor,
    }))
}
#[utoipa::path(post,path="/search",request_body=VisualSearchRequest,responses((status=200,body=VisualSearchResult)),operation_id="visual_search")]
async fn search(
    State(s): State<AppState>,
    Body(value): Body<VisualSearchRequest>,
) -> ApiResult<VisualSearchResult> {
    Ok(Json(wire(
        blocking(move || s.visual.search(wire(value)?)).await?,
    )?))
}
#[utoipa::path(post,path="/duplicates",request_body=VisualDuplicatesRequest,responses((status=200,body=VisualDuplicates)),operation_id="visual_duplicates")]
async fn duplicates(
    State(s): State<AppState>,
    Body(value): Body<VisualDuplicatesRequest>,
) -> ApiResult<VisualDuplicates> {
    let kind = if value.asset.is_some() {
        "phash_candidates"
    } else {
        "sha256_groups"
    }
    .to_owned();
    let recipe_id = if value.asset.is_some() {
        domain::visual::HASH_RECIPE
    } else {
        "sha256"
    }
    .to_owned();
    let (items, next_cursor) = blocking(move || {
        let key: Option<domain::AssetKey> = wire(value.asset)?;
        s.visual.duplicates(
            key.as_ref(),
            value.after.unwrap_or(0),
            value.limit,
            value.radius.unwrap_or(6),
        )
    })
    .await?;
    Ok(Json(VisualDuplicates {
        kind,
        recipe_id,
        items: wire(items)?,
        next_cursor,
    }))
}
#[utoipa::path(post,path="/indexes",request_body=VisualIndexRequest,responses((status=200,body=VisualIndexJob)),operation_id="visual_build_index")]
async fn index(
    State(s): State<AppState>,
    Body(value): Body<VisualIndexRequest>,
) -> ApiResult<VisualIndexJob> {
    Ok(Json(wire(
        blocking(move || s.visual.create_index(wire(value)?)).await?,
    )?))
}
#[utoipa::path(get,path="/indexes",params(("after"=Option<u64>,Query),("limit"=Option<u32>,Query)),responses((status=200,body=VisualIndexJobs)),operation_id="visual_index_jobs")]
async fn index_jobs(
    State(s): State<AppState>,
    Query(page): Query<Page>,
) -> ApiResult<VisualIndexJobs> {
    let limit = page.limit.unwrap_or(50).clamp(1, 100);
    let mut items =
        blocking(move || s.visual.index_jobs(page.after.unwrap_or(0), limit + 1)).await?;
    let more = items.len() > limit as usize;
    items.truncate(limit as usize);
    let next_cursor = if more {
        items.last().map(|j| j.ordinal)
    } else {
        None
    };
    Ok(Json(VisualIndexJobs {
        items: wire(items)?,
        next_cursor,
    }))
}
#[utoipa::path(get,path="/indexes/{id}",params(("id"=String,Path)),responses((status=200,body=VisualIndexJob)),operation_id="visual_index_job")]
async fn index_job(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult<VisualIndexJob> {
    Ok(Json(wire(
        blocking(move || s.visual.index_job(&id)).await?,
    )?))
}
#[utoipa::path(post,path="/indexes/{id}/actions",params(("id"=String,Path)),request_body=VisualAction,responses((status=200,body=VisualIndexJob)),operation_id="visual_index_action")]
async fn index_action(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Body(value): Body<VisualAction>,
) -> ApiResult<VisualIndexJob> {
    Ok(Json(wire(
        blocking(move || s.visual.index_action(&id, &value.action)).await?,
    )?))
}
#[utoipa::path(get,path="/discovery",responses((status=200,body=VisualDiscoveryCursors)),operation_id="visual_discovery")]
async fn discovery(State(s): State<AppState>) -> ApiResult<VisualDiscoveryCursors> {
    Ok(Json(VisualDiscoveryCursors {
        items: wire(blocking(move || s.visual.discovery_cursors()).await?)?,
    }))
}
#[utoipa::path(get,path="/retention",responses((status=200,body=VisualRetentionPolicy)),operation_id="visual_retention")]
async fn retention(State(s): State<AppState>) -> ApiResult<VisualRetentionPolicy> {
    Ok(Json(wire(
        blocking(move || s.visual.retention_policy()).await?,
    )?))
}
#[utoipa::path(put,path="/retention",request_body=VisualRetentionPolicy,responses((status=200,body=VisualRetentionPolicy)),operation_id="visual_set_retention")]
async fn set_retention(
    State(s): State<AppState>,
    Body(value): Body<VisualRetentionPolicy>,
) -> ApiResult<VisualRetentionPolicy> {
    Ok(Json(wire(
        blocking(move || s.visual.set_retention_policy(wire(value)?)).await?,
    )?))
}
#[utoipa::path(post,path="/retention/cleanup",request_body=VisualCleanupRequest,responses((status=200,body=VisualCleanupResults)),operation_id="visual_cleanup")]
async fn cleanup(
    State(s): State<AppState>,
    Body(value): Body<VisualCleanupRequest>,
) -> ApiResult<VisualCleanupResults> {
    Ok(Json(wire(
        blocking(move || s.visual.cleanup(value.apply)).await?,
    )?))
}
#[utoipa::path(get,path="/runs/{id}/metrics",params(("id"=String,Path)),responses((status=200,body=VisualMetrics)),operation_id="visual_metrics")]
async fn metrics(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult<VisualMetrics> {
    Ok(Json(wire(blocking(move || s.visual.metrics(&id)).await?)?))
}
#[derive(Deserialize)]
struct SnapshotPage {
    after: Option<String>,
    limit: Option<u32>,
}
#[utoipa::path(get,path="/snapshots",params(("after"=Option<String>,Query),("limit"=Option<u32>,Query)),responses((status=200,body=VisualSnapshots)),operation_id="visual_snapshots")]
async fn snapshots(
    State(s): State<AppState>,
    Query(page): Query<SnapshotPage>,
) -> ApiResult<VisualSnapshots> {
    let limit = page.limit.unwrap_or(50).clamp(1, 100);
    let mut items = blocking(move || {
        s.visual
            .snapshots(page.after.as_deref().unwrap_or(""), limit + 1)
    })
    .await?;
    let more = items.len() > limit as usize;
    items.truncate(limit as usize);
    let next_cursor = if more {
        items.last().map(|s| s.id.clone())
    } else {
        None
    };
    Ok(Json(VisualSnapshots {
        items: wire(items)?,
        next_cursor,
    }))
}
#[utoipa::path(get,path="/snapshots/{id}",params(("id"=String,Path)),responses((status=200,body=VisualSnapshot)),operation_id="visual_snapshot")]
async fn snapshot(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult<VisualSnapshot> {
    Ok(Json(wire(blocking(move || s.visual.snapshot(&id)).await?)?))
}
#[utoipa::path(post,path="/snapshots",request_body=VisualSnapshotRequest,responses((status=200,body=VisualSnapshot)),operation_id="visual_create_snapshot")]
async fn create_snapshot(
    State(s): State<AppState>,
    Body(value): Body<VisualSnapshotRequest>,
) -> ApiResult<VisualSnapshot> {
    Ok(Json(wire(
        blocking(move || s.visual.create_snapshot(&value.name, &value.recipe_ids)).await?,
    )?))
}
#[utoipa::path(post,path="/features",request_body=VisualFeaturePageRequest,responses((status=200,body=VisualFeaturePage)),operation_id="visual_features")]
async fn features(
    State(s): State<AppState>,
    Body(value): Body<VisualFeaturePageRequest>,
) -> ApiResult<VisualFeaturePage> {
    Ok(Json(wire(
        blocking(move || s.visual.feature_page(wire(value)?, false)).await?,
    )?))
}
#[utoipa::path(post,path="/exports",request_body=VisualFeaturePageRequest,responses((status=200,body=VisualExportPage)),operation_id="visual_export")]
async fn export(
    State(s): State<AppState>,
    Body(value): Body<VisualFeaturePageRequest>,
) -> ApiResult<VisualExportPage> {
    Ok(Json(wire(
        blocking(move || s.visual.feature_page(wire(value)?, true)).await?,
    )?))
}
#[derive(OpenApi)]
#[openapi(paths(
    runtime,
    configure,
    models,
    recipes,
    sources,
    runs,
    create,
    run,
    action,
    coverage,
    failures,
    search,
    duplicates,
    index,
    index_jobs,
    index_status,
    index_job,
    index_action,
    discovery,
    retention,
    set_retention,
    cleanup,
    metrics,
    snapshots,
    snapshot,
    create_snapshot,
    features,
    export
))]
pub struct VisualApiDoc;
#[utoipa::path(get,path="/index-status",responses((status=200,body=VisualIndexStates)),operation_id="visual_index_status")]
async fn index_status(State(s): State<AppState>) -> ApiResult<VisualIndexStates> {
    Ok(Json(wire(
        blocking(move || s.visual.index_status()).await?,
    )?))
}
pub(super) fn routes() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/runtime", get(runtime).put(configure))
        .route("/models", get(models))
        .route("/recipes", get(recipes))
        .route("/sources", get(sources))
        .route("/runs", get(runs).post(create))
        .route("/runs/{id}", get(run))
        .route("/runs/{id}/actions", post(action))
        .route("/runs/{id}/failures", get(failures))
        .route("/runs/{id}/metrics", get(metrics))
        .route("/snapshots", get(snapshots).post(create_snapshot))
        .route("/snapshots/{id}", get(snapshot))
        .route("/features", post(features))
        .route("/exports", post(export))
        .route("/coverage", get(coverage))
        .route("/search", post(search))
        .route("/duplicates", post(duplicates))
        .route("/indexes", get(index_jobs).post(index))
        .route("/index-status", get(index_status))
        .route("/indexes/{id}", get(index_job))
        .route("/indexes/{id}/actions", post(index_action))
        .route("/discovery", get(discovery))
        .route("/retention", get(retention).put(set_retention))
        .route("/retention/cleanup", post(cleanup))
}
