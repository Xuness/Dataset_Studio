use crate::*;
use sha2::{Digest, Sha256};

fn canonical(db: &Connection, scope: &ScopeRef) -> Result<ScopeRef> {
    let mut canonical = collection_edits::pin(db, &scope.project_id, scope)?;
    if let ScopeTarget::QueryResult { result_id } = &scope.target {
        let id: String = db.query_row("SELECT original.id FROM query_results current JOIN query_results original ON original.family_id=current.family_id AND original.member_revision=current.member_revision WHERE current.id=?1 ORDER BY original.cache_mode='reused',original.created_at,original.id LIMIT 1", [result_id], |r| r.get(0)).map_err(db_error)?;
        canonical.target = ScopeTarget::QueryResult { result_id: id };
    }
    Ok(canonical)
}

impl SqliteStore {
    pub fn ranked_members_available(&self, pid: &str, scope: &ScopeRef) -> Result<bool> {
        scope.validate_project(pid)?;
        let project = self.handle(pid)?;
        let db = project.read()?;
        match &scope.target {
            ScopeTarget::QueryResult{result_id} => db.query_row("SELECT EXISTS(SELECT 1 FROM query_results owner JOIN query_results alias ON alias.family_id=owner.family_id AND alias.member_revision=owner.member_revision WHERE owner.id=?1 AND alias.status='ready')", [result_id], |r| r.get(0)).map_err(db_error),
            ScopeTarget::Workset { collection_id, .. } => db.query_row("SELECT EXISTS(SELECT 1 FROM collections WHERE id=?1)", [collection_id], |r|r.get(0)).map_err(db_error),
            _ => Ok(false),
        }
    }
    pub fn canonical_ranked_scope(&self, pid: &str, scope: &ScopeRef) -> Result<ScopeRef> {
        scope.validate_project(pid)?;
        let project = self.handle(pid)?;
        canonical(&*project.read()?, scope)
    }
    /// Follow only the explicit, fixed input scope. Other artifact references do
    /// not imply that their scores define the order of this member set.
    pub fn ranked_scope(&self, pid: &str, scope: &ScopeRef) -> Result<Option<RankedScope>> {
        scope.validate_project(pid)?;
        let project = self.handle(pid)?;
        let db = project.read()?;
        let index_scope = canonical(&db, scope)?;
        if crate::ranking_memberships::has_presentation(&db, scope)?
            && let Some(recipe) = crate::ranking_memberships::resolve(&db, pid, scope)?
        {
            let artifact = artifacts::read(&db, pid, &recipe.artifact_id)?;
            if artifact.state != ArtifactState::Ready || artifact.kind != RANKING_KIND {
                return Err(Error::new("ARTIFACT_NOT_READY", "固定排名成果不可用"));
            }
            return Ok(Some(RankedScope {
                current_rating_filter: false,
                view_key: format!(
                    "ranked:{}",
                    hex::encode(Sha256::digest(
                        serde_json::to_vec(&(&recipe.workset_id, recipe.fingerprint()?))
                            .map_err(Error::io)?
                    ))
                ),
                index_scope,
                schema_version: artifact.schema_version,
                workset_id: recipe.workset_id,
                artifact_id: recipe.artifact_id,
                artifact_name: artifact.name,
                count: recipe.count,
                saved_filter: recipe.filter,
            }));
        }
        let mut view_spec = None;
        let mut target = scope.target.clone();
        let mut count = None;
        let mut current_rating_filter = false;
        for _ in 0..16 {
            match target {
                ScopeTarget::QueryResult { result_id } => {
                    let result = query::ready_result(&db, pid, &result_id)?;
                    current_rating_filter |=
                        result.spec.conditions.iter().any(|c| c.field == "rating");
                    if view_spec.is_none() {
                        let mut spec = result.spec.clone().normalize()?;
                        spec.order = QueryOrder::AssetKeyAsc;
                        view_spec = Some(spec);
                    }
                    count.get_or_insert(result.count.unwrap_or(0));
                    let Some(parent) = result.spec.input_scope else {
                        return Ok(None);
                    };
                    parent.validate_project(pid)?;
                    target = parent.target;
                }
                ScopeTarget::Workset { collection_id, .. } => {
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
                    let view_key = format!(
                        "ranked:{}",
                        hex::encode(Sha256::digest(
                            serde_json::to_vec(&(
                                pid,
                                &collection_id,
                                aid,
                                &saved_filter,
                                &view_spec
                            ))
                            .map_err(Error::io)?
                        ))
                    );
                    return Ok(Some(RankedScope {
                        current_rating_filter,
                        index_scope,
                        view_key,
                        schema_version: artifact.schema_version,
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
