use super::*;

#[utoipa::path(get,path="/v1/projects/{project_id}/sources/{source_id}/assets/{asset_id}/original",params(("project_id"=String,Path),("source_id"=String,Path),("asset_id"=String,Path)),responses((status=200,description="Original stored bytes, at most 64 MiB",content_type="application/octet-stream")))]
pub(super) async fn original(
    State(s): State<AppState>,
    Extension(read_context): Extension<RequestReadContext>,
    Path((pid, sid, aid)): Path<(String, String, String)>,
) -> std::result::Result<Response, Failure> {
    let media = blocking(move || {
        let source = s.store.source(&pid, &sid)?;
        crate::exports::read_original(
            &s.sources,
            &source,
            &aid,
            read_context.priority,
            read_context.cancelled,
        )
    })
    .await?;
    Ok((
        [
            ("content-type", media.content_type),
            ("cache-control", "no-store".into()),
        ],
        media.bytes,
    )
        .into_response())
}

#[utoipa::path(post,path="/v1/projects/{project_id}/sources/{source_id}/assets/{asset_id}/original/save",params(("project_id"=String,Path),("source_id"=String,Path),("asset_id"=String,Path)),request_body=SaveOriginal,responses((status=200,body=SavedOriginal)))]
pub(super) async fn save_original(
    State(s): State<AppState>,
    Extension(read_context): Extension<RequestReadContext>,
    Path((pid, sid, aid)): Path<(String, String, String)>,
    Body(body): Body<SaveOriginal>,
) -> ApiResult<SavedOriginal> {
    Ok(Json(
        blocking(move || {
            let source = s.store.source(&pid, &sid)?;
            let path = crate::exports::validate_save_path(&s.store, &body.path)?;
            let media = crate::exports::read_original(
                &s.sources,
                &source,
                &aid,
                read_context.priority,
                read_context.cancelled,
            )?;
            crate::exports::write_file(&path, &media.bytes)?;
            Ok(SavedOriginal {
                path: studio_protocol::display_path(&path),
                bytes: media.bytes.len() as u64,
            })
        })
        .await?,
    ))
}

#[utoipa::path(post,path="/v1/projects/{project_id}/jobs/{job_id}/export/reveal",operation_id="reveal_export",params(("project_id"=String,Path),("job_id"=String,Path)),responses((status=200,body=RevealedLocation)))]
pub(super) async fn reveal_export(
    State(s): State<AppState>,
    Path((pid, jid)): Path<(String, String)>,
) -> ApiResult<RevealedLocation> {
    Ok(Json(
        blocking(move || {
            let run = s.store.job_run(&pid, &jid)?.run;
            if !crate::exports::is_export(&run.operator_id) {
                return Err(domain::Error::invalid("该任务不是文件导出"));
            }
            let params = studio_operators::export::ExportParameters::parse(&run.parameters)?;
            let path = std::path::Path::new(&params.destination)
                .canonicalize()
                .map_err(|e| {
                    domain::Error::new("LOCATION_UNAVAILABLE", format!("导出文件夹暂不可用：{e}"))
                })?;
            management::open_location(&path)?;
            Ok(RevealedLocation {
                path: studio_protocol::display_path(&path),
                opened: true,
            })
        })
        .await?,
    ))
}
