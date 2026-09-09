use super::*;
use studio_storage::ranking_tables::{RankingInputTable, RankingPosition, RankingResultTable};

#[utoipa::path(get,path="/v1/projects/{project_id}/artifacts/{artifact_id}/ranking",operation_id="ranking_summary",params(("project_id"=String,Path),("artifact_id"=String,Path)),responses((status=200,body=RankingSummary)))]
pub(super) async fn summary(
    State(s): State<AppState>,
    Path((pid, aid)): Path<(String, String)>,
) -> ApiResult<RankingSummary> {
    Ok(Json(
        blocking(move || {
            let _lease = s.store.operation_lease(&pid)?;
            crate::ranking::paths(&s.store, &pid, &aid)?;
            s.store.ranking_summary(&pid, &aid).map(Into::into)
        })
        .await?,
    ))
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    project_id: String,
    artifact_id: String,
    filter: domain::RankingFilter,
    after: RankingPosition,
}
#[utoipa::path(post,path="/v1/projects/{project_id}/artifacts/{artifact_id}/ranking/rows",operation_id="ranking_rows",params(("project_id"=String,Path),("artifact_id"=String,Path)),request_body=RankingPageRequest,responses((status=200,body=RankingPage)))]
pub(super) async fn rows(
    State(s): State<AppState>,
    Path((pid, aid)): Path<(String, String)>,
    Body(body): Body<RankingPageRequest>,
) -> ApiResult<RankingPage> {
    Ok(Json(
        blocking(move || {
            let _lease = s.store.operation_lease(&pid)?;
            let (_, table, input) = crate::ranking::paths(&s.store, &pid, &aid)?;
            let filter: domain::RankingFilter = body.filter.into();
            filter.validate()?;
            let after = body
                .cursor
                .map(|cursor| -> domain::Result<RankingPosition> {
                    if cursor.len() > 4096 {
                        return Err(domain::Error::invalid("排名游标过长"));
                    }
                    let cursor: Cursor = URL_SAFE_NO_PAD
                        .decode(cursor)
                        .ok()
                        .and_then(|v| serde_json::from_slice(&v).ok())
                        .ok_or_else(|| domain::Error::invalid("排名游标无效"))?;
                    if cursor.project_id != pid
                        || cursor.artifact_id != aid
                        || cursor.filter != filter
                    {
                        return Err(domain::Error::invalid("排名游标不属于当前结果或过滤条件"));
                    }
                    Ok(cursor.after)
                })
                .transpose()?;
            let table = RankingResultTable::open(&table)?;
            let input = RankingInputTable::open(&input)?;
            let count = if after.is_none() {
                Some(table.filtered_count(&filter)?)
            } else {
                None
            };
            let (rows, next) =
                table.filtered_page(&filter, after.as_ref(), body.limit.unwrap_or(48))?;
            let items = rows
                .into_iter()
                .map(|scores| {
                    Ok(domain::RankingRow {
                        input: input.row(scores.ordinal)?,
                        scores,
                    }
                    .into())
                })
                .collect::<domain::Result<Vec<_>>>()?;
            let next_cursor = next
                .map(|after| {
                    serde_json::to_vec(&Cursor {
                        project_id: pid,
                        artifact_id: aid.clone(),
                        filter,
                        after,
                    })
                    .map(|v| URL_SAFE_NO_PAD.encode(v))
                    .map_err(domain::Error::io)
                })
                .transpose()?;
            Ok(RankingPage {
                artifact_id: aid,
                items,
                next_cursor,
                count,
            })
        })
        .await?,
    ))
}
#[utoipa::path(get,path="/v1/projects/{project_id}/artifacts/{artifact_id}/ranking/evidence",operation_id="ranking_evidence",params(("project_id"=String,Path),("artifact_id"=String,Path)),responses((status=200,body=RankingEvidence)))]
pub(super) async fn evidence(
    State(s): State<AppState>,
    Path((pid, aid)): Path<(String, String)>,
) -> ApiResult<RankingEvidence> {
    Ok(Json(
        blocking(move || {
            let _lease = s.store.operation_lease(&pid)?;
            let (_, _, input) = crate::ranking::paths(&s.store, &pid, &aid)?;
            let input = RankingInputTable::open(&input)?;
            let job_run: domain::JobRun = input.meta("job_run")?;
            let bases: Vec<domain::RankingBasis> = input.meta("bases")?;
            Ok(RankingEvidence {
                job_run: job_run.into(),
                bases: bases.into_iter().map(Into::into).collect(),
                metadata_fields: [
                    "rating",
                    "fav_count",
                    "up_score",
                    "down_score",
                    "score",
                    "created_at",
                    "observed_at",
                    "time_quality",
                    "tag_string_artist",
                    "technical_damage_tokens",
                    "parent_id",
                    "stored_dimensions_when_required",
                    "status_flags",
                    "observation_and_record_identity",
                ]
                .map(String::from)
                .into(),
            })
        })
        .await?,
    ))
}
#[utoipa::path(post,path="/v1/projects/{project_id}/artifacts/{artifact_id}/ranking/worksets",operation_id="ranking_workset",params(("project_id"=String,Path),("artifact_id"=String,Path)),request_body=RankingWorksetRequest,responses((status=200,body=Collection)))]
pub(super) async fn workset(
    State(s): State<AppState>,
    Path((pid, aid)): Path<(String, String)>,
    Body(body): Body<RankingWorksetRequest>,
) -> ApiResult<Collection> {
    Ok(Json(
        blocking(move || {
            let _lease = s.store.operation_lease(&pid)?;
            let (_, table, input) = crate::ranking::paths(&s.store, &pid, &aid)?;
            s.store
                .ranking_workset(
                    &pid,
                    &aid,
                    &body.idempotency_key,
                    &body.name,
                    &body.filter.into(),
                    (&table, &input),
                )
                .map(Into::into)
        })
        .await?,
    ))
}
#[utoipa::path(get,path="/v1/projects/{project_id}/jobs/{job_id}/ranking",operation_id="ranking_job_result",params(("project_id"=String,Path),("job_id"=String,Path)),responses((status=200,body=Artifact)))]
pub(super) async fn job_result(
    State(s): State<AppState>,
    Path((pid, jid)): Path<(String, String)>,
) -> ApiResult<Artifact> {
    Ok(Json(
        blocking(move || s.store.ranking_job_artifact(&pid, &jid).map(Into::into)).await?,
    ))
}
