use crate::ranking_projection::{RankingProjection, RankingProjectionReader};
use crate::*;

fn decode(raw: &str) -> Result<RankingProjection> {
    let recipe: RankingProjection = serde_json::from_str(raw).map_err(Error::io)?;
    recipe.validate()?;
    Ok(recipe)
}

fn from_result(db: &Connection, id: &str) -> Result<Option<RankingProjection>> {
    let row:Option<(String,u64)> = db.query_row("SELECT m.recipe_json,r.count FROM ranking_memberships m JOIN query_results r ON r.id=m.result_id WHERE m.result_id=?1 AND r.status='ready' AND r.storage_kind IN ('ranking','legacy','sealed')",[id],|r|Ok((r.get(0)?,unsigned(r,1)?))).optional().map_err(db_error)?;
    row.map(|(raw, count)| {
        let mut recipe = decode(&raw)?;
        recipe.count = count;
        Ok(recipe)
    })
    .transpose()
}

pub(super) fn has_presentation(db: &Connection, scope: &ScopeRef) -> Result<bool> {
    match &scope.target {
        ScopeTarget::QueryResult { result_id } => db.query_row("SELECT EXISTS(SELECT 1 FROM ranking_memberships m JOIN query_results r ON r.id=m.result_id WHERE m.result_id=?1 AND r.status='ready')",[result_id],|r|r.get(0)).map_err(db_error),
        ScopeTarget::Workset { collection_id, .. } => db.query_row("SELECT EXISTS(SELECT 1 FROM collection_scopes WHERE collection_id=?1 AND json_type(provenance_json,'$.ranking_artifact')='text')",[collection_id],|r|r.get(0)).map_err(db_error),
        _ => Ok(false),
    }
}

pub(super) fn sources(db: &Connection, aid: &str, input: &Path) -> Result<Vec<String>> {
    let versions: Option<String> = db.query_row("SELECT r.versions_json FROM artifacts a JOIN job_runs r ON r.job_id=a.job_id WHERE a.id=?1", [aid], |r|r.get(0)).optional().map_err(db_error)?;
    let mut sources = if let Some(raw) = versions {
        serde_json::from_str::<Vec<QuerySourceVersion>>(&raw)
            .map_err(Error::io)?
            .into_iter()
            .map(|v| v.source_id)
            .collect::<Vec<_>>()
    } else {
        // Old imported materials and small test fixtures may predate job_runs.
        let input = ranking_projection::open_read(input)?;
        let mut sources = input
            .prepare("SELECT DISTINCT source_id FROM members LIMIT 65")
            .map_err(db_error)?
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        if sources.is_empty() {
            sources = input
                .prepare("SELECT DISTINCT source_id FROM input_rows LIMIT 65")
                .map_err(db_error)?
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(db_error)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(db_error)?;
        }
        sources
    };
    sources.sort();
    sources.dedup();
    if sources.is_empty() || sources.len() > 64 {
        return Err(Error::invalid("排名输入来源不完整"));
    }
    Ok(sources)
}

pub(super) fn create_recipe(
    db: &Connection,
    directory: &Path,
    aid: &str,
    workset_id: &str,
    filter: &RankingFilter,
    files: (&Path, &Path),
    count: u64,
) -> Result<RankingProjection> {
    let (input, scores) = files;
    let root = directory.canonicalize().map_err(Error::io)?;
    let relative = |path: &Path| -> Result<String> {
        let full = path.canonicalize().map_err(Error::io)?;
        Ok(full
            .strip_prefix(&root)
            .map_err(|_| Error::invalid("排名文件不属于当前项目"))?
            .to_string_lossy()
            .replace('\\', "/"))
    };
    let recipe = RankingProjection {
        version: 1,
        artifact_id: aid.into(),
        input_file: relative(input)?,
        score_file: relative(scores)?,
        source_ids: sources(db, aid, input)?,
        filter: filter.clone(),
        ratings: None,
        count,
        workset_id: workset_id.into(),
        edits: Vec::new(),
        member_result: None,
    };
    recipe.files(directory)?;
    Ok(recipe)
}

/// Read the committed fixed member recipe. Legacy ranking worksets already
/// record the immutable artifact and exact saved predicate, so their first
/// browse needs neither a member rewrite nor a new per-workset ranking index.
pub(super) fn resolve(
    db: &Connection,
    pid: &str,
    scope: &ScopeRef,
) -> Result<Option<RankingProjection>> {
    scope.validate_project(pid)?;
    match &scope.target {
        ScopeTarget::QueryResult { result_id } => from_result(db, result_id),
        ScopeTarget::Workset {
            collection_id,
            revision,
        } => {
            let collection = collection_edits::read(db, collection_id, *revision)?;
            let row: Option<String> = db
                .query_row(
                    "SELECT provenance_json FROM collection_scopes WHERE collection_id=?1",
                    [collection_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(db_error)?;
            let Some(provenance) = row else {
                return Ok(None);
            };
            let base:Option<String>=db.query_row("SELECT result_id FROM collection_bases WHERE collection_id=?1 AND NOT EXISTS(SELECT 1 FROM collection_inclusions WHERE collection_id=?1) AND NOT EXISTS(SELECT 1 FROM collection_exclusions WHERE collection_id=?1)",[collection_id],|r|r.get(0)).optional().map_err(db_error)?;
            if let Some(base) = base
                && let Some(recipe) = from_result(db, &base)?
            {
                return with_edits(db, recipe, &collection).map(Some);
            }
            let provenance: serde_json::Value =
                serde_json::from_str(&provenance).map_err(Error::io)?;
            let Some(aid) = provenance.get("ranking_artifact").and_then(|v| v.as_str()) else {
                return Ok(None);
            };
            let artifact = artifacts::read(db, pid, aid)?;
            if artifact.state != ArtifactState::Ready || artifact.kind != RANKING_KIND {
                return Err(Error::new("ARTIFACT_NOT_READY", "工作集的排名成果不可用"));
            }
            let linked:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM artifact_references WHERE owner_kind='collection' AND owner_id=?1 AND artifact_id=?2)",params![collection_id,aid],|r|r.get(0)).map_err(db_error)?;
            if !linked {
                return Err(Error::new("ARTIFACT_INVALID", "工作集缺少排名成果引用"));
            }
            let filter: RankingFilter = serde_json::from_value(
                provenance
                    .get("filter")
                    .cloned()
                    .ok_or_else(|| Error::invalid("工作集缺少排名条件"))?,
            )
            .map_err(Error::io)?;
            let directory = Path::new(db.path().ok_or_else(|| Error::invalid("项目路径不可用"))?)
                .parent()
                .ok_or_else(|| Error::invalid("项目路径不可用"))?;
            let input = artifact
                .files
                .iter()
                .find(|f| f.path.ends_with(".ranking-input.sqlite"))
                .ok_or_else(|| Error::new("ARTIFACT_INVALID", "排名输入文件缺失"))?;
            let scores = artifact
                .files
                .iter()
                .find(|f| f.path.ends_with(".ranking.sqlite"))
                .ok_or_else(|| Error::new("ARTIFACT_INVALID", "排名评分文件缺失"))?;
            create_recipe(
                db,
                directory,
                aid,
                collection_id,
                &filter,
                (&directory.join(&input.path), &directory.join(&scores.path)),
                collection.count,
            )
            .and_then(|recipe| with_edits(db, recipe, &collection))
            .map(Some)
        }
        _ => Ok(None),
    }
}

fn with_edits(
    db: &Connection,
    mut recipe: RankingProjection,
    collection: &Collection,
) -> Result<RankingProjection> {
    let changed: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM collection_member_changes WHERE collection_id=?1 AND valid_from<=?2)",params![collection.id,collection.revision as i64],|r|r.get(0)).map_err(db_error)?;
    if changed {
        recipe
            .edits
            .push(crate::ranking_projection::RankingEditLayer {
                collection_id: collection.id.clone(),
                revision: collection.revision,
                before_ratings: recipe.ratings.take(),
                before_result: recipe.member_result.take(),
            });
        let mut stmt = db.prepare("SELECT s.id FROM sources s WHERE EXISTS(SELECT 1 FROM collection_member_changes e WHERE e.collection_id=?1 AND e.source_id=s.id AND e.present=1 AND e.valid_until IS NULL AND e.valid_from<=?2) OR EXISTS(SELECT 1 FROM collection_member_changes e WHERE e.collection_id=?1 AND e.source_id=s.id AND e.present=1 AND e.valid_until>?2 AND e.valid_from<=?2) ORDER BY s.id LIMIT 65").map_err(db_error)?;
        for sid in stmt
            .query_map(params![collection.id, collection.revision as i64], |r| {
                r.get::<_, String>(0)
            })
            .map_err(db_error)?
        {
            recipe.source_ids.push(sid.map_err(db_error)?);
        }
        recipe.source_ids.sort();
        recipe.source_ids.dedup();
    } else if recipe.count != collection.count {
        return Err(Error::new(
            "ARTIFACT_INVALID",
            "工作集与固定排名成员数量不一致",
        ));
    }
    recipe.workset_id = collection.id.clone();
    recipe.count = collection.count;
    recipe.validate()?;
    Ok(recipe)
}

/// A non-Rating query has its own fixed members. Keep its ranking presentation
/// at submission so later edits (or disposal of intermediate query views) do
/// not make those members inherit a different workset version.
pub(super) fn snapshot_presentation(
    db: &Connection,
    pid: &str,
    result_id: &str,
    spec: &QuerySpec,
) -> Result<()> {
    let Some(scope) = &spec.input_scope else {
        return Ok(());
    };
    if !has_presentation(db, scope)? {
        return Ok(());
    }
    let Some(mut recipe) = resolve(db, pid, scope)? else {
        return Ok(());
    };
    if recipe.edits.is_empty() && recipe.member_result.is_none() {
        return Ok(());
    }
    recipe.member_result = Some(result_id.into());
    save_recipe(db, result_id, &recipe)
}

fn save_recipe(db: &Connection, result_id: &str, recipe: &RankingProjection) -> Result<()> {
    db.execute("INSERT INTO ranking_memberships VALUES(?1,?2,?3,?4) ON CONFLICT(result_id) DO UPDATE SET artifact_id=excluded.artifact_id,recipe_json=excluded.recipe_json,fingerprint=excluded.fingerprint", params![result_id,recipe.artifact_id,serde_json::to_string(recipe).map_err(Error::io)?,recipe.fingerprint()?]).map_err(db_error)?;
    db.execute(
        "DELETE FROM collection_version_references WHERE result_id=?1",
        [result_id],
    )
    .map_err(db_error)?;
    db.execute(
        "DELETE FROM result_references WHERE owner_kind='ranking_recipe' AND owner_id=?1",
        [result_id],
    )
    .map_err(db_error)?;
    for layer in &recipe.edits {
        db.execute(
            "INSERT OR IGNORE INTO collection_version_references VALUES(?1,?2,?3)",
            params![result_id, layer.collection_id, layer.revision as i64],
        )
        .map_err(db_error)?;
    }
    for fence in recipe.fences().filter(|id| *id != result_id) {
        db.execute(
            "INSERT OR IGNORE INTO result_references VALUES('ranking_recipe',?1,?2)",
            params![result_id, fence],
        )
        .map_err(db_error)?;
    }
    db.execute(
        "INSERT OR IGNORE INTO artifact_references VALUES('query_result',?1,?2)",
        params![result_id, recipe.artifact_id],
    )
    .map_err(db_error)?;
    Ok(())
}

pub(super) fn insert_result(
    db: &Connection,
    pid: &str,
    recipe: &RankingProjection,
    spec: &QuerySpec,
    versions: &[QuerySourceVersion],
    definition: Option<(&str, u64)>,
    internal: bool,
) -> Result<QueryResult> {
    let fingerprint = recipe.fingerprint()?;
    let previous:Option<(String,String,i64)>=db.query_row("SELECT r.id,r.family_id,r.member_revision FROM ranking_memberships m JOIN query_results r ON r.id=m.result_id WHERE m.fingerprint=?1 AND r.status='ready' ORDER BY r.created_at,r.id LIMIT 1",[&fingerprint],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(db_error)?;
    let result = query::insert_result(db, pid, definition, spec, versions)?;
    save_recipe(db, &result.id, recipe)?;
    db.execute("UPDATE query_results SET storage_kind='ranking',status='ready',cache_mode='ranking',count=?2,internal=?3,post_ready=0 WHERE id=?1",params![result.id,recipe.count as i64,internal]).map_err(db_error)?;
    db.execute("UPDATE query_families SET fixed=1,cached=0,latest_result_id=?1,latest_count=?2 WHERE id=?1",params![result.id,recipe.count as i64]).map_err(db_error)?;
    if let Some((base, family, revision)) = previous {
        db.execute("UPDATE query_results SET family_id=?2,member_revision=?3,cache_base=?4,cache_mode='reused' WHERE id=?1",params![result.id,family,revision,base]).map_err(db_error)?;
        db.execute("DELETE FROM query_families WHERE id=?1", [&result.id])
            .map_err(db_error)?;
        db.execute("UPDATE query_families SET latest_result_id=?2,latest_count=?3,touched_at=CAST(?4 AS INTEGER) WHERE id=?1",params![family,result.id,recipe.count as i64,now()]).map_err(db_error)?;
    }
    db.execute(
        "INSERT OR IGNORE INTO artifact_references VALUES('query_result',?1,?2)",
        params![result.id, recipe.artifact_id],
    )
    .map_err(db_error)?;
    // The recipe is self-contained and owns its artifact reference. Keeping a
    // chain of query inputs would unnecessarily prevent releasing old views.
    query::clear_input_references(db, &result.id)?;
    query::read_result(db, pid, &result.id)
}

impl SqliteStore {
    pub fn ranking_projection(
        &self,
        pid: &str,
        scope: &ScopeRef,
    ) -> Result<Option<RankingProjection>> {
        let project = self.handle(pid)?;
        resolve(&*project.read()?, pid, scope)
    }
    /// Rating-only refinements over a fixed ranking range are fixed themselves;
    /// they need a new small reference, not an asynchronous member copy.
    pub fn create_ranking_result(
        &self,
        pid: &str,
        spec: &QuerySpec,
        definition: Option<(&str, u64)>,
        versions: &[QuerySourceVersion],
        cancelled: Arc<AtomicBool>,
    ) -> Result<Option<QueryResult>> {
        let spec = {
            let project = self.handle(pid)?;
            collection_edits::pin_spec(&*project.read()?, pid, spec)?
        };
        let spec = &spec;
        let Some(scope) = &spec.input_scope else {
            return Ok(None);
        };
        let project = self.handle(pid)?;
        let mut recipe = {
            let db = project.read()?;
            if !has_presentation(&db, scope)? {
                return Ok(None);
            }
            let Some(mut recipe) = resolve(&db, pid, scope)? else {
                return Ok(None);
            };
            if recipe.source_ids != spec.source_ids {
                return Ok(None);
            }
            for condition in &spec.conditions {
                if ranking_field_id(&condition.field)? != Some(recipe.artifact_id.as_str()) {
                    return Ok(None);
                }
                let values = match (&condition.operator, &condition.value) {
                    (QueryOperator::Eq, Some(QueryValue::Text(value))) => vec![value.clone()],
                    (QueryOperator::In, Some(QueryValue::TextList(values))) => values.clone(),
                    _ => return Ok(None),
                };
                recipe.restrict_ratings(&values);
            }
            recipe.validate()?;
            recipe
        };
        let fingerprint = recipe.fingerprint()?;
        let cached = {
            let db = project.read()?;
            db.query_row("SELECT r.count FROM ranking_memberships m JOIN query_results r ON r.id=m.result_id WHERE m.fingerprint=?1 AND r.status='ready' LIMIT 1",[&fingerprint],|r|unsigned(r,0)).optional().map_err(db_error)?
        };
        recipe.count = if let Some(count) = cached {
            count
        } else {
            RankingProjectionReader::open(&project.project.directory, &recipe, cancelled.clone())?
                .count()?
        };
        studio_application::read_cancelled(&cancelled)?;
        let mut db = project.write_cancelled(&cancelled)?;
        let tx = db.project_transaction().map_err(db_error)?;
        query::validate_sources(&tx, spec)?;
        query::validate_input(&tx, pid, spec)?;
        if let Some((id, revision)) = definition {
            let current = query::read_definition(&tx, pid, id)?;
            if current.revision != revision
                || collection_edits::pin_spec(&tx, pid, &current.spec.clone().normalize()?)?
                    != *spec
            {
                return Err(Error::new("REVISION_CONFLICT", "查询定义已变化"));
            }
        }
        if versions.iter().map(|v| &v.source_id).collect::<Vec<_>>()
            != spec.source_ids.iter().collect::<Vec<_>>()
        {
            return Err(Error::invalid("查询来源版本不完整"));
        }
        let result = insert_result(&tx, pid, &recipe, spec, versions, definition, false)?;
        studio_application::read_cancelled(&cancelled)?;
        tx.commit().map_err(db_error)?;
        Ok(Some(result))
    }
}
