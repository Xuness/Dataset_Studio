use super::*;
use studio_application::{ArtifactRepository, DraftRepository};
#[utoipa::path(post,path="/v1/projects/{project_id}/tools/validate-scope",params(("project_id"=String,Path)),request_body=CaptureScope,responses((status=200,body=OkResponse)))]
pub(super) async fn validate_scope(
    State(s): State<AppState>,
    Path(pid): Path<String>,
    Body(body): Body<CaptureScope>,
) -> ApiResult<OkResponse> {
    Ok(Json(
        blocking(move || {
            let scope = body.scope.into();
            query::validate_scope(&s, &pid, &scope)?;
            if matches!(scope.target, domain::ScopeTarget::Source { .. }) {
                query::source_capture(&s, &pid, &scope)?;
            } else {
                s.store.scope_source_ids(&pid, &scope)?;
            }
            Ok(OkResponse { ok: true })
        })
        .await?,
    ))
}

#[utoipa::path(get,path="/v1/operators",responses((status=200,body=Operators)))]
pub(super) async fn operators() -> ApiResult<Operators> {
    Ok(Json(Operators {
        protocol_version: 1,
        items: studio_operators::registry()?
            .descriptors()
            .into_iter()
            .map(Into::into)
            .collect(),
    }))
}
#[utoipa::path(post,path="/v1/projects/{project_id}/tools/jobs",params(("project_id"=String,Path)),request_body=ToolSubmission,responses((status=200,body=Job)))]
pub(super) async fn submit(
    State(s): State<AppState>,
    Extension(read_context): Extension<RequestReadContext>,
    Path(pid): Path<String>,
    Body(body): Body<ToolSubmission>,
) -> ApiResult<Job> {
    Ok(Json(
        blocking(move || {
            let _lease = s.store.operation_lease(&pid)?;
            let _permit = read_permit(&s, domain::ReadClass::Index, &read_context)?;
            let mut request: domain::ToolSubmission = body.into();
            request.scope.validate_project(&pid)?;
            request.run = studio_operators::registry()?.normalize(request.run)?;
            if let Some(old) = s.store.retry_registered_job(&pid, &request)? {
                return Ok(old.into());
            }
            query::validate_scope(&s, &pid, &request.scope)?;
            let frozen = crate::tool_inputs::capture(
                &s.store,
                &pid,
                &request.scope,
                request.run.clone(),
                &s.sources,
            )?;
            let capture = if matches!(request.scope.target, domain::ScopeTarget::Source { .. }) {
                Some(query::source_capture(&s, &pid, &request.scope)?)
            } else {
                None
            };
            let job = s
                .store
                .submit_registered_job(&pid, &request, &frozen, capture)?;
            if domain::is_ranking_operator(&job.operator) && job.stage.is_none() {
                s.store.job_stage(
                    &pid,
                    &job.id,
                    &domain::JobStage {
                        name: if job.input_members_frozen {
                            "queued"
                        } else {
                            "waiting_input"
                        }
                        .into(),
                        total: job.total,
                        ..Default::default()
                    },
                )?;
            }
            s.store.job(&pid, &job.id).map(Into::into)
        })
        .await?,
    ))
}
#[utoipa::path(get,path="/v1/projects/{project_id}/jobs/{job_id}/run",params(("project_id"=String,Path),("job_id"=String,Path)),responses((status=200,body=JobRun)))]
pub(super) async fn run(
    State(s): State<AppState>,
    Path((pid, jid)): Path<(String, String)>,
) -> ApiResult<JobRun> {
    Ok(Json(
        blocking(move || s.store.job_run(&pid, &jid).map(Into::into)).await?,
    ))
}
#[utoipa::path(post,path="/v1/projects/{project_id}/jobs/{job_id}/retry",params(("project_id"=String,Path),("job_id"=String,Path)),responses((status=200,body=Job)))]
pub(super) async fn retry(
    State(s): State<AppState>,
    Path((pid, jid)): Path<(String, String)>,
) -> ApiResult<Job> {
    let job = s.store.job(&pid, &jid)?;
    if !matches!(job.status.as_str(), "failed" | "cancelled") {
        return Err(domain::Error::new("JOB_NOT_RETRYABLE", "任务尚未失败或取消").into());
    }
    crate::jobs::wait_stopped(&pid, &jid).await?;
    Ok(Json(
        blocking(move || {
            let fixed = s.store.job_run(&pid, &jid)?;
            let registry = studio_operators::registry()?;
            if !registry
                .resolve(&fixed.run)?
                .descriptor()
                .capabilities
                .retry
            {
                return Err(domain::Error::new(
                    "JOB_NOT_RETRYABLE",
                    "算子版本不支持重试",
                ));
            }
            s.store.retry_job(&pid, &jid).map(Into::into)
        })
        .await?,
    ))
}
#[utoipa::path(get,path="/v1/projects/{project_id}/artifacts",params(("project_id"=String,Path),("cursor"=Option<String>,Query),("limit"=Option<usize>,Query)),responses((status=200,body=Artifacts)))]
pub(super) async fn artifacts(
    State(s): State<AppState>,
    Path(pid): Path<String>,
    Query(q): Query<QueryListParams>,
) -> ApiResult<Artifacts> {
    Ok(Json(
        blocking(move || {
            let limit = q.limit.unwrap_or(48).clamp(1, 128);
            let mut items = s.store.artifacts(&pid, q.cursor.as_deref(), limit + 1)?;
            let more = items.len() > limit;
            items.truncate(limit);
            Ok(Artifacts {
                next_cursor: if more {
                    items.last().map(|a| a.id.clone())
                } else {
                    None
                },
                items: items.into_iter().map(Into::into).collect(),
            })
        })
        .await?,
    ))
}
#[utoipa::path(get,path="/v1/projects/{project_id}/artifacts/{artifact_id}",operation_id="get_artifact",params(("project_id"=String,Path),("artifact_id"=String,Path)),responses((status=200,body=Artifact)))]
pub(super) async fn artifact(
    State(s): State<AppState>,
    Path((pid, aid)): Path<(String, String)>,
) -> ApiResult<Artifact> {
    Ok(Json(
        blocking(move || {
            let _lease = s.store.operation_lease(&pid)?;
            let item = s.store.artifact(&pid, &aid)?;
            if item.state == domain::ArtifactState::Legacy {
                crate::artifacts::verify(&s.store, &pid, &aid).map(Into::into)
            } else {
                Ok(item.into())
            }
        })
        .await?,
    ))
}
#[utoipa::path(post,path="/v1/projects/{project_id}/artifacts/{artifact_id}/verify",params(("project_id"=String,Path),("artifact_id"=String,Path)),responses((status=200,body=Artifact)))]
pub(super) async fn verify(
    State(s): State<AppState>,
    Path((pid, aid)): Path<(String, String)>,
) -> ApiResult<Artifact> {
    Ok(Json(
        blocking(move || {
            let _lease = s.store.operation_lease(&pid)?;
            crate::artifacts::verify(&s.store, &pid, &aid).map(Into::into)
        })
        .await?,
    ))
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactCursor {
    project_id: String,
    artifact_id: String,
    after: domain::AssetKey,
}
#[utoipa::path(get,path="/v1/projects/{project_id}/artifacts/{artifact_id}/rows",params(("project_id"=String,Path),("artifact_id"=String,Path),("cursor"=Option<String>,Query),("limit"=Option<usize>,Query)),responses((status=200,body=ArtifactPage)))]
pub(super) async fn rows(
    State(s): State<AppState>,
    Path((pid, aid)): Path<(String, String)>,
    Query(q): Query<QueryListParams>,
) -> ApiResult<ArtifactPage> {
    Ok(Json(
        blocking(move || {
            let after = q
                .cursor
                .map(|value| -> domain::Result<_> {
                    if value.len() > 2048 {
                        return Err(domain::Error::invalid("成果游标过长"));
                    }
                    let cursor: ArtifactCursor = URL_SAFE_NO_PAD
                        .decode(value)
                        .ok()
                        .and_then(|b| serde_json::from_slice(&b).ok())
                        .ok_or_else(|| domain::Error::invalid("无效成果游标"))?;
                    if cursor.project_id != pid || cursor.artifact_id != aid {
                        return Err(domain::Error::invalid("成果游标不属于当前项目或成果"));
                    }
                    Ok(cursor.after)
                })
                .transpose()?;
            let page = s
                .store
                .artifact_page(&pid, &aid, after.as_ref(), q.limit.unwrap_or(48))?;
            let next_cursor = page
                .next
                .map(|after| {
                    serde_json::to_vec(&ArtifactCursor {
                        project_id: pid,
                        artifact_id: aid.clone(),
                        after,
                    })
                    .map(|b| URL_SAFE_NO_PAD.encode(b))
                    .map_err(domain::Error::io)
                })
                .transpose()?;
            Ok(ArtifactPage {
                artifact_id: aid,
                items: page.items.into_iter().map(Into::into).collect(),
                next_cursor,
            })
        })
        .await?,
    ))
}
#[utoipa::path(post,path="/v1/projects/{project_id}/artifacts/{artifact_id}/release",operation_id="release_artifact",params(("project_id"=String,Path),("artifact_id"=String,Path)),responses((status=200,body=Artifact)))]
pub(super) async fn release(
    State(s): State<AppState>,
    Path((pid, aid)): Path<(String, String)>,
) -> ApiResult<Artifact> {
    Ok(Json(
        blocking(move || {
            let _lease = s.store.operation_lease(&pid)?;
            let item = super::management::release_files(&s, &pid, &aid)?;
            Ok(item.into())
        })
        .await?,
    ))
}
#[utoipa::path(get,path="/v1/projects/{project_id}/drafts/{module_id}/{instance_id}",params(("project_id"=String,Path),("module_id"=String,Path),("instance_id"=String,Path)),responses((status=200,body=MaybeDraft)))]
pub(super) async fn draft(
    State(s): State<AppState>,
    Path((pid, module, instance)): Path<(String, String, String)>,
) -> ApiResult<MaybeDraft> {
    Ok(Json(
        blocking(move || {
            Ok(MaybeDraft {
                draft: s.store.draft(&pid, &module, &instance)?.map(Into::into),
            })
        })
        .await?,
    ))
}
#[utoipa::path(put,path="/v1/projects/{project_id}/drafts/{module_id}/{instance_id}",params(("project_id"=String,Path),("module_id"=String,Path),("instance_id"=String,Path)),request_body=SaveDraft,responses((status=200,body=Draft)))]
pub(super) async fn save_draft(
    State(s): State<AppState>,
    Path((pid, module, instance)): Path<(String, String, String)>,
    Body(body): Body<SaveDraft>,
) -> ApiResult<Draft> {
    Ok(Json(
        blocking(move || {
            s.store
                .save_draft(&pid, &module, &instance, body.into())
                .map(Into::into)
        })
        .await?,
    ))
}
#[utoipa::path(get,path="/v1/preferences/{key}",params(("key"=String,Path)),responses((status=200,body=MaybePreference)))]
pub(super) async fn preference(
    State(s): State<AppState>,
    Path(key): Path<String>,
) -> ApiResult<MaybePreference> {
    Ok(Json(
        blocking(move || {
            Ok(MaybePreference {
                preference: s.store.preference(&key)?.map(Into::into),
            })
        })
        .await?,
    ))
}
#[utoipa::path(put,path="/v1/preferences/{key}",params(("key"=String,Path)),request_body=SaveDraft,responses((status=200,body=Preference)))]
pub(super) async fn save_preference(
    State(s): State<AppState>,
    Path(key): Path<String>,
    Body(body): Body<SaveDraft>,
) -> ApiResult<Preference> {
    Ok(Json(
        blocking(move || s.store.save_preference(&key, body.into()).map(Into::into)).await?,
    ))
}
