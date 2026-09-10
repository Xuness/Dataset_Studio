use crate::*;
use studio_application::QueryRepository;

fn decode<T: serde::de::DeserializeOwned>(text: String) -> Result<T> {
    serde_json::from_str(&text)
        .map_err(|e| Error::new("DATABASE_ERROR", format!("项目查询记录无效：{e}")))
}
pub(super) fn read_definition(db: &Connection, pid: &str, id: &str) -> Result<QueryDefinition> {
    validate_id(id)?;
    if management::removed(db, "query", id)? {
        return Err(Error::new(
            "OBJECT_REMOVED",
            "查询定义已删除，旧结果保留生成时的条件",
        ));
    }
    let row = db
        .query_row(
            "SELECT name,revision,spec_json,created_at FROM query_definitions WHERE id=?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    unsigned(r, 1)?,
                    r.get::<_, String>(2)?,
                    r.get(3)?,
                ))
            },
        )
        .optional()
        .map_err(db_error)?
        .ok_or_else(|| Error::new("NOT_FOUND", "查询定义不属于当前项目"))?;
    Ok(QueryDefinition {
        id: id.into(),
        project_id: pid.into(),
        name: management::display_name(db, "query", id, &row.0)?,
        revision: row.1,
        spec: decode(row.2)?,
        created_at: row.3,
    })
}
pub(super) fn read_result(db: &Connection, pid: &str, id: &str) -> Result<QueryResult> {
    validate_id(id)?;
    let row = db.query_row("SELECT definition_id,definition_revision,spec_json,versions_json,status,processed,count,created_at,error FROM query_results WHERE id=?1",[id],|r| Ok((r.get(0)?,r.get::<_,Option<i64>>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,unsigned(r,5)?,r.get::<_,Option<i64>>(6)?,r.get(7)?,r.get(8)?)))
        .optional().map_err(db_error)?.ok_or_else(|| Error::new("NOT_FOUND", "查询结果不属于当前项目"))?;
    let state: ResultState = decode(format!("\"{}\"", row.4))?;
    let raw_cache = db
        .query_row(
            "SELECT r.cache_mode,r.evaluated_count,r.changed_members,f.tier,f.fixed,f.session_only,r.basis_ratings_json,r.candidate_records FROM query_results r JOIN query_families f ON f.id=r.family_id WHERE r.id=?1",
            [id],
            |r| {
                Ok((r.get::<_,String>(0)?,unsigned(r,1)?,unsigned(r,2)?,r.get::<_,String>(3)?,r.get::<_,bool>(4)?,r.get::<_,bool>(5)?,r.get::<_,String>(6)?,unsigned(r,7)?))
            },
        )
        .map_err(db_error)?;
    let cache = QueryCacheInfo {
        mode: raw_cache.0,
        evaluated_objects: raw_cache.1,
        changed_members: raw_cache.2,
        tier: decode(format!("\"{}\"", raw_cache.3))?,
        fixed: raw_cache.4,
        session_only: raw_cache.5,
        basis_ratings: decode(raw_cache.6)?,
        candidate_records: raw_cache.7,
    };
    Ok(QueryResult {
        id: id.into(),
        project_id: pid.into(),
        definition_id: row.0,
        definition_revision: row.1.map(|r| r as u64),
        spec: decode(row.2)?,
        source_versions: decode(row.3)?,
        state,
        processed: row.5,
        count: if state == ResultState::Ready {
            row.6.map(|r| r as u64)
        } else {
            None
        },
        created_at: row.7,
        error: row.8,
        cache,
    })
}
pub(super) fn ready_result(db: &Connection, pid: &str, id: &str) -> Result<QueryResult> {
    let result = read_result(db, pid, id)?;
    if result.state != ResultState::Ready {
        return Err(Error::new("RESULT_NOT_READY", "结果尚未完整构建或已释放"));
    }
    Ok(result)
}
pub(super) fn validate_sources(db: &Connection, spec: &QuerySpec) -> Result<()> {
    let mut stmt = db
        .prepare("SELECT 1 FROM sources WHERE id=?1 AND NOT EXISTS(SELECT 1 FROM object_metadata m WHERE m.kind='source' AND m.id=?1 AND m.deleted=1)")
        .map_err(db_error)?;
    for id in &spec.source_ids {
        if !stmt.exists([id]).map_err(db_error)? {
            return Err(Error::new("NOT_FOUND", "查询来源不属于当前项目"));
        }
    }
    Ok(())
}
fn listed_ids(
    db: &Connection,
    table: &str,
    after: Option<&str>,
    limit: usize,
) -> Result<Vec<String>> {
    // `table` is supplied only by the two repository methods below.
    let before = after
        .map(|id| -> Result<String> {
            validate_id(id)?;
            db.query_row(
                &format!("SELECT created_at FROM {table} WHERE id=?1"),
                [id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?
            .ok_or_else(|| Error::invalid("列表游标不属于当前项目"))
        })
        .transpose()?;
    let visibility = if table == "query_results" {
        "internal=0"
    } else {
        "NOT EXISTS(SELECT 1 FROM object_metadata m WHERE m.kind='query' AND m.id=query_definitions.id AND m.deleted=1)"
    };
    let predicate = if after.is_some() {
        format!("WHERE {visibility} AND (created_at,id)<(?1,?2)")
    } else {
        format!("WHERE {visibility}")
    };
    let mut stmt = db
        .prepare(&format!(
            "SELECT id FROM {table} {predicate} ORDER BY created_at DESC,id DESC LIMIT ?3"
        ))
        .map_err(db_error)?;
    stmt.query_map(
        params![
            before.unwrap_or_default(),
            after.unwrap_or(""),
            limit.clamp(1, 101) as u32
        ],
        |r| r.get::<_, String>(0),
    )
    .map_err(db_error)?
    .collect::<std::result::Result<Vec<_>, _>>()
    .map_err(db_error)
}
pub(super) fn insert_result(
    db: &Connection,
    pid: &str,
    definition: Option<(&str, u64)>,
    spec: &QuerySpec,
    versions: &[QuerySourceVersion],
) -> Result<QueryResult> {
    validate_sources(db, spec)?;
    let id = new_id();
    db.execute("INSERT INTO query_results(id,definition_id,definition_revision,spec_json,versions_json,status,count,created_at) VALUES (?1,?2,?3,?4,?5,'queued',0,?6)",params![id,definition.map(|d|d.0),definition.map(|d|d.1 as i64),serde_json::to_string(spec).map_err(Error::io)?,serde_json::to_string(versions).map_err(Error::io)?,now()]).map_err(db_error)?;
    derived_fields::references(db, pid, "query_result", &id, spec)?;
    input_references(db, pid, "query_input", &id, spec)?;
    event(db, "result.created", &id)?;
    read_result(db, pid, &id)
}
impl QueryRepository for SqliteStore {
    fn save_query(
        &self,
        pid: &str,
        name: &str,
        spec: QuerySpec,
        previous: Option<(&str, u64)>,
    ) -> Result<QueryDefinition> {
        let name = validate_name(name)?;
        let spec = spec.normalize()?;
        let p = self.handle(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.transaction().map_err(db_error)?;
        validate_sources(&tx, &spec)?;
        let json = serde_json::to_string(&spec).map_err(Error::io)?;
        let id = if let Some((id, expected)) = previous {
            let old = read_definition(&tx, pid, id)?;
            if old.revision != expected {
                return Err(Error::new("REVISION_CONFLICT", "查询定义已被修改"));
            }
            tx.execute(
                "UPDATE query_definitions SET name=?2,spec_json=?3,revision=revision+1 WHERE id=?1",
                params![id, name, json],
            )
            .map_err(db_error)?;
            id.to_owned()
        } else {
            let id = new_id();
            tx.execute(
                "INSERT INTO query_definitions VALUES (?1,?2,1,?3,?4)",
                params![id, name, json, now()],
            )
            .map_err(db_error)?;
            id
        };
        derived_fields::references(&tx, pid, "query_definition", &id, &spec)?;
        input_references(&tx, pid, "query_definition_input", &id, &spec)?;
        management::named(&tx, "query", &id, &name)?;
        event(&tx, "query.changed", &id)?;
        let result = read_definition(&tx, pid, &id)?;
        tx.commit().map_err(db_error)?;
        Ok(result)
    }
    fn query_definition(&self, pid: &str, id: &str) -> Result<QueryDefinition> {
        let p = self.handle(pid)?;
        read_definition(&*p.read()?, pid, id)
    }
    fn query_definitions(
        &self,
        pid: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<QueryDefinition>> {
        let p = self.handle(pid)?;
        let db = p.read()?;
        let ids = listed_ids(&db, "query_definitions", after, limit)?;
        ids.iter().map(|id| read_definition(&db, pid, id)).collect()
    }
    fn create_result(
        &self,
        pid: &str,
        definition: Option<(&str, u64)>,
        spec: QuerySpec,
        mut versions: Vec<QuerySourceVersion>,
    ) -> Result<QueryResult> {
        let spec = spec.normalize()?;
        versions.sort_by(|a, b| a.source_id.cmp(&b.source_id));
        if versions.iter().map(|v| &v.source_id).collect::<Vec<_>>()
            != spec.source_ids.iter().collect::<Vec<_>>()
        {
            return Err(Error::invalid("查询来源版本不完整"));
        }
        let p = self.handle(pid)?;
        self.mark_background(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.transaction().map_err(db_error)?;
        validate_sources(&tx, &spec)?;
        if let Some((id, revision)) = definition {
            let current = read_definition(&tx, pid, id)?;
            if current.revision != revision || current.spec != spec {
                return Err(Error::new(
                    "REVISION_CONFLICT",
                    "查询定义已变化，请刷新后构建",
                ));
            }
        }
        let result = insert_result(&tx, pid, definition, &spec, &versions)?;
        tx.commit().map_err(db_error)?;
        Ok(result)
    }
    fn query_result(&self, pid: &str, id: &str) -> Result<QueryResult> {
        let p = self.handle(pid)?;
        read_result(&*p.read()?, pid, id)
    }
    fn query_results(
        &self,
        pid: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<QueryResult>> {
        let p = self.handle(pid)?;
        let db = p.read()?;
        let ids = listed_ids(&db, "query_results", after, limit)?;
        ids.iter().map(|id| read_result(&db, pid, id)).collect()
    }
    fn result_page(
        &self,
        pid: &str,
        id: &str,
        after: Option<&AssetKey>,
        limit: usize,
    ) -> Result<ResultPage> {
        let p = self.handle(pid)?;
        let db = p.read()?;
        let result = ready_result(&db, pid, id)?;
        let order = result.spec.order;
        if order.by_post() {
            return post_page(&db, id, after, limit, order);
        }
        let desc = order.descending();
        let (source, asset) = after
            .map(|k| (k.source_id.as_str(), k.asset_id.as_str()))
            .unwrap_or(("", ""));
        let condition = if after.is_some() {
            format!(
                " AND (source_id,asset_id){}(?2,?3)",
                if desc { "<" } else { ">" }
            )
        } else {
            String::new()
        };
        let sql = format!(
            "SELECT source_id,asset_id FROM result_members WHERE result_id=?1{condition} ORDER BY source_id {},asset_id {} LIMIT ?4",
            if desc { "DESC" } else { "ASC" },
            if desc { "DESC" } else { "ASC" }
        );
        let limit = limit.clamp(1, 128);
        let mut stmt = db.prepare(&sql).map_err(db_error)?;
        let mut keys = stmt
            .query_map(params![id, source, asset, (limit + 1) as u32], |r| {
                Ok(AssetKey {
                    source_id: r.get(0)?,
                    asset_id: r.get(1)?,
                })
            })
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        let more = keys.len() > limit;
        keys.truncate(limit);
        Ok(ResultPage {
            next: if more { keys.last().cloned() } else { None },
            keys,
        })
    }
    fn cancel_result(&self, pid: &str, id: &str) -> Result<QueryResult> {
        let p = self.handle(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.transaction().map_err(db_error)?;
        read_result(&tx, pid, id)?;
        if tx.execute("UPDATE query_results SET status='cancelled',count=NULL,error='已取消构建' WHERE id=?1 AND status IN ('queued','running')",[id]).map_err(db_error)?>0 { event(&tx,"result.changed",id)?; }
        clear_input_references(&tx, id)?;
        let result = read_result(&tx, pid, id)?;
        tx.commit().map_err(db_error)?;
        Ok(result)
    }
    fn release_result(&self, pid: &str, id: &str) -> Result<QueryResult> {
        let p = self.handle(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.transaction().map_err(db_error)?;
        let result = read_result(&tx, pid, id)?;
        if matches!(result.state, ResultState::Queued | ResultState::Running) {
            return Err(Error::new("RESULT_IN_USE", "请先取消正在构建的结果"));
        }
        let references: u64 = tx
            .query_row(
                "SELECT COUNT(*) FROM result_references WHERE result_id=?1",
                [id],
                |r| unsigned(r, 0),
            )
            .map_err(db_error)?;
        if references > 0 {
            return Err(Error::new(
                "RESULT_IN_USE",
                format!("结果仍由 {references} 个选择、工作集、查询或任务引用"),
            ));
        }
        // Memberships may be shared by another result or a retained cache.
        clear_input_references(&tx, id)?;
        tx.execute(
            "DELETE FROM artifact_references WHERE owner_kind='query_result' AND owner_id=?1",
            [id],
        )
        .map_err(db_error)?;
        tx.execute(
            "UPDATE query_results SET status='released',count=NULL WHERE id=?1",
            [id],
        )
        .map_err(db_error)?;
        event(&tx, "result.changed", id)?;
        crate::query_cache::collect_family(&tx, &result.id)?;
        let result = read_result(&tx, pid, id)?;
        tx.commit().map_err(db_error)?;
        Ok(result)
    }
}
pub(super) fn post_page(
    db: &Connection,
    id: &str,
    after: Option<&AssetKey>,
    limit: usize,
    order: QueryOrder,
) -> Result<ResultPage> {
    let ready: bool = db
        .query_row(
            "SELECT post_ready FROM query_results WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .map_err(db_error)?;
    if !ready {
        return Err(Error::new(
            "SORT_REQUIRES_REFRESH",
            "该结果尚无帖子 ID 排序信息，请重新执行查询",
        ));
    }
    let (family, revision): (String, i64) = db
        .query_row(
            "SELECT family_id,member_revision FROM query_results WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(db_error)?;
    let visible = "valid_from<=?2 AND (valid_until IS NULL OR valid_until>?2)";
    let (source, asset) = after
        .map(|k| (k.source_id.as_str(), k.asset_id.as_str()))
        .unwrap_or(("", ""));
    let post: Option<i64> = if after.is_some() {
        db.query_row(&format!("SELECT post_id FROM query_member_data WHERE family_id=?1 AND {visible} AND source_id=?3 AND asset_id=?4"),params![family,revision,source,asset],|r|r.get(0)).optional().map_err(db_error)?.ok_or_else(||Error::invalid("结果排序游标不属于该成员版本"))?
    } else {
        None
    };
    let op = if order.descending() { "<" } else { ">" };
    let direction = if order.descending() { "DESC" } else { "ASC" };
    let limit = limit.clamp(1, 128);
    let mut keys = Vec::new();
    for missing in [false, true] {
        if !missing && after.is_some() && post.is_none() {
            continue;
        }
        let remaining = limit + 1 - keys.len();
        if remaining == 0 {
            break;
        }
        let condition = if missing {
            if after.is_some() && post.is_none() {
                format!("post_id IS NULL AND (source_id,asset_id){op}(?4,?5)")
            } else {
                "post_id IS NULL".into()
            }
        } else if post.is_some() {
            format!("post_id IS NOT NULL AND (post_id,source_id,asset_id){op}(?3,?4,?5)")
        } else {
            "post_id IS NOT NULL".into()
        };
        let sql = format!(
            "SELECT source_id,asset_id FROM query_member_data WHERE family_id=?1 AND {visible} AND {condition} ORDER BY post_id {direction},source_id {direction},asset_id {direction} LIMIT ?6"
        );
        let mut stmt = db.prepare(&sql).map_err(db_error)?;
        keys.extend(
            stmt.query_map(
                params![family, revision, post, source, asset, remaining as i64],
                |r| {
                    Ok(AssetKey {
                        source_id: r.get(0)?,
                        asset_id: r.get(1)?,
                    })
                },
            )
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?,
        );
    }
    let more = keys.len() > limit;
    keys.truncate(limit);
    Ok(ResultPage {
        next: if more { keys.last().cloned() } else { None },
        keys,
    })
}
impl SqliteStore {
    pub fn next_result(&self, pid: &str) -> Result<Option<QueryResult>> {
        let p = self.handle(pid)?;
        let db = p.read()?;
        let id: Option<String> = db
            .query_row(
                "SELECT id FROM query_results WHERE status='queued' ORDER BY created_at,id LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?;
        id.map(|id| read_result(&db, pid, &id)).transpose()
    }
    pub fn start_result(&self, pid: &str, id: &str) -> Result<bool> {
        let p = self.handle(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.transaction().map_err(db_error)?;
        let changed = tx
            .execute(
                "UPDATE query_results SET status='running' WHERE id=?1 AND status='queued'",
                [id],
            )
            .map_err(db_error)?
            > 0;
        if changed {
            event(&tx, "result.changed", id)?;
        }
        tx.commit().map_err(db_error)?;
        Ok(changed)
    }
    pub fn append_result(
        &self,
        pid: &str,
        id: &str,
        keys: &[AssetKey],
        processed: u64,
    ) -> Result<()> {
        if keys.len() > 512 || processed > 512 {
            return Err(Error::invalid("结果批次超过 512 行"));
        }
        let p = self.handle(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.transaction().map_err(db_error)?;
        let result = read_result(&tx, pid, id)?;
        if result.state != ResultState::Running {
            return Err(Error::new("CANCELLED", "结果已停止构建"));
        }
        let mut added = 0;
        {
            let mut insert = tx
                .prepare("INSERT OR IGNORE INTO query_member_data(family_id,source_id,asset_id,valid_from) SELECT family_id,?2,?3,member_revision FROM query_results WHERE id=?1")
                .map_err(db_error)?;
            for key in keys {
                if !result.spec.source_ids.contains(&key.source_id)
                    || key.asset_id.is_empty()
                    || key.asset_id.len() > 128
                {
                    return Err(Error::invalid("结果成员不属于查询范围"));
                }
                added += insert
                    .execute(params![id, key.source_id, key.asset_id])
                    .map_err(db_error)?;
            }
        }
        tx.execute(
            "UPDATE query_results SET processed=processed+?2,count=count+?3 WHERE id=?1",
            params![id, processed as i64, added as i64],
        )
        .map_err(db_error)?;
        tx.execute("UPDATE query_families SET stored_members=stored_members+?2 WHERE id=(SELECT family_id FROM query_results WHERE id=?1)",params![id,added as i64]).map_err(db_error)?;
        if added > 0 {
            crate::query_cache::touch_sizes(&tx)?;
        }
        tx.commit().map_err(db_error)
    }
    pub fn finish_result(&self, pid: &str, id: &str, error: Option<&Error>) -> Result<QueryResult> {
        let p = self.handle(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.transaction().map_err(db_error)?;
        // The final mutable-selection fence and publication share one transaction.
        let result = read_result(&tx, pid, id)?;
        if result.state != ResultState::Running {
            clear_input_references(&tx, id)?;
            crate::query_cache::rollback_revision(&tx, id)?;
            tx.commit().map_err(db_error)?;
            return Ok(result);
        }
        let scope_error = if error.is_none() && result.state == ResultState::Running {
            validate_input(&tx, pid, &result.spec).err()
        } else {
            None
        };
        let (status, message) = match error.or(scope_error.as_ref()) {
            None => ("ready", None),
            Some(e) => (
                match e.code {
                    "INTERRUPTED" => "interrupted",
                    "CANCELLED" => "cancelled",
                    _ => "failed",
                },
                Some(e.to_string()),
            ),
        };
        if tx.execute("UPDATE query_results SET status=?2,error=?3,count=CASE WHEN ?2='ready' THEN count ELSE NULL END WHERE id=?1 AND status='running'",params![id,status,message]).map_err(db_error)?>0 { event(&tx,"result.changed",id)?; }
        clear_input_references(&tx, id)?;
        if status == "ready" {
            tx.execute("UPDATE query_families SET latest_revision=(SELECT member_revision FROM query_results WHERE id=?1),prune_pending=1,latest_result_id=?1,latest_count=(SELECT COALESCE(count,0) FROM query_results WHERE id=?1),post_ready=(SELECT post_ready FROM query_results WHERE id=?1),touched_at=CAST(?2 AS INTEGER) WHERE id=(SELECT family_id FROM query_results WHERE id=?1)",params![id,now()]).map_err(db_error)?;
        } else {
            crate::query_cache::rollback_revision(&tx, id)?;
        }
        let result = read_result(&tx, pid, id)?;
        tx.commit().map_err(db_error)?;
        Ok(result)
    }
}

pub(super) fn validate_input(
    db: &Connection,
    pid: &str,
    spec: &QuerySpec,
) -> Result<Option<scopes::ResolvedScope>> {
    match &spec.input_scope {
        None => Ok(None),
        Some(scope) => {
            scope.validate_project(pid)?;
            if matches!(scope.target, ScopeTarget::Source { .. }) {
                Ok(None)
            } else {
                scopes::resolve(db, pid, scope).map(Some)
            }
        }
    }
}

pub(super) fn input_references(
    db: &Connection,
    pid: &str,
    kind: &str,
    owner: &str,
    spec: &QuerySpec,
) -> Result<()> {
    db.execute(
        "DELETE FROM result_references WHERE owner_kind=?1 AND owner_id=?2",
        params![kind, owner],
    )
    .map_err(db_error)?;
    if let Some(scope) = validate_input(db, pid, spec)? {
        for id in scope.results {
            db.execute(
                "INSERT OR IGNORE INTO result_references VALUES (?1,?2,?3)",
                params![kind, owner, id],
            )
            .map_err(db_error)?;
        }
    }
    Ok(())
}

pub(super) fn clear_input_references(db: &Connection, id: &str) -> Result<()> {
    db.execute(
        "DELETE FROM result_references WHERE owner_kind='query_input' AND owner_id=?1",
        [id],
    )
    .map_err(db_error)?;
    Ok(())
}

impl SqliteStore {
    pub fn query_input_count(&self, pid: &str, spec: &QuerySpec) -> Result<Option<u64>> {
        let p = self.handle(pid)?;
        let db = p.read()?;
        Ok(validate_input(&db, pid, spec)?.map(|scope| scope.count))
    }

    pub fn query_input_keys(
        &self,
        pid: &str,
        spec: &QuerySpec,
        source_id: &str,
        after: Option<&str>,
    ) -> Result<Vec<AssetKey>> {
        let p = self.handle(pid)?;
        let db = p.read()?;
        let scope = validate_input(&db, pid, spec)?
            .ok_or_else(|| Error::invalid("该查询没有固定的项目范围"))?;
        let mut stmt = db.prepare(&format!("SELECT source_id,asset_id FROM ({}) WHERE source_id=?1 AND asset_id>?2 ORDER BY asset_id LIMIT 512",scope.sql)).map_err(db_error)?;
        stmt.query_map(params![source_id, after.unwrap_or("")], |r| {
            Ok(AssetKey {
                source_id: r.get(0)?,
                asset_id: r.get(1)?,
            })
        })
        .map_err(db_error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_error)
    }

    pub fn filter_query_input(
        &self,
        pid: &str,
        spec: &QuerySpec,
        keys: &[AssetKey],
    ) -> Result<Vec<AssetKey>> {
        if keys.len() > 512 {
            return Err(Error::invalid("查询范围批次超过 512 个对象"));
        }
        if spec.input_scope.is_none()
            || spec
                .input_scope
                .as_ref()
                .is_some_and(|s| matches!(s.target, ScopeTarget::Source { .. }))
        {
            return Ok(keys.to_vec());
        }
        let p = self.handle(pid)?;
        let db = p.read()?;
        let scope = validate_input(&db, pid, spec)?.expect("project scope validated");
        let mut stmt = db
            .prepare(&format!(
                "SELECT 1 FROM ({}) WHERE source_id=?1 AND asset_id=?2 LIMIT 1",
                scope.sql
            ))
            .map_err(db_error)?;
        let mut kept = Vec::new();
        for key in keys {
            if stmt
                .exists(params![key.source_id, key.asset_id])
                .map_err(db_error)?
            {
                kept.push(key.clone());
            }
        }
        Ok(kept)
    }
}
