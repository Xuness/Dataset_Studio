use crate::*;

impl SqliteStore {
    /// Follow only the explicit, fixed input scope. Other artifact references do
    /// not imply that their scores define the order of this member set.
    pub fn ranked_scope(&self, pid: &str, scope: &ScopeRef) -> Result<Option<RankedScope>> {
        scope.validate_project(pid)?;
        let project = self.handle(pid)?;
        let db = project.db.lock().map_err(lock_error)?;
        let mut target = scope.target.clone();
        let mut count = None;
        for _ in 0..16 {
            match target {
                ScopeTarget::QueryResult { result_id } => {
                    let result = query::ready_result(&db, pid, &result_id)?;
                    count.get_or_insert(result.count.unwrap_or(0));
                    let Some(parent) = result.spec.input_scope else {
                        return Ok(None);
                    };
                    parent.validate_project(pid)?;
                    target = parent.target;
                }
                ScopeTarget::Workset { collection_id } => {
                    let row: Option<(u64, String)> = db
                        .query_row(
                            "SELECT c.count,s.provenance_json FROM collections c JOIN collection_scopes s ON s.collection_id=c.id WHERE c.id=?1",
                            [&collection_id],
                            |r| Ok((unsigned(r, 0)?, r.get(1)?)),
                        )
                        .optional()
                        .map_err(db_error)?;
                    let Some((members, raw)) = row else {
                        return Ok(None);
                    };
                    let provenance: serde_json::Value =
                        serde_json::from_str(&raw).map_err(Error::io)?;
                    let Some(aid) = provenance.get("ranking_artifact").and_then(|v| v.as_str())
                    else {
                        return Ok(None);
                    };
                    validate_id(aid)?;
                    let linked: bool = db
                        .query_row(
                            "SELECT EXISTS(SELECT 1 FROM artifact_references WHERE owner_kind='collection' AND owner_id=?1 AND artifact_id=?2)",
                            params![collection_id, aid],
                            |r| r.get(0),
                        )
                        .map_err(db_error)?;
                    if !linked {
                        return Err(Error::new("ARTIFACT_INVALID", "工作集缺少原排名成果引用"));
                    }
                    let artifact = artifacts::read(&db, pid, aid)?;
                    if artifact.kind != RANKING_KIND || artifact.state != ArtifactState::Ready {
                        return Err(Error::new("ARTIFACT_NOT_READY", "工作集的排名成果已不可用"));
                    }
                    let saved_filter: RankingFilter =
                        serde_json::from_value(provenance.get("filter").cloned().ok_or_else(
                            || Error::new("ARTIFACT_INVALID", "工作集缺少保存时的排名条件"),
                        )?)
                        .map_err(Error::io)?;
                    saved_filter.validate()?;
                    return Ok(Some(RankedScope {
                        workset_id: collection_id,
                        artifact_id: aid.into(),
                        artifact_name: artifact.name,
                        count: count.unwrap_or(members),
                        saved_filter,
                    }));
                }
                _ => return Ok(None),
            }
        }
        Err(Error::invalid("排名浏览的来源范围嵌套过深"))
    }
}
