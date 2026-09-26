use super::*;
use studio_application::QueryAdapter;

#[utoipa::path(post,path="/v1/projects/{project_id}/source-requirements",params(("project_id"=String,Path)),request_body=SourceRequirementsRequest,responses((status=200,body=SourceRequirementsResult)))]
pub(super) async fn requirements(
    State(s): State<AppState>,
    Path(pid): Path<String>,
    Body(body): Body<SourceRequirementsRequest>,
) -> ApiResult<SourceRequirementsResult> {
    Ok(Json(
        blocking(move || {
            if body.projections.is_empty()
                || body.projections.len() > 16
                || body.projections.iter().any(|p| p.len() > 128)
            {
                return Err(domain::Error::invalid("需要 1–16 个有界投影名称"));
            }
            let scope: domain::ScopeRef = body.scope.into();
            scope.validate_project(&pid)?;
            let rows = s
                .store
                .scope_source_ids(&pid, &scope)?
                .into_iter()
                .map(|id| {
                    let source = s.store.source(&pid, &id)?;
                    let result = s.sources.descriptor(&source).and_then(|d| {
                        for p in &body.projections {
                            d.require_projection(p)?;
                        }
                        Ok(())
                    });
                    Ok(SourceRequirementStatus {
                        source_id: id,
                        name: source.name,
                        supported: result.is_ok(),
                        reason: result.err().map(|e| e.message),
                    })
                })
                .collect::<domain::Result<Vec<_>>>()?;
            Ok(SourceRequirementsResult {
                supported: !rows.is_empty() && rows.iter().all(|r| r.supported),
                sources: rows,
            })
        })
        .await?,
    ))
}

#[utoipa::path(get,path="/v1/source-adapters",responses((status=200,body=SourceRegistrations)))]
pub(super) async fn adapters(State(s): State<AppState>) -> ApiResult<SourceRegistrations> {
    Ok(Json(SourceRegistrations {
        items: s
            .sources
            .registrations()
            .into_iter()
            .map(|(kind, descriptor)| {
                let name = descriptor.display_name.clone();
                SourceRegistration {
                    kind,
                    name,
                    descriptor: descriptor.into(),
                }
            })
            .collect(),
    }))
}
#[utoipa::path(post,path="/v1/source-probes",request_body=ProbeSource,responses((status=200,body=SourcePreflight)))]
pub(super) async fn probe(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestReadContext>,
    Body(body): Body<ProbeSource>,
) -> ApiResult<SourcePreflight> {
    Ok(Json(
        blocking(move || {
            let mut source = domain::Source {
                id: String::new(),
                name: String::new(),
                kind: body.kind,
                index_root: body.index_root.map(Into::into),
                media_root: body.media_root.map(Into::into),
            };
            let read = read_permit(&s, domain::ReadClass::Index, &ctx)?;
            read.infer_kind(&mut source)?;
            let probe = read.probe(&source)?;
            source.id = probe.id.clone();
            let descriptor = s.sources.descriptor(&source)?;
            drop(read);
            let analysis_sequence = if descriptor.capabilities.raw_metadata {
                let read = read_permit(&s, domain::ReadClass::NativeQuery, &ctx)?;
                let version = read
                    .query(domain::METADATA_MEMORY_BYTES, false)
                    .read_version(&source, true)?;
                if version.catalog_revision != probe.revision {
                    return Err(domain::Error::new(
                        "SOURCE_CHANGED",
                        "预检期间来源版本发生变化，请重新检查",
                    ));
                }
                version.analysis_sequence
            } else {
                None
            };
            Ok(SourcePreflight {
                source_id: source.id,
                kind: source.kind,
                descriptor: descriptor.into(),
                revision: probe.revision,
                enumeration: probe.enumeration,
                count: probe.count.map(|n| n.to_string()),
                analysis_sequence,
            })
        })
        .await?,
    ))
}
