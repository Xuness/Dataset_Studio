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
    after: Option<RankingPosition>,
    #[serde(default)]
    pending: Vec<u64>,
    #[serde(default)]
    scanned: u64,
}
fn encode(cursor: &Cursor) -> domain::Result<String> {
    Ok(URL_SAFE_NO_PAD.encode(serde_json::to_vec(cursor).map_err(domain::Error::io)?))
}
fn count_key(
    pid: &str,
    artifact: &domain::Artifact,
    filter: &domain::RankingFilter,
) -> domain::Result<String> {
    Ok(hex::encode(Sha256::digest(
        serde_json::to_vec(&(pid, &artifact.id, &artifact.files, filter))
            .map_err(domain::Error::io)?,
    )))
}
fn count_state(
    s: &AppState,
    key: &str,
) -> Arc<std::sync::Mutex<crate::ranking_reads::CountProgress>> {
    s.ranking_reads.counts.get_or_insert(key.to_owned(), || {
        Arc::new(std::sync::Mutex::new(
            crate::ranking_reads::CountProgress::default(),
        ))
    })
}
fn finish_page(
    aid: &str,
    input: &RankingInputTable,
    mut cursor: Cursor,
    mut picked: Vec<domain::RankingScores>,
    limit: usize,
    count: Option<u64>,
) -> domain::Result<RankingPage> {
    let next_cursor = if picked.len() > limit {
        let extra = &picked[limit];
        cursor.after = Some(RankingPosition::for_scores(extra, cursor.filter.order));
        cursor.pending = vec![extra.ordinal];
        cursor.scanned = 0;
        Some(encode(&cursor)?)
    } else {
        None
    };
    picked.truncate(limit);
    let items = picked
        .into_iter()
        .map(|scores| {
            Ok(domain::RankingRow {
                input: input.row(scores.ordinal)?,
                scores,
            }
            .into())
        })
        .collect::<domain::Result<Vec<_>>>()?;
    Ok(RankingPage {
        artifact_id: aid.into(),
        items,
        next_cursor,
        count,
        preparing: None,
        scan: None,
    })
}
fn read_page(
    s: &AppState,
    read: &RequestReadContext,
    pid: &str,
    aid: &str,
    body: RankingPageRequest,
) -> domain::Result<RankingPage> {
    let _permit = read_permit(s, domain::ReadClass::Index, read)?;
    let _lease = s.store.operation_lease(pid)?;
    let (artifact, table_path, input_path) = crate::ranking::paths(&s.store, pid, aid)?;
    let filter: domain::RankingFilter = body.filter.into();
    filter.validate()?;
    let total = artifact
        .count
        .ok_or_else(|| domain::Error::new("ARTIFACT_INVALID", "排名输入数量缺失"))?;
    let table = RankingResultTable::open(&table_path)?;
    table.cancel_reads(read.cancelled.clone())?;
    let input = RankingInputTable::open(&input_path)?;
    let count = table.known_count(&filter)?.or_else(|| {
        s.ranking_reads
            .counts
            .get(&count_key(pid, &artifact, &filter).ok()?)
            .and_then(|v| {
                v.lock()
                    .ok()
                    .and_then(|p| (p.scanned == total).then_some(p.count))
            })
    });
    let mut cursor = if let Some(raw) = body.cursor {
        if raw.len() > 16384 {
            return Err(domain::Error::invalid("排名游标过长"));
        }
        let cursor: Cursor = URL_SAFE_NO_PAD
            .decode(raw)
            .ok()
            .and_then(|v| serde_json::from_slice(&v).ok())
            .ok_or_else(|| domain::Error::invalid("排名游标无效"))?;
        if cursor.project_id != pid
            || cursor.artifact_id != aid
            || cursor.filter != filter
            || cursor.pending.len() > 129
            || cursor.scanned > total
        {
            return Err(domain::Error::invalid("排名游标不属于当前结果或过滤条件"));
        }
        cursor
    } else {
        Cursor {
            project_id: pid.into(),
            artifact_id: aid.into(),
            filter: filter.clone(),
            after: None,
            pending: Vec::new(),
            scanned: 0,
        }
    };
    if let Some(after) = &cursor.after
        && (after.ordinal >= total
            || RankingPosition::for_scores(&table.row(after.ordinal)?, filter.order)
                .compare(after, false)
                != std::cmp::Ordering::Equal)
    {
        return Err(domain::Error::invalid("排名游标位置无效"));
    }
    let mut picked = Vec::new();
    let mut previous: Option<RankingPosition> = None;
    for ordinal in cursor.pending.drain(..) {
        let scores = table.row(ordinal)?;
        let position = RankingPosition::for_scores(&scores, filter.order);
        if ordinal >= total
            || !table.matches_filter(ordinal, &filter)?
            || previous
                .as_ref()
                .is_some_and(|v| v.compare(&position, false) != std::cmp::Ordering::Less)
            || cursor
                .after
                .as_ref()
                .is_none_or(|v| position.compare(v, false) == std::cmp::Ordering::Greater)
        {
            return Err(domain::Error::invalid("排名分页缓冲无效"));
        }
        previous = Some(position);
        picked.push(scores);
    }
    let limit = body.limit.unwrap_or(48).clamp(1, 128);
    let mut examined = 0;
    loop {
        studio_application::read_cancelled(&read.cancelled)?;
        if picked.len() > limit {
            return finish_page(aid, &input, cursor, picked, limit, count);
        }
        if examined >= 4096 {
            cursor.pending = picked.iter().map(|r| r.ordinal).collect();
            return Ok(RankingPage {
                artifact_id: aid.into(),
                items: Vec::new(),
                next_cursor: Some(encode(&cursor)?),
                count,
                preparing: Some("正在读取这一页的排名成员".into()),
                scan: Some(BrowseScan {
                    scanned: cursor.scanned,
                    total,
                }),
            });
        }
        let page = table.browse_scan(
            &filter,
            filter.order,
            false,
            cursor.after.as_ref(),
            (4096 - examined).min(512),
        )?;
        for (scores, matches) in page.rows {
            examined += 1;
            cursor.scanned += 1;
            cursor.after = Some(RankingPosition::for_scores(&scores, filter.order));
            if matches {
                picked.push(scores);
            }
            if picked.len() > limit {
                return finish_page(aid, &input, cursor, picked, limit, count);
            }
        }
        if !page.more {
            return finish_page(aid, &input, cursor, picked, limit, count);
        }
    }
}
#[utoipa::path(post,path="/v1/projects/{project_id}/artifacts/{artifact_id}/ranking/rows",operation_id="ranking_rows",params(("project_id"=String,Path),("artifact_id"=String,Path)),request_body=RankingPageRequest,responses((status=200,body=RankingPage)))]
pub(super) async fn rows(
    State(s): State<AppState>,
    Extension(read): Extension<RequestReadContext>,
    Path((pid, aid)): Path<(String, String)>,
    Body(body): Body<RankingPageRequest>,
) -> ApiResult<RankingPage> {
    Ok(Json(
        blocking(move || {
            let result = read_page(&s, &read, &pid, &aid, body);
            studio_application::read_cancelled(&read.cancelled)?;
            result
        })
        .await?,
    ))
}
#[utoipa::path(post,path="/v1/projects/{project_id}/artifacts/{artifact_id}/ranking/count",operation_id="ranking_count",params(("project_id"=String,Path),("artifact_id"=String,Path)),request_body=RankingCountRequest,responses((status=200,body=RankingCount)))]
pub(super) async fn count(
    State(s): State<AppState>,
    Extension(read): Extension<RequestReadContext>,
    Path((pid, aid)): Path<(String, String)>,
    Body(body): Body<RankingCountRequest>,
) -> ApiResult<RankingCount> {
    Ok(Json(
        blocking(move || {
            let _permit = read_permit(&s, domain::ReadClass::Index, &read)?;
            let _lease = s.store.operation_lease(&pid)?;
            let (artifact, table_path, _) = crate::ranking::paths(&s.store, &pid, &aid)?;
            let total = artifact
                .count
                .ok_or_else(|| domain::Error::new("ARTIFACT_INVALID", "排名输入数量缺失"))?;
            let filter: domain::RankingFilter = body.filter.into();
            filter.validate()?;
            let table = RankingResultTable::open(&table_path)?;
            table.cancel_reads(read.cancelled.clone())?;
            if let Some(value) = table.known_count(&filter)? {
                return Ok(RankingCount {
                    count: Some(value),
                    scanned: total,
                    total,
                });
            }
            let progress = count_state(&s, &count_key(&pid, &artifact, &filter)?);
            let mut progress = match progress.try_lock() {
                Ok(progress) => progress,
                Err(std::sync::TryLockError::WouldBlock) => {
                    return Ok(RankingCount {
                        count: None,
                        scanned: 0,
                        total,
                    });
                }
                Err(_) => return Err(domain::Error::new("INTERNAL_ERROR", "排名统计不可用")),
            };
            studio_application::read_cancelled(&read.cancelled)?;
            if progress.scanned < total {
                let result = table.count_scan(&filter, progress.scanned, total);
                studio_application::read_cancelled(&read.cancelled)?;
                let (scanned, added) = result?;
                progress.scanned = scanned;
                progress.count += added;
            }
            Ok(RankingCount {
                count: (progress.scanned == total).then_some(progress.count),
                scanned: progress.scanned,
                total,
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
            let full_tags = input.is_v2()?;
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
                .into_iter()
                .chain(full_tags.then_some("tag_string"))
                .map(String::from)
                .collect(),
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
