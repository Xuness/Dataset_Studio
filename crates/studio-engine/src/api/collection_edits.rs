use super::*;
use std::collections::HashMap;

#[utoipa::path(post,path="/v1/projects/{project_id}/collections/{collection_id}/members",operation_id="edit_collection_members",params(("project_id"=String,Path),("collection_id"=String,Path)),request_body=CollectionEdit,responses((status=200,body=CollectionEditResult)))]
pub(super) async fn edit(
    State(s): State<AppState>,
    Extension(context): Extension<RequestReadContext>,
    Path((pid, cid)): Path<(String, String)>,
    Body(body): Body<CollectionEdit>,
) -> ApiResult<CollectionEditResult> {
    Ok(Json(
        blocking(move || {
            let _lease = s.store.operation_lease(&pid)?;
            let request: domain::CollectionEdit = body.into();
            if let Some(receipt) = s.store.collection_edit_receipt(&pid, &cid, &request)? {
                return Ok(receipt.into());
            }
            let read = read_permit(&s, domain::ReadClass::NativeQuery, &context)?;
            let enrich = s
                .store
                .ranking_projection(
                    &pid,
                    &domain::ScopeRef {
                        project_id: pid.clone(),
                        target: domain::ScopeTarget::Workset {
                            collection_id: cid.clone(),
                            revision: Some(request.expected_revision),
                        },
                    },
                )?
                .is_some();
            if let domain::CollectionChange::Add {
                input: domain::CollectionMemberInput::Scope { scope },
            }
            | domain::CollectionChange::Remove {
                input: domain::CollectionMemberInput::Scope { scope },
            } = &request.change
            {
                query::validate_scope(&s, &pid, scope, &read)?;
            }
            let sources = s
                .store
                .sources(&pid)?
                .into_iter()
                .map(|s| (s.id.clone(), s))
                .collect::<BTreeMap<_, _>>();
            let mut versions = HashMap::new();
            let mut load =
                |keys: &[domain::AssetKey]| -> domain::Result<Vec<domain::RankingInput>> {
                    let mut rows = HashMap::new();
                    let mut groups = BTreeMap::<_, Vec<_>>::new();
                    for key in keys {
                        groups.entry(&key.source_id).or_default().push(key.clone());
                    }
                    for (sid, keys) in groups {
                        let source = sources.get(sid).ok_or_else(|| {
                            domain::Error::new("NOT_FOUND", "图片来源不属于当前项目")
                        })?;
                        if !enrich {
                            for value in read.freeze(source, &keys)? {
                                let key = value.asset.key;
                                rows.insert(
                                    key.clone(),
                                    domain::RankingInput {
                                        source_id: key.source_id,
                                        asset_id: key.asset_id,
                                        stored_extension: value.asset.extension,
                                        stored_bytes: value.asset.bytes,
                                        ..Default::default()
                                    },
                                );
                            }
                            continue;
                        }
                        let descriptor = s.sources.descriptor(source)?;
                        let query = read.query(domain::METADATA_MEMORY_BYTES, false);
                        if !versions.contains_key(sid) {
                            versions.insert(
                                sid.clone(),
                                query.read_version(source, descriptor.capabilities.metadata)?,
                            );
                        }
                        let version = versions.get(sid).expect("source version");
                        let frozen =
                            read.freeze_at(source, &keys, Some(&version.catalog_revision))?;
                        let mut ratings = HashMap::new();
                        if descriptor.capabilities.query
                            && query
                                .fields(source)?
                                .fields
                                .iter()
                                .any(|f| f.id == "rating")
                        {
                            for rating in ["g", "s", "q", "e"] {
                                let spec = domain::QuerySpec {
                                    version: 1,
                                    source_ids: vec![sid.clone()],
                                    conditions: vec![domain::QueryCondition {
                                        field: "rating".into(),
                                        operator: domain::QueryOperator::Eq,
                                        value: Some(domain::QueryValue::Text(rating.into())),
                                    }],
                                    observation_rule: domain::ObservationRule::CurrentPost,
                                    order: domain::QueryOrder::AssetKeyAsc,
                                    input_scope: None,
                                };
                                query.execute_query_keys(
                                    source,
                                    &spec,
                                    version,
                                    context.cancelled.clone(),
                                    &keys,
                                    &mut |matches, _| {
                                        for key in matches {
                                            ratings
                                                .entry(key.asset_id.clone())
                                                .and_modify(|v: &mut Option<String>| {
                                                    if v.as_deref() != Some(rating) {
                                                        *v = None;
                                                    }
                                                })
                                                .or_insert_with(|| Some(rating.into()));
                                        }
                                        Ok(())
                                    },
                                )?;
                            }
                        }
                        let summaries = if descriptor.capabilities.metadata {
                            match read.summaries_at(
                                source,
                                &keys.iter().map(|k| k.asset_id.clone()).collect::<Vec<_>>(),
                                Some(&version.catalog_revision),
                                context.cancelled.clone(),
                            ) {
                                Ok(rows) => rows,
                                Err(error) if error.code == "METADATA_UNSUPPORTED" => Vec::new(),
                                Err(error) => return Err(error),
                            }
                        } else {
                            Vec::new()
                        };
                        let posts = summaries
                            .into_iter()
                            .map(|s| {
                                (
                                    s.asset_id,
                                    s.post_ids
                                        .iter()
                                        .filter_map(|p| p.parse::<i64>().ok())
                                        .min(),
                                )
                            })
                            .collect::<HashMap<_, _>>();
                        for value in frozen {
                            let key = value.asset.key;
                            let row = domain::RankingInput {
                                source_id: key.source_id.clone(),
                                asset_id: key.asset_id.clone(),
                                post_id: posts.get(&key.asset_id).copied().flatten(),
                                rating: ratings.remove(&key.asset_id).flatten(),
                                stored_extension: value.asset.extension,
                                stored_bytes: value.asset.bytes,
                                ..Default::default()
                            };
                            rows.insert(key, row);
                        }
                    }
                    keys.iter()
                        .map(|key| {
                            rows.remove(key).ok_or_else(|| {
                                domain::Error::new("INPUT_INVALID", "图片批次缺少请求对象")
                            })
                        })
                        .collect()
                };
            s.store
                .edit_collection(&pid, &cid, &request, &mut load)
                .map(Into::into)
        })
        .await?,
    ))
}
