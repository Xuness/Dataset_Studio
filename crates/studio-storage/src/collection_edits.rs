//! Workset identity is stable; immutable bases and versioned point changes make
//! old inputs reproducible without copying the unchanged part of a workset.
use crate::*;

#[cfg(test)]
mod tests;

pub(super) fn read(db: &Connection, id: &str, revision: Option<u64>) -> Result<Collection> {
    validate_id(id)?;
    let mut item = db.query_row(
        "SELECT c.name,c.count,coalesce(s.revision,0) FROM collections c LEFT JOIN collection_membership_state s ON s.collection_id=c.id WHERE c.id=?1 AND NOT EXISTS(SELECT 1 FROM evaluation_workset_builds b WHERE b.collection_id=c.id AND b.state!='ready')",
        [id], |r| Ok(Collection { id: id.into(), name: r.get(0)?, count: unsigned(r,1)?, revision: unsigned(r,2)? }),
    ).optional().map_err(db_error)?.ok_or_else(|| Error::new("NOT_FOUND", "工作集不属于当前项目或尚未就绪"))?;
    item.name = management::display_name(db, "workset", id, &item.name)?;
    if let Some(revision) = revision {
        if revision > item.revision || revision > i64::MAX as u64 {
            return Err(Error::new(
                "REVISION_CONFLICT",
                "工作集成员版本不存在，请刷新后重试",
            ));
        }
        if revision != item.revision {
            item.count = db
                .query_row(
                    "SELECT count FROM collection_versions WHERE collection_id=?1 AND revision=?2",
                    params![id, revision as i64],
                    |r| unsigned(r, 0),
                )
                .optional()
                .map_err(db_error)?
                .ok_or_else(|| Error::new("REVISION_CONFLICT", "工作集成员版本不可用"))?;
            item.revision = revision;
        }
    }
    Ok(item)
}

pub(super) fn pin(db: &Connection, pid: &str, scope: &ScopeRef) -> Result<ScopeRef> {
    scope.validate_project(pid)?;
    let mut pinned = scope.clone();
    if let ScopeTarget::Workset {
        collection_id,
        revision,
    } = &mut pinned.target
    {
        *revision = Some(read(db, collection_id, *revision)?.revision);
    }
    Ok(pinned)
}

pub(super) fn pin_spec(db: &Connection, pid: &str, spec: &QuerySpec) -> Result<QuerySpec> {
    let mut spec = spec.clone();
    spec.input_scope = spec
        .input_scope
        .as_ref()
        .map(|s| pin(db, pid, s))
        .transpose()?;
    Ok(spec)
}

/// IDs are validated by the scope/read boundary. Numeric revisions are not SQL
/// parameters so the outer keyset predicate can use its own bounded parameters.
pub(super) fn members_sql(id: &str, revision: u64) -> String {
    format!("SELECT m.source_id,m.asset_id FROM collection_base_members m WHERE m.collection_id='{id}'
      AND NOT EXISTS(SELECT 1 FROM collection_member_changes e WHERE e.collection_id='{id}' AND e.source_id=m.source_id AND e.asset_id=m.asset_id AND e.valid_from<={revision} AND (e.valid_until IS NULL OR e.valid_until>{revision}))
      UNION ALL SELECT e.source_id,e.asset_id FROM collection_member_changes e WHERE e.collection_id='{id}' AND e.present=1 AND e.valid_from<={revision} AND (e.valid_until IS NULL OR e.valid_until>{revision})")
}

/// A job/selection with a base copies only overrides, retaining its own frozen
/// member relation when subsequent workset edits publish another revision.
pub(super) fn overrides_sql(id: &str, revision: u64, included: bool) -> String {
    let initial = if included {
        "collection_inclusions"
    } else {
        "collection_exclusions"
    };
    let present = u8::from(included);
    let exists = if included { "NOT EXISTS" } else { "EXISTS" };
    format!("SELECT i.source_id,i.asset_id FROM {initial} i WHERE i.collection_id='{id}'
      AND NOT EXISTS(SELECT 1 FROM collection_member_changes e WHERE e.collection_id='{id}' AND e.source_id=i.source_id AND e.asset_id=i.asset_id AND e.valid_from<={revision} AND (e.valid_until IS NULL OR e.valid_until>{revision}))
      UNION ALL SELECT e.source_id,e.asset_id FROM collection_member_changes e WHERE e.collection_id='{id}' AND e.present={present} AND e.valid_from<={revision} AND (e.valid_until IS NULL OR e.valid_until>{revision})
      AND {exists}(SELECT 1 FROM collection_bases b JOIN result_members m ON m.result_id=b.result_id WHERE b.collection_id='{id}' AND m.source_id=e.source_id AND m.asset_id=e.asset_id)")
}

fn shared_base_add(
    db: &Connection,
    pid: &str,
    id: &str,
    revision: u64,
    scope: &ScopeRef,
    input_sql: &str,
    current_sql: &str,
) -> Result<bool> {
    let target: Option<String> = db
        .query_row(
            "SELECT result_id FROM collection_bases WHERE collection_id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_error)?;
    let Some(target) = target else {
        return Ok(false);
    };
    let (base, extras): (Option<String>, String) = match &scope.target {
        ScopeTarget::QueryResult { result_id } => (
            Some(result_id.clone()),
            "SELECT '' AS source_id,'' AS asset_id WHERE 0".into(),
        ),
        ScopeTarget::Selection { .. } => (
            db.query_row(
                "SELECT result_id FROM selection_base WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?,
            "SELECT source_id,asset_id FROM selection".into(),
        ),
        ScopeTarget::Workset {
            collection_id,
            revision,
        } => (
            db.query_row(
                "SELECT result_id FROM collection_bases WHERE collection_id=?1",
                [collection_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?,
            overrides_sql(collection_id, revision.expect("pinned scope"), true),
        ),
        _ => return Ok(false),
    };
    let Some(base) = base else {
        return Ok(false);
    };
    if base != target {
        let recipe = |rid: String| {
            ranking_memberships::resolve(
                db,
                pid,
                &ScopeRef {
                    project_id: pid.into(),
                    target: ScopeTarget::QueryResult { result_id: rid },
                },
            )
        };
        let (Some(target), Some(input)) = (recipe(target)?, recipe(base)?) else {
            return Ok(false);
        };
        let mut filter = target.filter.clone();
        filter.order = RankingOrder::Main;
        if !target.edits.is_empty()
            || target.member_result.is_some()
            || target.ratings.is_some()
            || filter
                != (RankingFilter {
                    order: RankingOrder::Main,
                    ..Default::default()
                })
            || !input.edits.is_empty()
            || input.member_result.is_some()
            || target.artifact_id != input.artifact_id
            || target.input_file != input.input_file
            || target.score_file != input.score_file
        {
            return Ok(false);
        }
    }
    // The input base is already covered. Only target exclusions and explicit
    // input additions can change anything, even for a ten-million-row base.
    let missing = overrides_sql(id, revision, false);
    db.execute(&format!("INSERT OR IGNORE INTO temp.collection_edit_candidates SELECT s.source_id,s.asset_id,1 FROM ({missing}) s WHERE EXISTS(SELECT 1 FROM ({input_sql}) m WHERE m.source_id=s.source_id AND m.asset_id=s.asset_id)"),[]).map_err(db_error)?;
    db.execute(&format!("INSERT OR IGNORE INTO temp.collection_edit_candidates SELECT s.source_id,s.asset_id,1 FROM ({extras}) s WHERE NOT EXISTS(SELECT 1 FROM ({current_sql}) m WHERE m.source_id=s.source_id AND m.asset_id=s.asset_id)"),[]).map_err(db_error)?;
    Ok(true)
}

impl SqliteStore {
    pub fn collection_edit_receipt(
        &self,
        pid: &str,
        id: &str,
        request: &CollectionEdit,
    ) -> Result<Option<CollectionEditResult>> {
        validate_id(id)?;
        validate_id(&request.request_id)?;
        let p = self.handle(pid)?;
        let db = p.read()?;
        let row:Option<(String,String)>=db.query_row("SELECT request_json,response_json FROM collection_edit_requests WHERE request_id=?1",[&request.request_id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(db_error)?;
        row.map(|(old, response)| {
            if old != serde_json::to_string(&(id, request)).map_err(Error::io)? {
                return Err(Error::new(
                    "IDEMPOTENCY_CONFLICT",
                    "此请求标识已用于其他工作集编辑",
                ));
            }
            serde_json::from_str(&response).map_err(Error::io)
        })
        .transpose()
    }
    pub fn pin_scope(&self, pid: &str, scope: &ScopeRef) -> Result<ScopeRef> {
        let p = self.handle(pid)?;
        pin(&*p.read()?, pid, scope)
    }

    pub fn collection(&self, pid: &str, id: &str) -> Result<Collection> {
        let p = self.handle(pid)?;
        read(&*p.read()?, id, None)
    }

    pub fn edit_collection(
        &self,
        pid: &str,
        id: &str,
        request: &CollectionEdit,
        metadata: &mut dyn FnMut(&[AssetKey]) -> Result<Vec<RankingInput>>,
    ) -> Result<CollectionEditResult> {
        validate_id(id)?;
        validate_id(&request.request_id)?;
        if request.expected_revision >= i64::MAX as u64 {
            return Err(Error::invalid("工作集成员版本无效"));
        }
        let request_json = serde_json::to_string(&(id, request)).map_err(Error::io)?;
        let p = self.handle(pid)?;
        let operation = self.begin_member_write(pid, &request.request_id)?;
        let cancelled = operation.cancelled();
        let mut db = p.write_cancelled(&cancelled)?;
        let flag = cancelled.clone();
        db.progress_handler(1000, Some(move || flag.load(Ordering::Acquire)))
            .map_err(db_error)?;
        let outcome = (|| {
            let tx = db.project_transaction().map_err(db_error)?;
            let previous: Option<(String,String)> = tx.query_row("SELECT request_json,response_json FROM collection_edit_requests WHERE request_id=?1", [&request.request_id], |r| Ok((r.get(0)?,r.get(1)?)))
                .optional().map_err(db_error)?;
            if let Some((old, response)) = previous {
                if old != request_json {
                    return Err(Error::new(
                        "IDEMPOTENCY_CONFLICT",
                        "此请求标识已用于其他工作集编辑",
                    ));
                }
                return serde_json::from_str(&response).map_err(Error::io);
            }
            let old = read(&tx, id, None)?;
            if old.revision != request.expected_revision {
                return Err(Error::new(
                    "REVISION_CONFLICT",
                    "工作集成员已被修改，请刷新后重试",
                ));
            }
            let scope = ScopeRef {
                project_id: pid.into(),
                target: ScopeTarget::Workset {
                    collection_id: id.into(),
                    revision: Some(old.revision),
                },
            };
            let current_sql = members_sql(id, old.revision);
            tx.execute_batch("DROP TABLE IF EXISTS temp.collection_edit_candidates; CREATE TEMP TABLE collection_edit_candidates(source_id TEXT,asset_id TEXT,present INTEGER NOT NULL,PRIMARY KEY(source_id,asset_id)) WITHOUT ROWID;")
                .map_err(db_error)?;
            let mut additions = false;
            let requested = match &request.change {
                CollectionChange::Add { input } | CollectionChange::Remove { input } => {
                    additions = matches!(request.change, CollectionChange::Add { .. });
                    let exists = if additions { "NOT EXISTS" } else { "EXISTS" };
                    let present = u8::from(additions);
                    match input {
                        CollectionMemberInput::Scope { scope } => {
                            let scope = pin(&tx, pid, scope)?;
                            let resolved = scopes::resolve(&tx, pid, &scope)?;
                            let same = matches!(&scope.target,ScopeTarget::Workset { collection_id,revision } if collection_id==id && *revision==Some(old.revision));
                            if !(additions
                                && (same
                                    || shared_base_add(
                                        &tx,
                                        pid,
                                        id,
                                        old.revision,
                                        &scope,
                                        &resolved.sql,
                                        &current_sql,
                                    )?))
                            {
                                tx.execute(&format!("INSERT INTO temp.collection_edit_candidates SELECT s.source_id,s.asset_id,{present} FROM ({}) s WHERE {exists}(SELECT 1 FROM ({current_sql}) m WHERE m.source_id=s.source_id AND m.asset_id=s.asset_id)", resolved.sql), []).map_err(db_error)?;
                            }
                            resolved.count
                        }
                        CollectionMemberInput::Keys { keys } => {
                            if keys.is_empty() || keys.len() > 512 {
                                return Err(Error::invalid(
                                    "单次图片编辑需要 1–512 个对象；批量编辑请使用范围引用",
                                ));
                            }
                            let mut seen = std::collections::BTreeSet::new();
                            for key in keys {
                                validate_id(&key.source_id)?;
                                if key.asset_id.is_empty()
                                    || key.asset_id.len() > 128
                                    || key.asset_id.contains('\0')
                                {
                                    return Err(Error::invalid("图片身份无效"));
                                }
                                if !tx
                                    .query_row(
                                        "SELECT EXISTS(SELECT 1 FROM sources WHERE id=?1)",
                                        [&key.source_id],
                                        |r| r.get::<_, bool>(0),
                                    )
                                    .map_err(db_error)?
                                {
                                    return Err(Error::new("NOT_FOUND", "图片来源不属于当前项目"));
                                }
                                seen.insert((&key.source_id, &key.asset_id));
                                tx.execute(&format!("INSERT OR IGNORE INTO temp.collection_edit_candidates SELECT ?1,?2,{present} WHERE {exists}(SELECT 1 FROM ({current_sql}) m WHERE m.source_id=?1 AND m.asset_id=?2)"), params![key.source_id,key.asset_id]).map_err(db_error)?;
                            }
                            seen.len() as u64
                        }
                    }
                }
                CollectionChange::Restore { revision } => {
                    read(&tx, id, Some(*revision))?;
                    let target = members_sql(id, *revision);
                    tx.execute(&format!("INSERT INTO temp.collection_edit_candidates SELECT c.source_id,c.asset_id,EXISTS(SELECT 1 FROM ({target}) m WHERE m.source_id=c.source_id AND m.asset_id=c.asset_id) FROM (SELECT DISTINCT source_id,asset_id FROM collection_member_changes WHERE collection_id=?1 AND valid_from>?2) c WHERE EXISTS(SELECT 1 FROM ({target}) m WHERE m.source_id=c.source_id AND m.asset_id=c.asset_id) != EXISTS(SELECT 1 FROM ({current_sql}) m WHERE m.source_id=c.source_id AND m.asset_id=c.asset_id)"), params![id,*revision as i64]).map_err(db_error)?;
                    tx.query_row(
                        "SELECT count(*) FROM temp.collection_edit_candidates",
                        [],
                        |r| unsigned(r, 0),
                    )
                    .map_err(db_error)?
                }
            };
            studio_application::read_cancelled(&cancelled)?;
            let (changed, added): (u64, u64) = tx
                .query_row(
                    "SELECT count(*),coalesce(sum(present),0) FROM temp.collection_edit_candidates",
                    [],
                    |r| Ok((unsigned(r, 0)?, unsigned(r, 1)?)),
                )
                .map_err(db_error)?;
            let mut result = CollectionEditResult {
                collection: old.clone(),
                previous_revision: old.revision,
                requested,
                changed,
            };
            if changed != 0 {
                let revision = old.revision + 1;
                let count = old
                    .count
                    .checked_add(added)
                    .and_then(|v| v.checked_sub(changed - added))
                    .filter(|v| *v <= i64::MAX as u64)
                    .ok_or_else(|| Error::new("DATABASE_ERROR", "工作集成员数量无效"))?;
                let projection = ranking_memberships::resolve(&tx, pid, &scope)?;
                let materials = projection
                    .as_ref()
                    .map(|recipe| -> Result<_> {
                        let (input, scores) = recipe.files(&p.project.directory)?;
                        let input = ranking_tables::RankingInputTable::open(&input)?;
                        let scores = ranking_tables::RankingResultTable::open(&scores)?;
                        Ok((input, scores))
                    })
                    .transpose()?;
                let inherited = projection
                    .as_ref()
                    .filter(|r| !r.edits.is_empty())
                    .map(|r| {
                        ranking_projection::RankingProjectionReader::open(
                            &p.project.directory,
                            r,
                            cancelled.clone(),
                        )
                    })
                    .transpose()?;
                tx.execute(
                    "INSERT OR IGNORE INTO collection_versions VALUES(?1,?2,?3,?4)",
                    params![id, old.revision as i64, old.count as i64, now()],
                )
                .map_err(db_error)?;
                operation.update(0, Some(changed));
                let mut after = (String::new(), String::new());
                let mut completed = 0;
                loop {
                    studio_application::read_cancelled(&cancelled)?;
                    let rows = tx.prepare("SELECT source_id,asset_id,present FROM temp.collection_edit_candidates WHERE (source_id,asset_id)>(?1,?2) ORDER BY source_id,asset_id LIMIT 128").map_err(db_error)?
                        .query_map(params![after.0,after.1], |r| Ok((AssetKey { source_id:r.get(0)?,asset_id:r.get(1)? }, r.get::<_,bool>(2)?))).map_err(db_error)?
                        .collect::<std::result::Result<Vec<_>,_>>().map_err(db_error)?;
                    let Some((last, _)) = rows.last() else { break };
                    after = (last.source_id.clone(), last.asset_id.clone());
                    let mut fetched = HashMap::new();
                    let mut missing = Vec::new();
                    for (key, present) in &rows {
                        if !present {
                            continue;
                        }
                        let saved: Option<String> = tx.query_row("SELECT input_json FROM collection_member_changes WHERE collection_id=?1 AND source_id=?2 AND asset_id=?3 AND present=1 AND input_json IS NOT NULL ORDER BY valid_from DESC LIMIT 1", params![id,key.source_id,key.asset_id], |r| r.get(0)).optional().map_err(db_error)?;
                        if let Some(raw) = saved {
                            fetched.insert(
                                key.clone(),
                                serde_json::from_str::<RankingInput>(&raw).map_err(Error::io)?,
                            );
                        } else if let Some((input, _)) = &materials
                            && let Some(ordinal) = input.ordinal_for_key(key)?
                        {
                            fetched.insert(key.clone(), input.row(ordinal)?);
                        } else if let Some(reader) = &inherited
                            && let Some(row) = reader.edited_input_for_key(key)?
                        {
                            fetched.insert(key.clone(), row);
                        } else {
                            missing.push(key.clone());
                        }
                    }
                    // Restoring a version reuses its durable identity/metadata.
                    // New additions validate source identity in bounded batches.
                    if !missing.is_empty() && (materials.is_some() || additions) {
                        let values = metadata(&missing)?;
                        if values.len() != missing.len()
                            || values
                                .iter()
                                .zip(&missing)
                                .any(|(row, key)| row.key() != *key)
                        {
                            return Err(Error::new("INPUT_INVALID", "图片批次与请求身份不一致"));
                        }
                        fetched.extend(values.into_iter().map(|row| (row.key(), row)));
                    }
                    for (key, present) in &rows {
                        tx.execute("UPDATE collection_member_changes SET valid_until=?4 WHERE collection_id=?1 AND source_id=?2 AND asset_id=?3 AND valid_until IS NULL", params![id,key.source_id,key.asset_id,revision as i64]).map_err(db_error)?;
                        tx.execute("INSERT INTO collection_member_changes(collection_id,source_id,asset_id,valid_from,present) VALUES(?1,?2,?3,?4,?5)", params![id,key.source_id,key.asset_id,revision as i64,present]).map_err(db_error)?;
                        let change_id = tx.last_insert_rowid();
                        if let Some((input, scores)) = &materials {
                            let original = input.ordinal_for_key(key)?;
                            let ordinal = original.unwrap_or((1u64 << 62) + change_id as u64);
                            if let Some(mut row) = fetched.remove(key) {
                                row.ordinal = ordinal;
                                let score = if let Some(original) = original {
                                    scores.row(original)?
                                } else {
                                    RankingScores {
                                        ordinal,
                                        rating: row.rating.clone(),
                                        eligibility: RankingEligibility::MetadataUnavailable,
                                        missing_flags: vec!["not_in_ranking_input".into()],
                                        time_reason: "not_ranked".into(),
                                        ..Default::default()
                                    }
                                };
                                let direct = score.v2.as_ref().map(|s| s.direct_rank);
                                let fused = score.v2.as_ref().map(|s| s.fused_rank);
                                tx.execute("UPDATE collection_member_changes SET ordinal=?2,input_json=?3,scores_json=?4,rating=?5,post_id=?6,main_rank=?7,rescue_rank=?8,direct_rank=?9,fused_rank=?10 WHERE change_id=?1", params![change_id,ordinal as i64,serde_json::to_string(&row).map_err(Error::io)?,serde_json::to_string(&score).map_err(Error::io)?,score.rating,row.post_id,score.main_rank.map(|v|v as i64),score.rescue_rank.map(|v|v as i64),direct.map(|v|v as i64),fused.map(|v|v as i64)]).map_err(db_error)?;
                            } else {
                                tx.execute("UPDATE collection_member_changes SET ordinal=?2 WHERE change_id=?1", params![change_id,ordinal as i64]).map_err(db_error)?;
                            }
                        }
                    }
                    completed += rows.len() as u64;
                    operation.update(completed, Some(changed));
                }
                tx.execute("INSERT INTO collection_membership_state VALUES(?1,?2) ON CONFLICT(collection_id) DO UPDATE SET revision=excluded.revision", params![id,revision as i64]).map_err(db_error)?;
                tx.execute(
                    "INSERT INTO collection_versions VALUES(?1,?2,?3,?4)",
                    params![id, revision as i64, count as i64, now()],
                )
                .map_err(db_error)?;
                tx.execute(
                    "UPDATE collections SET count=?2 WHERE id=?1",
                    params![id, count as i64],
                )
                .map_err(db_error)?;
                if projection.is_some() {
                    ranking_memberships::resolve(
                        &tx,
                        pid,
                        &ScopeRef {
                            project_id: pid.into(),
                            target: ScopeTarget::Workset {
                                collection_id: id.into(),
                                revision: Some(revision),
                            },
                        },
                    )?
                    .ok_or_else(|| Error::new("ARTIFACT_INVALID", "工作集的排名成员描述不可用"))?;
                }
                event(&tx, "collection.members.changed", id)?;
                result.collection.count = count;
                result.collection.revision = revision;
            }
            tx.execute(
                "INSERT INTO collection_edit_requests VALUES(?1,?2,?3,?4)",
                params![
                    request.request_id,
                    id,
                    request_json,
                    serde_json::to_string(&result).map_err(Error::io)?
                ],
            )
            .map_err(db_error)?;
            tx.execute_batch("DROP TABLE temp.collection_edit_candidates;")
                .map_err(db_error)?;
            studio_application::read_cancelled(&cancelled)?;
            tx.commit().map_err(db_error)?;
            Ok(result)
        })();
        db.progress_handler(0, None::<fn() -> bool>)
            .map_err(db_error)?;
        operation.finish(outcome)
    }
}
