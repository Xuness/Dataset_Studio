use super::*;
use studio_application::aesthetic::AestheticRepository;
use studio_protocol::aesthetic::*;

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AestheticControl {
    pub action: String,
}
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AestheticRetry {
    pub acknowledge_possible_charge: bool,
}
#[derive(Deserialize, Default)]
pub struct Page {
    after: Option<String>,
    limit: Option<usize>,
    protected: Option<bool>,
    disposition: Option<domain::aesthetic::AestheticDisposition>,
}
#[derive(Serialize, utoipa::ToSchema)]
pub struct AestheticStages {
    items: Vec<AestheticStage>,
    next_cursor: Option<String>,
}
#[derive(Serialize, utoipa::ToSchema)]
pub struct AestheticBatches {
    items: Vec<AestheticBatch>,
    next_cursor: Option<String>,
}
#[derive(Serialize, utoipa::ToSchema)]
pub struct AestheticCandidates {
    items: Vec<AestheticCandidate>,
    next_cursor: Option<String>,
}
#[derive(Serialize, utoipa::ToSchema)]
pub struct AestheticAttempts {
    items: Vec<AestheticAttempt>,
}
#[derive(Serialize, utoipa::ToSchema)]
pub struct AestheticBackup {
    relative_path: String,
}

#[utoipa::path(operation_id="aesthetic_capabilities",get,path="/capabilities",params(("project_id"=String,Path)),responses((status=200,body=AestheticCapabilities)))]
async fn capabilities(
    State(s): State<AppState>,
    Path(pid): Path<String>,
) -> ApiResult<AestheticCapabilities> {
    blocking(move || s.store.project(&pid)).await?;
    Ok(Json(studio_application::aesthetic::capabilities().into()))
}
#[utoipa::path(operation_id="aesthetic_preflight",post,path="/preflight",params(("project_id"=String,Path)),request_body=AestheticCreate,responses((status=200,body=AestheticPreflight)))]
async fn preflight(
    State(s): State<AppState>,
    Path(pid): Path<String>,
    Body(value): Body<AestheticCreate>,
) -> ApiResult<AestheticPreflight> {
    Ok(Json(
        blocking(move || crate::aesthetic::preflight(&s, &pid, value.into()))
            .await?
            .into(),
    ))
}
#[utoipa::path(operation_id="aesthetic_decide_candidate",post,path="/stages/{id}/candidates/{ordinal}/disposition",params(("project_id"=String,Path),("id"=String,Path),("ordinal"=u64,Path)),request_body=AestheticCandidateDecision,responses((status=200,body=AestheticCandidate)))]
async fn decide_candidate(
    State(s): State<AppState>,
    Path((pid, id, ordinal)): Path<(String, String, u64)>,
    Body(value): Body<AestheticCandidateDecision>,
) -> ApiResult<AestheticCandidate> {
    Ok(Json(
        blocking(move || {
            let db = s.store.evaluation(&pid)?;
            let candidate = db.decide_candidate(&id, ordinal, value.into())?;
            s.store.sync_evaluation(&pid, &db.stage(&id)?)?;
            Ok(candidate)
        })
        .await?
        .into(),
    ))
}
#[utoipa::path(operation_id="aesthetic_abandon_creation",post,path="/creation-intents/{id}/abandon",params(("project_id"=String,Path),("id"=String,Path)),responses((status=200,body=OkResponse)))]
async fn abandon_creation(
    State(s): State<AppState>,
    Path((pid, id)): Path<(String, String)>,
) -> ApiResult<OkResponse> {
    blocking(move || s.store.abandon_evaluation_creation(&pid, &id)).await?;
    Ok(Json(OkResponse { ok: true }))
}

#[utoipa::path(operation_id="aesthetic_create",post,path="/stages",params(("project_id"=String,Path)),request_body=AestheticCreate,responses((status=200,body=AestheticStage)))]
async fn create(
    State(s): State<AppState>,
    Path(pid): Path<String>,
    Body(value): Body<AestheticCreate>,
) -> ApiResult<AestheticStage> {
    let copy = s.clone();
    let p = pid.clone();
    let stage = blocking(move || crate::aesthetic::create(&copy, &p, value.into())).await?;
    if stage.state == "preparing"
        && let Err(e) = s.aesthetic.launch(s.clone(), pid.clone(), stage.id.clone())
        && e.code != "REVISION_CONFLICT"
    {
        let copy = s.clone();
        let id = stage.id.clone();
        let message = e.to_string();
        blocking(move || {
            let db = copy.store.evaluation(&pid)?;
            let stage = db.settle(&id, Some(message))?;
            copy.store.sync_evaluation(&pid, &stage)
        })
        .await?;
        return Err(e.into());
    }
    Ok(Json(stage.into()))
}
#[utoipa::path(operation_id="aesthetic_stages",get,path="/stages",params(("project_id"=String,Path),("after"=Option<String>,Query),("limit"=Option<usize>,Query)),responses((status=200,body=AestheticStages)))]
async fn stages(
    State(s): State<AppState>,
    Path(pid): Path<String>,
    Query(page): Query<Page>,
) -> ApiResult<AestheticStages> {
    let limit = page.limit.unwrap_or(25).clamp(1, 50);
    let mut rows = blocking(move || {
        s.store
            .evaluation(&pid)?
            .stages(page.after.as_deref(), limit + 1)
    })
    .await?;
    let more = rows.len() > limit;
    rows.truncate(limit);
    Ok(Json(AestheticStages {
        next_cursor: if more {
            rows.last().map(|s| s.id.clone())
        } else {
            None
        },
        items: rows.into_iter().map(Into::into).collect(),
    }))
}
#[utoipa::path(operation_id="aesthetic_stage",get,path="/stages/{id}",params(("project_id"=String,Path),("id"=String,Path)),responses((status=200,body=AestheticStage)))]
async fn stage(
    State(s): State<AppState>,
    Path((pid, id)): Path<(String, String)>,
) -> ApiResult<AestheticStage> {
    Ok(Json(
        blocking(move || s.store.evaluation(&pid)?.stage(&id))
            .await?
            .into(),
    ))
}
#[utoipa::path(operation_id="aesthetic_control",post,path="/stages/{id}/control",params(("project_id"=String,Path),("id"=String,Path)),request_body=AestheticControl,responses((status=200,body=AestheticStage)))]
async fn control(
    State(s): State<AppState>,
    Path((pid, id)): Path<(String, String)>,
    Body(value): Body<AestheticControl>,
) -> ApiResult<AestheticStage> {
    let copy = s.clone();
    let p = pid.clone();
    let sid = id.clone();
    let action = value.action.clone();
    let stage = blocking(move || {
        let db = copy.store.evaluation(&p)?;
        if action == "start" {
            copy.aesthetic.check_start(&copy, &p)?;
        }
        let stage = if action == "parse" {
            db.parse_received(&sid)?;
            db.stage(&sid)?
        } else {
            db.control(&sid, &action)?
        };
        copy.store.sync_evaluation(&p, &stage)?;
        Ok(stage)
    })
    .await?;
    if value.action == "start" {
        if let Err(error) = s.aesthetic.launch(s.clone(), pid.clone(), id.clone()) {
            let copy = s.clone();
            let message = error.to_string();
            blocking(move || {
                let db = copy.store.evaluation(&pid)?;
                let stage = db.settle(&id, Some(message))?;
                copy.store.sync_evaluation(&pid, &stage)
            })
            .await?;
            return Err(error.into());
        }
    } else if value.action == "cancel" {
        s.aesthetic.cancel(&pid, &id);
    }
    Ok(Json(stage.into()))
}
fn ordinal(value: Option<&str>) -> domain::Result<Option<u64>> {
    value
        .map(|v| {
            v.parse::<u64>()
                .ok()
                .filter(|n| *n <= i64::MAX as u64)
                .ok_or_else(|| domain::Error::invalid("分页游标无效"))
        })
        .transpose()
}
#[utoipa::path(operation_id="aesthetic_batches",get,path="/stages/{id}/batches",params(("project_id"=String,Path),("id"=String,Path),("after"=Option<String>,Query),("limit"=Option<usize>,Query)),responses((status=200,body=AestheticBatches)))]
async fn batches(
    State(s): State<AppState>,
    Path((pid, id)): Path<(String, String)>,
    Query(page): Query<Page>,
) -> ApiResult<AestheticBatches> {
    let after = ordinal(page.after.as_deref())?.unwrap_or(0);
    let limit = page.limit.unwrap_or(20).clamp(1, 50);
    let mut rows =
        blocking(move || s.store.evaluation(&pid)?.batches(&id, after, limit + 1)).await?;
    let more = rows.len() > limit;
    rows.truncate(limit);
    Ok(Json(AestheticBatches {
        next_cursor: if more {
            rows.last().map(|v| v.sequence.to_string())
        } else {
            None
        },
        items: rows.into_iter().map(Into::into).collect(),
    }))
}
#[utoipa::path(operation_id="aesthetic_candidates",get,path="/stages/{id}/candidates",params(("project_id"=String,Path),("id"=String,Path),("after"=Option<String>,Query),("protected"=Option<bool>,Query),("disposition"=Option<String>,Query,description="active, needs_review, rejudge, or excluded; omitted returns all dispositions")),responses((status=200,body=AestheticCandidates)))]
async fn candidates(
    State(s): State<AppState>,
    Path((pid, id)): Path<(String, String)>,
    Query(page): Query<Page>,
) -> ApiResult<AestheticCandidates> {
    let after = ordinal(page.after.as_deref())?;
    let rows = blocking(move || {
        s.store.evaluation(&pid)?.filtered_candidates(
            &id,
            after,
            page.protected.unwrap_or(false),
            page.disposition,
        )
    })
    .await?;
    Ok(Json(AestheticCandidates {
        next_cursor: if rows.len() == 64 {
            rows.last().map(|v| v.ordinal.to_string())
        } else {
            None
        },
        items: rows.into_iter().map(Into::into).collect(),
    }))
}
#[utoipa::path(operation_id="aesthetic_candidate",get,path="/stages/{id}/candidates/{ordinal}",params(("project_id"=String,Path),("id"=String,Path),("ordinal"=u64,Path)),responses((status=200,body=AestheticCandidate)))]
async fn candidate(
    State(s): State<AppState>,
    Path((pid, id, ordinal)): Path<(String, String, u64)>,
) -> ApiResult<AestheticCandidate> {
    Ok(Json(
        blocking(move || s.store.evaluation(&pid)?.candidate(&id, ordinal))
            .await?
            .into(),
    ))
}
#[utoipa::path(operation_id="aesthetic_attempts",get,path="/stages/{id}/batches/{batch}/attempts",params(("project_id"=String,Path),("id"=String,Path),("batch"=u64,Path)),responses((status=200,body=AestheticAttempts)))]
async fn attempts(
    State(s): State<AppState>,
    Path((pid, id, batch)): Path<(String, String, u64)>,
) -> ApiResult<AestheticAttempts> {
    Ok(Json(AestheticAttempts {
        items: blocking(move || s.store.evaluation(&pid)?.attempts(&id, batch))
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    }))
}
#[utoipa::path(operation_id="aesthetic_retry",post,path="/stages/{id}/batches/{batch}/retry",params(("project_id"=String,Path),("id"=String,Path),("batch"=u64,Path)),request_body=AestheticRetry,responses((status=200,body=OkResponse)))]
async fn retry(
    State(s): State<AppState>,
    Path((pid, id, batch)): Path<(String, String, u64)>,
    Body(value): Body<AestheticRetry>,
) -> ApiResult<OkResponse> {
    if !value.acknowledge_possible_charge {
        return Err(domain::Error::invalid("重试可能新增费用，需要明确确认").into());
    }
    blocking(move || s.store.evaluation(&pid)?.retry_batch(&id, batch)).await?;
    Ok(Json(OkResponse { ok: true }))
}
#[utoipa::path(operation_id="aesthetic_metrics",get,path="/metrics",params(("project_id"=String,Path)),responses((status=200,body=AestheticMetrics)))]
async fn metrics(
    State(s): State<AppState>,
    Path(pid): Path<String>,
) -> ApiResult<AestheticMetrics> {
    let m = blocking(move || {
        let mut m = s.aesthetic.metrics(&pid, &s.store.directory(&pid)?)?;
        let (queued, peak) = s.store.evaluation(&pid)?.metrics()?;
        (
            m.queued_write_count,
            m.oldest_write_wait_ms,
            m.last_write_commit_ms,
            m.reserved_receipt_write_bytes,
        ) = s.store.evaluation(&pid)?.write_metrics()?;
        m.queued_write_bytes = queued;
        m.peak_write_bytes = peak;
        Ok(m)
    })
    .await?;
    Ok(Json(m.into()))
}
#[utoipa::path(operation_id="aesthetic_backup",post,path="/backup",params(("project_id"=String,Path)),responses((status=200,body=AestheticBackup)))]
async fn backup(State(s): State<AppState>, Path(pid): Path<String>) -> ApiResult<AestheticBackup> {
    let relative_path = blocking(move || {
        let dir = s.store.directory(&pid)?;
        let root = dir.join(".backups");
        std::fs::create_dir_all(&root).map_err(domain::Error::io)?;
        if !root
            .canonicalize()
            .map_err(domain::Error::io)?
            .starts_with(&dir)
        {
            return Err(domain::Error::invalid("备份目录必须在项目内"));
        }
        let relative = format!(
            ".backups/evaluation-{}-{}.sqlite",
            studio_storage::now(),
            domain::new_id()
        );
        s.store.evaluation(&pid)?.backup(&dir.join(&relative))?;
        Ok(relative)
    })
    .await?;
    Ok(Json(AestheticBackup { relative_path }))
}
#[derive(OpenApi)]
#[openapi(paths(
    create,
    stages,
    stage,
    control,
    batches,
    candidates,
    attempts,
    retry,
    metrics,
    backup,
    capabilities,
    preflight,
    decide_candidate,
    abandon_creation,
    candidate,
    reparse_batch,
    recovery_package
))]
pub struct AestheticApiDoc;
pub(super) fn routes() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/capabilities", get(capabilities))
        .route("/preflight", post(preflight))
        .route("/creation-intents/{id}/abandon", post(abandon_creation))
        .route("/stages", get(stages).post(create))
        .route("/stages/{id}", get(stage))
        .route("/stages/{id}/control", post(control))
        .route("/stages/{id}/batches", get(batches))
        .route("/stages/{id}/candidates", get(candidates))
        .route("/stages/{id}/candidates/{ordinal}", get(candidate))
        .route(
            "/stages/{id}/candidates/{ordinal}/disposition",
            post(decide_candidate),
        )
        .route("/stages/{id}/batches/{batch}/attempts", get(attempts))
        .route("/stages/{id}/batches/{batch}/retry", post(retry))
        .route("/stages/{id}/batches/{batch}/reparse", post(reparse_batch))
        .route("/metrics", get(metrics))
        .route("/backup", post(backup))
        .route("/recovery-package", post(recovery_package))
}
#[utoipa::path(operation_id="aesthetic_reparse_batch",post,path="/stages/{id}/batches/{batch}/reparse",params(("project_id"=String,Path),("id"=String,Path),("batch"=u64,Path)),responses((status=200,body=OkResponse)))]
async fn reparse_batch(
    State(s): State<AppState>,
    Path((pid, id, batch)): Path<(String, String, u64)>,
) -> ApiResult<OkResponse> {
    blocking(move || crate::aesthetic::reparse_batch(&s, &pid, &id, batch)).await?;
    Ok(Json(OkResponse { ok: true }))
}

#[utoipa::path(operation_id="aesthetic_recovery_package",post,path="/recovery-package",params(("project_id"=String,Path)),responses((status=200,body=AestheticBackup)))]
async fn recovery_package(
    State(s): State<AppState>,
    Path(pid): Path<String>,
) -> ApiResult<AestheticBackup> {
    let relative_path = blocking(move || s.store.recovery_package(&pid)).await?;
    Ok(Json(AestheticBackup { relative_path }))
}
