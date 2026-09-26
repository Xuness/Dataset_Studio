use super::*;
use studio_application::{ArtifactRepository, ManagementRepository};

#[derive(Deserialize)]
pub(super) struct ListParams {
    #[serde(default)]
    search: String,
    #[serde(default)]
    order: String,
    #[serde(default)]
    state: String,
    subtype: Option<String>,
    #[serde(default)]
    include_archived: bool,
    cursor: Option<String>,
    limit: Option<usize>,
}
#[utoipa::path(get,path="/v1/projects/{project_id}/objects/{kind}",params(("project_id"=String,Path),("kind"=String,Path),("search"=Option<String>,Query),("order"=Option<String>,Query),("state"=Option<String>,Query),("subtype"=Option<String>,Query),("include_archived"=Option<bool>,Query),("cursor"=Option<String>,Query),("limit"=Option<usize>,Query)),responses((status=200,body=ObjectPage)))]
pub(super) async fn list(
    State(s): State<AppState>,
    Path((pid, kind)): Path<(String, String)>,
    Query(q): Query<ListParams>,
) -> ApiResult<ObjectPage> {
    Ok(Json(
        blocking(move || {
            s.store
                .managed_objects(
                    &pid,
                    domain::ObjectListing {
                        kind: domain::ObjectKind::parse(&kind)?,
                        search: q.search,
                        order: q.order,
                        state: q.state,
                        subtype: q.subtype,
                        include_archived: q.include_archived,
                        after: q.cursor,
                        limit: q.limit.unwrap_or(32),
                    },
                )
                .map(Into::into)
        })
        .await?,
    ))
}
#[utoipa::path(get,path="/v1/projects/{project_id}/job-history",params(("project_id"=String,Path),("search"=Option<String>,Query),("state"=Option<String>,Query),("include_archived"=Option<bool>,Query),("order"=Option<String>,Query),("cursor"=Option<String>,Query),("limit"=Option<usize>,Query)),responses((status=200,body=ManagedJobPage)))]
pub(super) async fn jobs(
    State(s): State<AppState>,
    Path(pid): Path<String>,
    Query(q): Query<ListParams>,
) -> ApiResult<ManagedJobPage> {
    Ok(Json(
        blocking(move || {
            let page = s.store.managed_objects(
                &pid,
                domain::ObjectListing {
                    kind: domain::ObjectKind::Job,
                    search: q.search,
                    order: q.order,
                    state: q.state,
                    subtype: q.subtype,
                    include_archived: q.include_archived,
                    after: q.cursor,
                    limit: q.limit.unwrap_or(32),
                },
            )?;
            let items = page
                .items
                .into_iter()
                .map(|object| {
                    Ok(ManagedJob {
                        job: s.store.job(&pid, &object.id)?.into(),
                        result_available: s.store.job_result_available(&pid, &object.id)?,
                        object: object.into(),
                    })
                })
                .collect::<domain::Result<Vec<_>>>()?;
            Ok(ManagedJobPage {
                items,
                next_cursor: page.next_cursor,
            })
        })
        .await?,
    ))
}
fn details(
    s: &AppState,
    pid: &str,
    kind: domain::ObjectKind,
    id: &str,
) -> domain::Result<domain::ObjectDetails> {
    let mut value = s.store.object_details(pid, kind, id)?;
    if kind == domain::ObjectKind::Artifact {
        let a = s.store.artifact(pid, id)?;
        let mut bytes = 0u64;
        value.paths.clear();
        for file in &a.files {
            let path = crate::artifacts::controlled_path(&s.store, pid, &file.path)?;
            match std::fs::metadata(&path) {
                Ok(metadata) => bytes = bytes.saturating_add(metadata.len()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(domain::Error::io(e)),
            }
            value.paths.push(studio_protocol::display_path(&path));
        }
        value.object.bytes = Some(bytes.to_string());
    } else {
        value.paths = value
            .paths
            .into_iter()
            .map(|p| studio_protocol::display_path(std::path::Path::new(&p)))
            .collect();
    }
    Ok(value)
}
#[utoipa::path(get,path="/v1/projects/{project_id}/objects/{kind}/{object_id}",params(("project_id"=String,Path),("kind"=String,Path),("object_id"=String,Path)),responses((status=200,body=ObjectDetails)))]
pub(super) async fn read(
    State(s): State<AppState>,
    Path((pid, kind, id)): Path<(String, String, String)>,
) -> ApiResult<ObjectDetails> {
    Ok(Json(
        blocking(move || details(&s, &pid, domain::ObjectKind::parse(&kind)?, &id).map(Into::into))
            .await?,
    ))
}
#[utoipa::path(patch,path="/v1/projects/{project_id}/objects/{kind}/{object_id}",params(("project_id"=String,Path),("kind"=String,Path),("object_id"=String,Path)),request_body=EditObject,responses((status=200,body=ManagedObject)))]
pub(super) async fn edit(
    State(s): State<AppState>,
    Path((pid, kind, id)): Path<(String, String, String)>,
    Body(body): Body<EditObject>,
) -> ApiResult<ManagedObject> {
    Ok(Json(
        blocking(move || {
            let _lease = s.store.operation_lease(&pid)?;
            s.store
                .edit_object(&pid, domain::ObjectKind::parse(&kind)?, &id, body.into())
                .map(Into::into)
        })
        .await?,
    ))
}
#[derive(Deserialize)]
pub(super) struct LinkParams {
    #[serde(default)]
    incoming: bool,
    cursor: Option<String>,
    limit: Option<usize>,
}
#[utoipa::path(get,path="/v1/projects/{project_id}/objects/{kind}/{object_id}/links",params(("project_id"=String,Path),("kind"=String,Path),("object_id"=String,Path),("incoming"=Option<bool>,Query),("cursor"=Option<String>,Query),("limit"=Option<usize>,Query)),responses((status=200,body=ObjectLinkPage)))]
pub(super) async fn links(
    State(s): State<AppState>,
    Path((pid, kind, id)): Path<(String, String, String)>,
    Query(q): Query<LinkParams>,
) -> ApiResult<ObjectLinkPage> {
    Ok(Json(
        blocking(move || {
            s.store
                .object_links(
                    &pid,
                    domain::ObjectKind::parse(&kind)?,
                    &id,
                    q.incoming,
                    q.cursor.as_deref(),
                    q.limit.unwrap_or(32),
                )
                .map(Into::into)
        })
        .await?,
    ))
}
fn cleanup_job(s: &AppState, pid: &str, id: &str) -> domain::Result<()> {
    domain::validate_id(id)?;
    let root = s.store.directory(pid)?;
    let staging = root.join(".staging").join(id);
    if !staging.exists() {
        return Ok(());
    }
    if staging.canonicalize().map_err(domain::Error::io)? != staging {
        return Err(domain::Error::invalid("任务暂存不能指向其他目录"));
    }
    let entries = std::fs::read_dir(&staging)
        .map_err(domain::Error::io)?
        .take(129)
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(domain::Error::io)?;
    if entries.len() > 128 {
        return Err(domain::Error::invalid(
            "任务暂存包含过多未知文件，保留供检查",
        ));
    }
    for entry in &entries {
        if !entry.file_type().map_err(domain::Error::io)?.is_file()
            || entry.path().canonicalize().map_err(domain::Error::io)? != entry.path()
        {
            return Err(domain::Error::invalid(
                "任务暂存包含未知子目录或链接，保留供检查",
            ));
        }
    }
    for entry in entries {
        std::fs::remove_file(entry.path()).map_err(domain::Error::io)?;
    }
    std::fs::remove_dir(staging).map_err(domain::Error::io)
}
pub(super) fn release_files(s: &AppState, pid: &str, id: &str) -> domain::Result<domain::Artifact> {
    let item = s.store.release_artifact(pid, id)?;
    for file in &item.files {
        let path = crate::artifacts::controlled_path(&s.store, pid, &file.path)?;
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(domain::Error::new(
                    "ARTIFACT_CLEANUP_PENDING",
                    format!("成果已标记删除，部分文件暂未清理：{e}。可在管理面板重试清理。"),
                ));
            }
        }
    }
    Ok(item)
}
#[utoipa::path(post,path="/v1/projects/{project_id}/objects/{kind}/{object_id}/actions",params(("project_id"=String,Path),("kind"=String,Path),("object_id"=String,Path)),request_body=ObjectAction,responses((status=200,body=OkResponse)))]
pub(super) async fn action(
    State(s): State<AppState>,
    Path((pid, kind, id)): Path<(String, String, String)>,
    Body(body): Body<ObjectAction>,
) -> ApiResult<OkResponse> {
    Ok(Json(blocking(move||{
        let _lease=s.store.operation_lease(&pid)?;
        let kind=domain::ObjectKind::parse(&kind)?;
        match body.action {
            ObjectActionKind::Remove=>{
                if matches!(kind,domain::ObjectKind::Artifact|domain::ObjectKind::QueryResult) {
                    let value=s.store.object_details(&pid,kind,&id)?;
                    if value.object.revision!=body.expected_revision{return Err(domain::Error::new("REVISION_CONFLICT","对象刚被修改，请刷新详情"));}
                    if let Some(reason)=value.remove_reason{return Err(domain::Error::new("OBJECT_IN_USE",reason));}
                    if kind==domain::ObjectKind::Artifact {release_files(&s,&pid,&id)?;} else {let result=s.store.release_result(&pid,&id)?; query_views::release_versions(&s,&pid,&result)?;}
                } else {
                    s.store.remove_object(&pid,kind,&id,body.expected_revision)?;
                    if kind==domain::ObjectKind::Job {query_views::release_job(&s,&pid,&id)?;}
                    if kind==domain::ObjectKind::Job {cleanup_job(&s,&pid,&id).map_err(|error|domain::Error::new("JOB_CLEANUP_PENDING",format!("任务记录已清理，部分暂存暂未删除：{}。可在管理面板重试清理。",error.message)))?;}
                }
            },
            ObjectActionKind::Archive|ObjectActionKind::Unarchive=>{
                if kind!=domain::ObjectKind::Job{return Err(domain::Error::invalid("只有任务记录支持归档"));}
                s.store.archive_job(&pid,&id,matches!(body.action,ObjectActionKind::Archive),body.expected_revision)?;
            },
            ObjectActionKind::Reconnect=>{
                if kind!=domain::ObjectKind::Source{return Err(domain::Error::invalid("只有数据湖支持重新关联"));}
                s.store.restore_source(&pid,&id,body.expected_revision)?;
            },
        }
        Ok(OkResponse{ok:true})
    }).await?))
}
#[utoipa::path(get,path="/v1/projects/{project_id}/selection/history",params(("project_id"=String,Path)),responses((status=200,body=HistoryStatus)))]
pub(super) async fn history(
    State(s): State<AppState>,
    Path(pid): Path<String>,
) -> ApiResult<HistoryStatus> {
    Ok(Json(
        blocking(move || s.store.selection_history(&pid).map(Into::into)).await?,
    ))
}
#[utoipa::path(post,path="/v1/projects/{project_id}/selection/history",params(("project_id"=String,Path)),request_body=HistoryAction,responses((status=200,body=HistoryStatus)))]
pub(super) async fn restore(
    State(s): State<AppState>,
    Path(pid): Path<String>,
    Body(body): Body<HistoryAction>,
) -> ApiResult<HistoryStatus> {
    Ok(Json(
        blocking(move || {
            let _lease = s.store.operation_lease(&pid)?;
            match body.action {
                HistoryActionKind::Clear => s
                    .store
                    .clear_selection_history(&pid, body.expected_revision),
                HistoryActionKind::Undo | HistoryActionKind::Redo => s.store.restore_selection(
                    &pid,
                    body.expected_revision,
                    matches!(body.action, HistoryActionKind::Redo),
                ),
            }
            .map(Into::into)
        })
        .await?,
    ))
}
#[utoipa::path(get,path="/v1/settings/editing",responses((status=200,body=EditingSettings)))]
pub(super) async fn editing(State(s): State<AppState>) -> ApiResult<EditingSettings> {
    Ok(Json(
        blocking(move || s.store.editing_settings().map(Into::into)).await?,
    ))
}
#[utoipa::path(put,path="/v1/settings/editing",request_body=ConfigureEditing,responses((status=200,body=EditingSettings)))]
pub(super) async fn configure_editing(
    State(s): State<AppState>,
    Body(body): Body<ConfigureEditing>,
) -> ApiResult<EditingSettings> {
    Ok(Json(
        blocking(move || {
            s.store
                .configure_editing(body.undo_limit, body.expected_revision)
                .map(Into::into)
        })
        .await?,
    ))
}
#[derive(Deserialize)]
pub(super) struct PresetParams {
    operator_id: String,
    cursor: Option<String>,
    limit: Option<usize>,
}
#[utoipa::path(get,path="/v1/projects/{project_id}/presets",params(("project_id"=String,Path),("operator_id"=String,Query),("cursor"=Option<String>,Query),("limit"=Option<usize>,Query)),responses((status=200,body=PresetPage)))]
pub(super) async fn presets(
    State(s): State<AppState>,
    Path(pid): Path<String>,
    Query(q): Query<PresetParams>,
) -> ApiResult<PresetPage> {
    Ok(Json(
        blocking(move || {
            s.store
                .tool_presets(
                    &pid,
                    &q.operator_id,
                    q.cursor.as_deref(),
                    q.limit.unwrap_or(32),
                )
                .map(Into::into)
        })
        .await?,
    ))
}
#[utoipa::path(post,path="/v1/projects/{project_id}/presets",params(("project_id"=String,Path)),request_body=SaveToolPreset,responses((status=200,body=ToolPreset)))]
pub(super) async fn save_preset(
    State(s): State<AppState>,
    Path(pid): Path<String>,
    Body(body): Body<SaveToolPreset>,
) -> ApiResult<ToolPreset> {
    Ok(Json(
        blocking(move || {
            let _lease = s.store.operation_lease(&pid)?;
            let run = studio_operators::registry()?.normalize(body.run.into())?;
            s.store
                .save_tool_preset(
                    &pid,
                    body.id.as_deref(),
                    &body.name,
                    &body.notes,
                    body.expected_revision,
                    run,
                )
                .map(Into::into)
        })
        .await?,
    ))
}
#[utoipa::path(post,path="/v1/projects/{project_id}/presets/{preset_id}/delete",params(("project_id"=String,Path),("preset_id"=String,Path)),request_body=DeleteToolPreset,responses((status=200,body=OkResponse)))]
pub(super) async fn delete_preset(
    State(s): State<AppState>,
    Path((pid, id)): Path<(String, String)>,
    Body(body): Body<DeleteToolPreset>,
) -> ApiResult<OkResponse> {
    Ok(Json(
        blocking(move || {
            let _lease = s.store.operation_lease(&pid)?;
            s.store
                .delete_tool_preset(&pid, &id, body.expected_revision)?;
            Ok(OkResponse { ok: true })
        })
        .await?,
    ))
}
#[utoipa::path(post,path="/v1/projects/{project_id}/objects/{kind}/{object_id}/reveal",params(("project_id"=String,Path),("kind"=String,Path),("object_id"=String,Path)),request_body=RevealObject,responses((status=200,body=RevealedLocation)))]
pub(super) async fn reveal(
    State(s): State<AppState>,
    Path((pid, kind, id)): Path<(String, String, String)>,
    Body(body): Body<RevealObject>,
) -> ApiResult<RevealedLocation> {
    Ok(Json(
        blocking(move || {
            let kind = domain::ObjectKind::parse(&kind)?;
            let value = details(&s, &pid, kind, &id)?;
            let requested = value
                .paths
                .get(body.file_index)
                .ok_or_else(|| domain::Error::invalid("该对象没有此文件位置"))?;
            let mut path = std::path::PathBuf::from(requested);
            if kind == domain::ObjectKind::Artifact && !path.exists() {
                path = path
                    .parent()
                    .ok_or_else(|| domain::Error::invalid("成果目录无效"))?
                    .to_path_buf();
            }
            let path = path.canonicalize().map_err(|e| {
                domain::Error::new("LOCATION_UNAVAILABLE", format!("文件位置暂不可用：{e}"))
            })?;
            if body.open {
                open_location(&path)?;
            }
            Ok(RevealedLocation {
                path: studio_protocol::display_path(&path),
                opened: body.open,
            })
        })
        .await?,
    ))
}
fn open_location(path: &std::path::Path) -> domain::Result<()> {
    #[cfg(windows)]
    {
        let mut command = std::process::Command::new("explorer.exe");
        if path.is_file() {
            command.arg("/select,");
        }
        command.arg(studio_protocol::display_path(path));
        command.spawn().map_err(domain::Error::io)?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let directory = if path.is_dir() {
            path
        } else {
            path.parent()
                .ok_or_else(|| domain::Error::invalid("文件位置无效"))?
        };
        std::process::Command::new(if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        })
        .arg(directory)
        .spawn()
        .map_err(domain::Error::io)?;
        Ok(())
    }
}
