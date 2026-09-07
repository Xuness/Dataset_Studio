use super::*;

#[utoipa::path(post,path="/v1/projects/{project_id}/sources/{source_id}/relink",params(("project_id"=String,Path),("source_id"=String,Path)),request_body=RelinkSource,responses((status=200,body=SourceRelinked)))]
pub(super) async fn relink(
    State(s): State<AppState>,
    Path((pid, sid)): Path<(String, String)>,
    Body(body): Body<RelinkSource>,
) -> ApiResult<SourceRelinked> {
    Ok(Json(
        blocking(move || {
            let mut source = s.store.source(&pid, &sid)?;
            if source.kind != "danbooru" {
                return Err(domain::Error::invalid("此来源不需要重新关联本机位置"));
            }
            source.index_root = Some(body.index_root.into());
            source.media_root = Some(body.media_root.into());
            // Catalog checks both manifests against the existing logical ID before mutation.
            let probe = SourceRouter.probe(&source)?;
            if probe.id != sid {
                return Err(domain::Error::new(
                    "SOURCE_ID_MISMATCH",
                    "新位置不是同一个数据湖",
                ));
            }
            s.store.relink_source(&pid, source)?;
            Ok(SourceRelinked {
                source_id: sid,
                revision: probe.revision,
                impact: "all_projects_in_this_app_registry".into(),
            })
        })
        .await?,
    ))
}
