use super::*;
use studio_domain::aesthetic_analysis as analysis;
use studio_protocol::aesthetic_analysis::*;

fn wire<T: serde::de::DeserializeOwned>(value: impl Serialize) -> domain::Result<T> {
    serde_json::from_value(serde_json::to_value(value).map_err(domain::Error::io)?)
        .map_err(domain::Error::io)
}
#[derive(Deserialize, Default)]
struct Page {
    after: Option<String>,
    limit: Option<usize>,
    rating: Option<String>,
    experiment_id: Option<String>,
    ordinal: Option<u64>,
}
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AestheticAnalysisControl {
    action: String,
}
#[derive(Serialize, utoipa::ToSchema)]
pub struct AestheticAnalysisJobs {
    items: Vec<AestheticAnalysisJob>,
    next_cursor: Option<String>,
}
#[derive(Serialize, utoipa::ToSchema)]
pub struct AestheticRankingRows {
    items: Vec<AestheticRankingRow>,
    next_cursor: Option<String>,
}
#[derive(Serialize, utoipa::ToSchema)]
pub struct AestheticComparisonRows {
    items: Vec<AestheticComparisonRow>,
    next_cursor: Option<String>,
}
#[derive(Serialize, utoipa::ToSchema)]
pub struct AestheticExperiments {
    items: Vec<AestheticExperiment>,
    next_cursor: Option<String>,
}
#[derive(Serialize, utoipa::ToSchema)]
pub struct AestheticReviews {
    items: Vec<AestheticReview>,
    next_cursor: Option<String>,
}
fn ordinal(after: Option<&str>) -> domain::Result<u64> {
    after
        .unwrap_or("0")
        .parse::<u64>()
        .ok()
        .filter(|n| *n <= i64::MAX as u64)
        .ok_or_else(|| domain::Error::invalid("分页游标无效"))
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectionCursor {
    signature: String,
    after: u64,
    review_watermark: u64,
}

#[utoipa::path(operation_id="aesthetic_ranking_select",post,path="/snapshots/{id}/select",params(("project_id"=String,Path),("id"=String,Path)),request_body=AestheticRankingQuery,responses((status=200,body=AestheticRankingSelection)))]
async fn select(
    State(s): State<AppState>,
    Path((pid, id)): Path<(String, String)>,
    Body(value): Body<AestheticRankingQuery>,
) -> ApiResult<AestheticRankingSelection> {
    let value: analysis::AestheticRankingQuery = wire(value)?;
    let result = blocking(move || {
        studio_application::aesthetic_analysis::validate_filter(&value.filter)?;
        let db = s.store.evaluation(&pid)?;
        let snapshot = db.ranking_snapshot(&id)?;
        let signature = hex::encode(Sha256::digest(
            serde_json::to_vec(&(&id, &value.filter)).map_err(domain::Error::io)?,
        ));
        let watermark = db.review_watermark()?;
        let mut cursor = if let Some(after) = value.after {
            if after.len() > 2048 {
                return Err(domain::Error::invalid("筛选游标过长"));
            }
            let bytes = URL_SAFE_NO_PAD
                .decode(after)
                .map_err(|_| domain::Error::invalid("筛选游标无效"))?;
            let cursor: SelectionCursor = serde_json::from_slice(&bytes)
                .map_err(|_| domain::Error::invalid("筛选游标无效"))?;
            if cursor.signature != signature
                || cursor.after > snapshot.input.candidates
                || cursor.review_watermark > watermark
            {
                return Err(domain::Error::invalid("筛选快照或条件与游标不一致"));
            }
            cursor
        } else {
            SelectionCursor {
                signature,
                after: 0,
                review_watermark: watermark,
            }
        };
        let limit = value.limit.unwrap_or(64).clamp(1, 128) as usize;
        let mut items = Vec::new();
        let mut scanned = 0;
        while scanned < 4096 && items.len() < limit {
            let page = db.ranking_page(&id, cursor.after, None, 256)?;
            if page.is_empty() {
                break;
            }
            let protection = db.effective_protection(&id, &page, cursor.review_watermark)?;
            for (ranking, effective_protected) in page.into_iter().zip(protection) {
                scanned += 1;
                cursor.after = ranking.position;
                if studio_application::aesthetic_analysis::matches_filter(
                    &ranking,
                    &value.filter,
                    effective_protected,
                ) {
                    items.push(analysis::AestheticSelectionRow {
                        ranking,
                        effective_protected,
                    });
                }
                if items.len() == limit {
                    break;
                }
            }
        }
        Ok(analysis::AestheticRankingSelection {
            items,
            next_cursor: if cursor.after < snapshot.input.candidates {
                Some(
                    URL_SAFE_NO_PAD.encode(serde_json::to_vec(&cursor).map_err(domain::Error::io)?),
                )
            } else {
                None
            },
            scanned,
            review_watermark: cursor.review_watermark,
        })
    })
    .await?;
    Ok(Json(wire(result)?))
}

async fn dispatch(
    s: &AppState,
    pid: &str,
    item: analysis::AestheticAnalysisJob,
) -> domain::Result<AestheticAnalysisJob> {
    if item.state == "queued" {
        let store = s.store.clone();
        let p = pid.to_owned();
        let id = item.id.clone();
        blocking(move || store.sync_analysis(&p, &id))
            .await
            .map_err(|e| e.0)?;
        if let Err(e) = s
            .aesthetic_analysis
            .launch(s.clone(), pid.into(), item.id.clone())
            && e.code != "REVISION_CONFLICT"
        {
            let store = s.store.clone();
            let p = pid.to_owned();
            let id = item.id.clone();
            let message = e.to_string();
            blocking(move || {
                store
                    .evaluation(&p)?
                    .analysis_fail(&id, "interrupted", &message)?;
                store.sync_analysis(&p, &id)
            })
            .await
            .map_err(|e| e.0)?;
            return Err(e);
        }
    }
    wire(item)
}
#[utoipa::path(operation_id="aesthetic_analysis_create",post,path="/jobs",params(("project_id"=String,Path)),request_body=AestheticAnalysisCreate,responses((status=200,body=AestheticAnalysisJob)))]
async fn create(
    State(s): State<AppState>,
    Path(pid): Path<String>,
    Body(value): Body<AestheticAnalysisCreate>,
) -> ApiResult<AestheticAnalysisJob> {
    let request = wire(value)?;
    let store = s.store.clone();
    let p = pid.clone();
    let item = blocking(move || store.evaluation(&p)?.analysis_create(request)).await?;
    Ok(Json(dispatch(&s, &pid, item).await?))
}
#[utoipa::path(operation_id="aesthetic_analysis_jobs",get,path="/jobs",params(("project_id"=String,Path),("after"=Option<String>,Query),("limit"=Option<usize>,Query),("experiment_id"=Option<String>,Query)),responses((status=200,body=AestheticAnalysisJobs)))]
async fn jobs(
    State(s): State<AppState>,
    Path(pid): Path<String>,
    Query(page): Query<Page>,
) -> ApiResult<AestheticAnalysisJobs> {
    let limit = page.limit.unwrap_or(25).clamp(1, 50);
    let mut items = blocking(move || {
        s.store.evaluation(&pid)?.analysis_jobs(
            page.after.as_deref().unwrap_or(""),
            page.experiment_id.as_deref(),
            limit + 1,
        )
    })
    .await?;
    let more = items.len() > limit;
    items.truncate(limit);
    Ok(Json(AestheticAnalysisJobs {
        next_cursor: if more {
            items.last().map(|r| r.id.clone())
        } else {
            None
        },
        items: wire(items)?,
    }))
}
#[utoipa::path(operation_id="aesthetic_analysis_job",get,path="/jobs/{id}",params(("project_id"=String,Path),("id"=String,Path)),responses((status=200,body=AestheticAnalysisJob)))]
async fn job(
    State(s): State<AppState>,
    Path((pid, id)): Path<(String, String)>,
) -> ApiResult<AestheticAnalysisJob> {
    Ok(Json(wire(
        blocking(move || s.store.evaluation(&pid)?.analysis_job(&id)).await?,
    )?))
}
#[utoipa::path(operation_id="aesthetic_analysis_control",post,path="/jobs/{id}/control",params(("project_id"=String,Path),("id"=String,Path)),request_body=AestheticAnalysisControl,responses((status=200,body=AestheticAnalysisJob)))]
async fn control(
    State(s): State<AppState>,
    Path((pid, id)): Path<(String, String)>,
    Body(value): Body<AestheticAnalysisControl>,
) -> ApiResult<AestheticAnalysisJob> {
    if value.action == "resume" && s.aesthetic_analysis.contains(&pid, &id) {
        return Err(
            domain::Error::new("REVISION_CONFLICT", "任务仍在退出，请等待状态稳定后继续").into(),
        );
    }
    let store = s.store.clone();
    let p = pid.clone();
    let key = id.clone();
    let action = value.action.clone();
    let item = blocking(move || {
        let item = store.evaluation(&p)?.analysis_control(&key, &action)?;
        store.sync_analysis(&p, &key)?;
        Ok(item)
    })
    .await?;
    if value.action == "cancel" {
        s.aesthetic_analysis.cancel(&pid, &id);
    }
    Ok(Json(dispatch(&s, &pid, item).await?))
}
#[utoipa::path(operation_id="aesthetic_snapshot",get,path="/snapshots/{id}",params(("project_id"=String,Path),("id"=String,Path)),responses((status=200,body=AestheticAnalysisJob)))]
async fn snapshot(
    State(s): State<AppState>,
    Path((pid, id)): Path<(String, String)>,
) -> ApiResult<AestheticAnalysisJob> {
    Ok(Json(wire(
        blocking(move || s.store.evaluation(&pid)?.ranking_snapshot(&id)).await?,
    )?))
}
#[utoipa::path(operation_id="aesthetic_ranking_rows",get,path="/snapshots/{id}/rows",params(("project_id"=String,Path),("id"=String,Path),("after"=Option<String>,Query),("limit"=Option<usize>,Query),("rating"=Option<String>,Query)),responses((status=200,body=AestheticRankingRows)))]
async fn rows(
    State(s): State<AppState>,
    Path((pid, id)): Path<(String, String)>,
    Query(page): Query<Page>,
) -> ApiResult<AestheticRankingRows> {
    let after = ordinal(page.after.as_deref())?;
    let limit = page.limit.unwrap_or(64).clamp(1, 128);
    let mut items = blocking(move || {
        s.store
            .evaluation(&pid)?
            .ranking_page(&id, after, page.rating.as_deref(), limit + 1)
    })
    .await?;
    let more = items.len() > limit;
    items.truncate(limit);
    Ok(Json(AestheticRankingRows {
        next_cursor: if more {
            items.last().map(|r| r.position.to_string())
        } else {
            None
        },
        items: wire(items)?,
    }))
}
#[utoipa::path(operation_id="aesthetic_ranking_candidate",get,path="/snapshots/{id}/candidates/{ordinal}",params(("project_id"=String,Path),("id"=String,Path),("ordinal"=u64,Path)),responses((status=200,body=AestheticRankingRow)))]
async fn candidate(
    State(s): State<AppState>,
    Path((pid, id, n)): Path<(String, String, u64)>,
) -> ApiResult<AestheticRankingRow> {
    if n > 1_000_000 {
        return Err(domain::Error::invalid("候选序号无效").into());
    }
    Ok(Json(wire(
        blocking(move || s.store.evaluation(&pid)?.ranking_candidate(&id, n)).await?,
    )?))
}
#[utoipa::path(operation_id="aesthetic_comparison_rows",get,path="/jobs/{id}/comparison",params(("project_id"=String,Path),("id"=String,Path),("after"=Option<String>,Query),("limit"=Option<usize>,Query)),responses((status=200,body=AestheticComparisonRows)))]
async fn comparison(
    State(s): State<AppState>,
    Path((pid, id)): Path<(String, String)>,
    Query(page): Query<Page>,
) -> ApiResult<AestheticComparisonRows> {
    let after = ordinal(page.after.as_deref())?;
    let limit = page.limit.unwrap_or(64).clamp(1, 128);
    let mut items = blocking(move || {
        s.store
            .evaluation(&pid)?
            .comparison_page(&id, after, limit + 1)
    })
    .await?;
    let more = items.len() > limit;
    items.truncate(limit);
    Ok(Json(AestheticComparisonRows {
        next_cursor: if more {
            items.last().map(|r| r.position.to_string())
        } else {
            None
        },
        items: wire(items)?,
    }))
}
#[utoipa::path(operation_id="aesthetic_experiment_create",post,path="/experiments",params(("project_id"=String,Path)),request_body=AestheticExperimentCreate,responses((status=200,body=AestheticExperiment)))]
async fn experiment_create(
    State(s): State<AppState>,
    Path(pid): Path<String>,
    Body(value): Body<AestheticExperimentCreate>,
) -> ApiResult<AestheticExperiment> {
    let value = wire(value)?;
    Ok(Json(wire(
        blocking(move || s.store.evaluation(&pid)?.experiment_create(value)).await?,
    )?))
}
#[utoipa::path(operation_id="aesthetic_experiments",get,path="/experiments",params(("project_id"=String,Path),("after"=Option<String>,Query)),responses((status=200,body=AestheticExperiments)))]
async fn experiments(
    State(s): State<AppState>,
    Path(pid): Path<String>,
    Query(page): Query<Page>,
) -> ApiResult<AestheticExperiments> {
    let mut items = blocking(move || {
        s.store
            .evaluation(&pid)?
            .experiments(page.after.as_deref().unwrap_or(""))
    })
    .await?;
    let more = items.len() > 25;
    items.truncate(25);
    Ok(Json(AestheticExperiments {
        next_cursor: if more {
            items.last().map(|r| r.id.clone())
        } else {
            None
        },
        items: wire(items)?,
    }))
}
#[utoipa::path(operation_id="aesthetic_experiment",get,path="/experiments/{id}",params(("project_id"=String,Path),("id"=String,Path)),responses((status=200,body=AestheticExperiment)))]
async fn experiment(
    State(s): State<AppState>,
    Path((pid, id)): Path<(String, String)>,
) -> ApiResult<AestheticExperiment> {
    Ok(Json(wire(
        blocking(move || s.store.evaluation(&pid)?.experiment(&id)).await?,
    )?))
}
#[utoipa::path(operation_id="aesthetic_experiment_run",post,path="/experiments/{id}/run",params(("project_id"=String,Path),("id"=String,Path)),responses((status=200,body=AestheticAnalysisJobs)))]
async fn experiment_run(
    State(s): State<AppState>,
    Path((pid, id)): Path<(String, String)>,
) -> ApiResult<AestheticAnalysisJobs> {
    let store = s.store.clone();
    let p = pid.clone();
    let jobs = blocking(move || {
        let db = store.evaluation(&p)?;
        let experiment = db.experiment(&id)?;
        experiment
            .request
            .variants
            .into_iter()
            .enumerate()
            .map(|(index, v)| {
                let hash = hex::encode(Sha256::digest(
                    format!("aesthetic-experiment:{id}:{index}").as_bytes(),
                ));
                db.analysis_create(analysis::AestheticAnalysisCreate {
                    idempotency_key: hash[..32].into(),
                    name: v.label.clone(),
                    spec: analysis::AestheticAnalysisSpec::Fit {
                        config: v.fit,
                        experiment_id: Some(id.clone()),
                        variant: Some(v.label),
                    },
                })
            })
            .collect::<domain::Result<Vec<_>>>()
    })
    .await?;
    let mut items = Vec::new();
    for job in jobs {
        items.push(dispatch(&s, &pid, job).await?);
    }
    Ok(Json(AestheticAnalysisJobs {
        items,
        next_cursor: None,
    }))
}
#[utoipa::path(operation_id="aesthetic_review_create",post,path="/reviews",params(("project_id"=String,Path)),request_body=AestheticReviewCreate,responses((status=200,body=AestheticReview)))]
async fn review_create(
    State(s): State<AppState>,
    Path(pid): Path<String>,
    Body(value): Body<AestheticReviewCreate>,
) -> ApiResult<AestheticReview> {
    let value = wire(value)?;
    Ok(Json(wire(
        blocking(move || s.store.evaluation(&pid)?.review_create(value)).await?,
    )?))
}
#[utoipa::path(operation_id="aesthetic_reviews",get,path="/snapshots/{id}/reviews",params(("project_id"=String,Path),("id"=String,Path),("after"=Option<String>,Query),("ordinal"=Option<u64>,Query,description="Optional candidate ordinal; returns newest first, with after as an exclusive upper sequence bound.")),responses((status=200,body=AestheticReviews)))]
async fn reviews(
    State(s): State<AppState>,
    Path((pid, id)): Path<(String, String)>,
    Query(page): Query<Page>,
) -> ApiResult<AestheticReviews> {
    let after = ordinal(page.after.as_deref())?;
    let mut items = blocking(move || {
        let db = s.store.evaluation(&pid)?;
        match page.ordinal {
            Some(candidate) => db.candidate_reviews(&id, candidate, after),
            None => db.reviews(&id, after),
        }
    })
    .await?;
    let more = items.len() > 64;
    items.truncate(64);
    Ok(Json(AestheticReviews {
        next_cursor: if more {
            items.last().map(|r| r.sequence.to_string())
        } else {
            None
        },
        items: wire(items)?,
    }))
}
#[derive(OpenApi)]
#[openapi(paths(
    create,
    jobs,
    job,
    control,
    snapshot,
    rows,
    candidate,
    select,
    comparison,
    experiment_create,
    experiments,
    experiment,
    experiment_run,
    review_create,
    reviews
))]
pub struct AestheticAnalysisApiDoc;
pub(super) fn routes() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/jobs", get(jobs).post(create))
        .route("/jobs/{id}", get(job))
        .route("/jobs/{id}/control", post(control))
        .route("/jobs/{id}/comparison", get(comparison))
        .route("/snapshots/{id}", get(snapshot))
        .route("/snapshots/{id}/rows", get(rows))
        .route("/snapshots/{id}/select", post(select))
        .route("/snapshots/{id}/candidates/{ordinal}", get(candidate))
        .route("/snapshots/{id}/reviews", get(reviews))
        .route("/experiments", get(experiments).post(experiment_create))
        .route("/experiments/{id}", get(experiment))
        .route("/experiments/{id}/run", post(experiment_run))
        .route("/reviews", post(review_create))
}
