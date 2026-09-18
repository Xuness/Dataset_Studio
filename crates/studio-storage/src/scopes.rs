use crate::*;
use studio_application::ScopeRepository;

pub(super) struct ResolvedScope {
    pub sql: String,
    pub count: u64,
    pub results: Vec<String>,
    pub artifacts: Vec<String>,
    pub provenance: serde_json::Value,
}
pub(super) fn resolve(db: &Connection, pid: &str, scope: &ScopeRef) -> Result<ResolvedScope> {
    scope.validate_project(pid)?;
    let (sql, count, results) = match &scope.target {
        ScopeTarget::Selection { revision } => {
            selection::check_revision(db, *revision)?;
            let mut stmt=db.prepare("SELECT result_id FROM result_references WHERE owner_kind='selection' AND owner_id='selection' ORDER BY result_id").map_err(db_error)?;
            let ids = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(db_error)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(db_error)?;
            (selection::MEMBERS.into(), selection::read(db)?.count, ids)
        }
        ScopeTarget::QueryResult { result_id } => {
            let result = query::ready_result(db, pid, result_id)?;
            (
                format!(
                    "SELECT source_id,asset_id FROM result_members WHERE result_id='{result_id}'"
                ),
                result.count.unwrap_or(0),
                vec![result_id.clone()],
            )
        }
        ScopeTarget::Workset { collection_id } => {
            let count = db
                .query_row(
                    "SELECT count FROM collections WHERE id=?1 AND NOT EXISTS(SELECT 1 FROM evaluation_workset_builds b WHERE b.collection_id=collections.id AND b.state!='ready')",
                    [collection_id],
                    |r| unsigned(r, 0),
                )
                .optional()
                .map_err(db_error)?
                .ok_or_else(|| Error::new("NOT_FOUND", "工作集不属于当前项目"))?;
            let mut stmt=db.prepare("SELECT result_id FROM result_references WHERE owner_kind='collection' AND owner_id=?1 ORDER BY result_id").map_err(db_error)?;
            let ids = stmt
                .query_map([collection_id], |r| r.get::<_, String>(0))
                .map_err(db_error)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(db_error)?;
            (
                format!(
                    "SELECT source_id,asset_id FROM collection_members WHERE collection_id='{collection_id}'"
                ),
                count,
                ids,
            )
        }
        ScopeTarget::Source { .. } => {
            return Err(Error::new(
                "SCOPE_REQUIRES_CAPTURE",
                "来源范围需要先在后台构建成员引用",
            ));
        }
    };
    let basis = results
        .iter()
        .map(|id| query::read_result(db, pid, id))
        .collect::<Result<Vec<_>>>()?;
    let mut artifacts = std::collections::BTreeSet::new();
    let owner = match &scope.target {
        ScopeTarget::Workset { collection_id } => Some(("collection", collection_id.as_str())),
        ScopeTarget::Selection { .. } => Some(("selection", "selection")),
        _ => None,
    };
    let mut stmt = db
        .prepare("SELECT artifact_id FROM artifact_references WHERE owner_kind=?1 AND owner_id=?2")
        .map_err(db_error)?;
    if let Some((kind, id)) = owner {
        for id in stmt
            .query_map(params![kind, id], |r| r.get::<_, String>(0))
            .map_err(db_error)?
        {
            artifacts.insert(id.map_err(db_error)?);
        }
    }
    for rid in &results {
        for id in stmt
            .query_map(params!["query_result", rid], |r| r.get::<_, String>(0))
            .map_err(db_error)?
        {
            artifacts.insert(id.map_err(db_error)?);
        }
    }
    let artifacts = artifacts.into_iter().collect::<Vec<_>>();
    let provenance = serde_json::json!({"version":1,"scope":scope,"queries":basis,"artifacts":artifacts,"meaning":"fixed_asset_members; metadata_values_are_not_frozen"});
    Ok(ResolvedScope {
        sql,
        count,
        results,
        artifacts,
        provenance,
    })
}
pub(super) fn references(db: &Connection, kind: &str, id: &str, results: &[String]) -> Result<()> {
    for result in results {
        db.execute(
            "INSERT OR IGNORE INTO result_references VALUES (?1,?2,?3)",
            params![kind, id, result],
        )
        .map_err(db_error)?;
    }
    Ok(())
}
pub(super) fn artifact_references(
    db: &Connection,
    kind: &str,
    id: &str,
    artifacts: &[String],
) -> Result<()> {
    for artifact in artifacts {
        db.execute(
            "INSERT OR IGNORE INTO artifact_references VALUES (?1,?2,?3)",
            params![kind, id, artifact],
        )
        .map_err(db_error)?;
    }
    Ok(())
}
impl ScopeRepository for SqliteStore {
    fn change_selection_scope(
        &self,
        pid: &str,
        expected: u64,
        scope: &ScopeRef,
        operation: ScopeOperation,
    ) -> Result<Selection> {
        scope.validate_project(pid)?;
        let history_limit = self.editing_settings()?.undo_limit;
        let p = self.handle(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.transaction().map_err(db_error)?;
        selection::check_revision(&tx, expected)?;
        let resolved = resolve(&tx, pid, scope)?;
        let history_id = history::begin(
            &tx,
            history_limit,
            match operation {
                ScopeOperation::Replace => "替换当前选择",
                ScopeOperation::Add => "将范围加入选择",
                ScopeOperation::Remove => "从选择移除范围",
                ScopeOperation::Intersect => "选择取交集",
            },
        )?;
        // Self-selection operations must read the original relation before any mutation.
        // A TEMP table is disk-backed and dropped in this transaction; no frontend ID list.
        tx.execute_batch("DROP TABLE IF EXISTS temp.scope_members; CREATE TEMP TABLE scope_members(source_id TEXT,asset_id TEXT,PRIMARY KEY(source_id,asset_id)) WITHOUT ROWID;").map_err(db_error)?;
        if operation == ScopeOperation::Replace
            && let ScopeTarget::QueryResult { result_id } = &scope.target
        {
            selection::clear(&tx)?;
            tx.execute("INSERT INTO selection_base VALUES (1,?1)", [result_id])
                .map_err(db_error)?;
        } else {
            tx.execute(
                &format!("INSERT INTO temp.scope_members {}", resolved.sql),
                [],
            )
            .map_err(db_error)?;
            match operation {
                ScopeOperation::Replace => {
                    selection::clear(&tx)?;
                    tx.execute("INSERT INTO selection SELECT * FROM temp.scope_members", [])
                        .map_err(db_error)?;
                }
                ScopeOperation::Add => {
                    tx.execute("DELETE FROM selection_exclusions WHERE (source_id,asset_id) IN (SELECT * FROM temp.scope_members)",[]).map_err(db_error)?;
                    tx.execute("INSERT OR IGNORE INTO selection SELECT s.source_id,s.asset_id FROM temp.scope_members s WHERE NOT EXISTS(SELECT 1 FROM result_members m WHERE m.result_id=(SELECT result_id FROM selection_base WHERE singleton=1) AND m.source_id=s.source_id AND m.asset_id=s.asset_id)",[]).map_err(db_error)?;
                }
                ScopeOperation::Remove => {
                    tx.execute("DELETE FROM selection WHERE (source_id,asset_id) IN (SELECT * FROM temp.scope_members)",[]).map_err(db_error)?;
                    tx.execute("INSERT OR IGNORE INTO selection_exclusions SELECT m.source_id,m.asset_id FROM result_members m JOIN temp.scope_members s ON s.source_id=m.source_id AND s.asset_id=m.asset_id WHERE m.result_id=(SELECT result_id FROM selection_base WHERE singleton=1)",[]).map_err(db_error)?;
                }
                ScopeOperation::Intersect => {
                    tx.execute("DELETE FROM selection WHERE (source_id,asset_id) NOT IN (SELECT * FROM temp.scope_members)",[]).map_err(db_error)?;
                    tx.execute("INSERT OR IGNORE INTO selection_exclusions SELECT m.source_id,m.asset_id FROM result_members m WHERE m.result_id=(SELECT result_id FROM selection_base WHERE singleton=1) AND NOT EXISTS(SELECT 1 FROM temp.scope_members s WHERE s.source_id=m.source_id AND s.asset_id=m.asset_id)",[]).map_err(db_error)?;
                }
            }
        }
        references(&tx, "selection", "selection", &resolved.results)?;
        artifact_references(&tx, "selection", "selection", &resolved.artifacts)?;
        tx.execute_batch("DROP TABLE temp.scope_members;")
            .map_err(db_error)?;
        let selection = selection::publish(&tx)?;
        history::finish(&tx, history_id, history_limit)?;
        tx.commit().map_err(db_error)?;
        Ok(selection)
    }
    fn save_scope_collection(&self, pid: &str, name: &str, scope: &ScopeRef) -> Result<Collection> {
        let name = validate_name(name)?;
        let p = self.handle(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.transaction().map_err(db_error)?;
        let resolved = resolve(&tx, pid, scope)?;
        if resolved.count == 0 {
            return Err(Error::invalid("工作集成员不能为空"));
        }
        let id = new_id();
        tx.execute(
            "INSERT INTO collections VALUES (?1,?2,?3)",
            params![id, name, resolved.count as i64],
        )
        .map_err(db_error)?;
        tx.execute(
            &format!(
                "INSERT INTO collection_members SELECT ?1,source_id,asset_id FROM ({})",
                resolved.sql
            ),
            [&id],
        )
        .map_err(db_error)?;
        references(&tx, "collection", &id, &resolved.results)?;
        artifact_references(&tx, "collection", &id, &resolved.artifacts)?;
        tx.execute(
            "INSERT INTO collection_scopes VALUES (?1,?2,?3)",
            params![
                id,
                serde_json::to_string(scope).map_err(Error::io)?,
                resolved.provenance.to_string()
            ],
        )
        .map_err(db_error)?;
        management::created(&tx, "workset", &id)?;
        event(&tx, "collection.created", &id)?;
        tx.commit().map_err(db_error)?;
        Ok(Collection {
            id,
            name,
            count: resolved.count,
        })
    }
}
