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
            // A registered writer must acknowledge the same roots before ordinary
            // relinking can succeed. Never silently split readers from publishers.
            if s.lake_updates.configured() {
                use domain::lake_updates::LakeUpdateOperation as Op;
                let values = s.lake_updates.execute(Op::Lakes, serde_json::json!({}))?;
                if values["items"]
                    .as_array()
                    .is_some_and(|rows| rows.iter().any(|r| r["id"] == sid))
                {
                    return Err(domain::Error::new(
                        "UPDATE_CONFLICT",
                        "此湖由更新服务管理；请在设置 → 数据湖 API → 迁移数据湖位置中协调读写位置",
                    ));
                }
            }
            if [&source.index_root, &Some(body.index_root.clone().into())]
                .iter()
                .any(|root| {
                    root.as_ref().is_some_and(|root| {
                        root.join("UPDATE-CONTROLLER.json").exists()
                            || root.join("LAKE-RELOCATION.json").exists()
                    })
                })
            {
                return Err(domain::Error::new(
                    "UPDATE_CONFLICT",
                    "请先恢复此湖的更新控制器，再完成协调迁移",
                ));
            }
            if !s.sources.has(&source, |c| c.relink) {
                return Err(domain::Error::invalid("此来源不需要重新关联本机位置"));
            }
            source.index_root = Some(body.index_root.into());
            source.media_root = Some(body.media_root.into());
            // Catalog checks both manifests against the existing logical ID before mutation.
            let probe = s.sources.validate_attachment(
                &mut source,
                studio_application::SourceReadContext::new(
                    std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    domain::ReadPriority::Interactive,
                ),
            )?;
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
