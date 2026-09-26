use crate::*;
use studio_application::ManagementRepository;

pub(super) fn display_name(
    db: &Connection,
    kind: &str,
    id: &str,
    fallback: &str,
) -> Result<String> {
    Ok(db
        .query_row(
            "SELECT name FROM object_metadata WHERE kind=?1 AND id=?2",
            params![kind, id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(db_error)?
        .flatten()
        .unwrap_or_else(|| fallback.into()))
}
pub(super) fn removed(db: &Connection, kind: &str, id: &str) -> Result<bool> {
    db.query_row(
        "SELECT COALESCE((SELECT deleted FROM object_metadata WHERE kind=?1 AND id=?2),0)",
        params![kind, id],
        |r| r.get(0),
    )
    .map_err(db_error)
}
pub(super) fn created(db: &Connection, kind: &str, id: &str) -> Result<()> {
    db.execute(
        "INSERT OR IGNORE INTO object_metadata(kind,id,created_at,updated_at) VALUES (?1,?2,?3,?3)",
        params![kind, id, now()],
    )
    .map_err(db_error)?;
    Ok(())
}
pub(super) fn named(db: &Connection, kind: &str, id: &str, name: &str) -> Result<()> {
    db.execute("INSERT INTO object_metadata(kind,id,name,revision,created_at,updated_at) VALUES (?1,?2,?3,1,?4,?4) ON CONFLICT(kind,id) DO UPDATE SET name=excluded.name,revision=revision+1,updated_at=excluded.updated_at",params![kind,id,name,now()]).map_err(db_error)?;
    Ok(())
}
pub(super) fn view(kind: ObjectKind) -> Result<String> {
    let (table, name, state, count, created, bytes, subtype) = match kind {
        ObjectKind::Source => (
            "sources",
            "json_extract(t.json,'$.name')",
            "CASE WHEN COALESCE(m.deleted,0)=1 THEN 'detached' ELSE 'attached' END",
            "NULL",
            "m.created_at",
            "NULL",
            "json_extract(t.json,'$.kind')",
        ),
        ObjectKind::Workset => (
            "(SELECT * FROM collections WHERE NOT EXISTS(SELECT 1 FROM evaluation_workset_builds b WHERE b.collection_id=collections.id AND b.state!='ready'))",
            "t.name",
            "'ready'",
            "t.count",
            "m.created_at",
            "NULL",
            "NULL",
        ),
        ObjectKind::Artifact => (
            "artifacts",
            "t.name",
            "t.status",
            "t.count",
            "t.created_at",
            "CAST((SELECT SUM(json_extract(value,'$.bytes')) FROM json_each(t.files_json)) AS TEXT)",
            "t.kind",
        ),
        ObjectKind::Query => (
            "query_definitions",
            "t.name",
            "CASE WHEN COALESCE(m.deleted,0)=1 THEN 'deleted' ELSE 'saved' END",
            "NULL",
            "t.created_at",
            "NULL",
            "NULL",
        ),
        ObjectKind::QueryResult => (
            "query_results",
            "COALESCE((SELECT COALESCE(qm.name,q.name) FROM query_definitions q LEFT JOIN object_metadata qm ON qm.kind='query' AND qm.id=q.id WHERE q.id=t.definition_id),'临时查询结果')",
            "t.status",
            "t.count",
            "t.created_at",
            "NULL",
            "NULL",
        ),
        ObjectKind::Job => (
            "jobs",
            "CASE t.operator WHEN 'danbooru.metarecall' THEN 'Danbooru 元数据排名' WHEN 'core.manifest' THEN '数据清单' WHEN 'core.scalar' THEN '标量计算' ELSE t.operator END",
            "CASE WHEN COALESCE(m.deleted,0)=1 THEN 'deleted' ELSE t.status END",
            "t.total",
            "t.created_at",
            "NULL",
            "t.operator",
        ),
        _ => return Err(Error::invalid("该对象类型没有分页列表")),
    };
    Ok(format!(
        "SELECT t.id,COALESCE(m.name,{name}) AS name,COALESCE(m.notes,'') AS notes,COALESCE(m.revision,0) AS revision,{state} AS state,COALESCE(m.archived,0) AS archived,{count} AS count,{created} AS created_at,m.updated_at,{bytes} AS bytes,{subtype} AS subtype,COALESCE(m.deleted,0) AS deleted FROM {table} t LEFT JOIN object_metadata m ON m.kind='{}' AND m.id=t.id",
        kind.key()
    ))
}
fn row(kind: ObjectKind, r: &rusqlite::Row) -> rusqlite::Result<ManagedObject> {
    Ok(ManagedObject {
        kind,
        id: r.get(0)?,
        name: r.get(1)?,
        notes: r.get(2)?,
        revision: unsigned(r, 3)?,
        state: r.get(4)?,
        archived: r.get(5)?,
        count: r.get::<_, Option<i64>>(6)?.map(|n| n.max(0) as u64),
        created_at: r.get(7)?,
        updated_at: r.get(8)?,
        bytes: r.get(9)?,
        subtype: r.get(10)?,
    })
}
pub(super) fn read(
    db: &Connection,
    project: &Project,
    kind: ObjectKind,
    id: &str,
) -> Result<ManagedObject> {
    if matches!(kind, ObjectKind::Selection | ObjectKind::SelectionHistory) {
        if id != "selection" {
            return Err(Error::new("NOT_FOUND", "选择对象不存在"));
        }
        return Ok(ManagedObject {
            kind,
            id: id.into(),
            name: if kind == ObjectKind::Selection {
                "当前选择"
            } else {
                "选择撤销历史"
            }
            .into(),
            notes: String::new(),
            revision: selection::read(db)?.revision,
            state: "ready".into(),
            archived: false,
            count: Some(selection::read(db)?.count),
            created_at: None,
            updated_at: None,
            bytes: None,
            subtype: None,
        });
    }
    validate_id(id)?;
    if kind == ObjectKind::Project {
        if id != project.id {
            return Err(Error::new("NOT_FOUND", "项目对象不存在"));
        }
        let meta = db.query_row("SELECT name,notes,revision,updated_at FROM object_metadata WHERE kind='project' AND id=?1",[id],|r|Ok((r.get::<_,Option<String>>(0)?,r.get::<_,String>(1)?,unsigned(r,2)?,r.get::<_,Option<String>>(3)?))).optional().map_err(db_error)?;
        let (name, notes, revision, updated_at) = meta.unwrap_or((None, String::new(), 0, None));
        return Ok(ManagedObject {
            kind,
            id: id.into(),
            name: name.unwrap_or_else(|| project.name.clone()),
            notes,
            revision,
            state: "open".into(),
            archived: false,
            count: None,
            created_at: Some(project.created_at.clone()),
            updated_at,
            bytes: None,
            subtype: None,
        });
    }
    db.query_row(&format!("SELECT id,name,notes,revision,state,archived,count,created_at,updated_at,bytes,subtype FROM ({}) WHERE id=?1",view(kind)?),[id],|r|row(kind,r)).optional().map_err(db_error)?.ok_or_else(||Error::new("NOT_FOUND","对象已删除或不属于当前项目"))
}
pub(super) fn check(
    db: &Connection,
    project: &Project,
    kind: ObjectKind,
    id: &str,
    expected: u64,
) -> Result<ManagedObject> {
    let item = read(db, project, kind, id)?;
    if expected != item.revision {
        return Err(Error::new(
            "REVISION_CONFLICT",
            "对象刚被其他操作修改，请重新载入详情",
        ));
    }
    Ok(item)
}
pub(super) fn removable(
    db: &Connection,
    pid: &str,
    item: &ManagedObject,
) -> Result<Option<String>> {
    if crate::aesthetic::reference_reason(db, item.kind, &item.id)? {
        return Ok(Some("美学评审仍使用此对象，请先完成或取消相关阶段".into()));
    }
    if matches!(
        item.kind,
        ObjectKind::Project | ObjectKind::Selection | ObjectKind::SelectionHistory
    ) {
        return Ok(Some("此对象不通过删除操作清理".into()));
    }
    if matches!(
        item.state.as_str(),
        "queued" | "waiting_input" | "preparing" | "running" | "publishing"
    ) {
        return Ok(Some("请先取消或等待正在执行的工作结束".into()));
    }
    let incoming = crate::object_links::page(db, pid, item.kind, &item.id, true, None, 1)?;
    if incoming.items.iter().any(|link| link.blocking) {
        return Ok(Some(
            "仍有对象需要使用它，请先处理“使用它的对象”中列出的引用".into(),
        ));
    }
    Ok(None)
}

#[derive(Serialize, Deserialize)]
struct ListCursor {
    signature: String,
    value: String,
    id: String,
}

impl ManagementRepository for SqliteStore {
    fn managed_objects(&self, pid: &str, listing: ObjectListing) -> Result<ObjectPage> {
        let p = self.handle(pid)?;
        let search = listing.search.trim();
        if search.chars().count() > 120 {
            return Err(Error::invalid("搜索内容最多 120 个字符"));
        }
        let (sort, descending) = match listing.order.as_str() {
            "name_asc" => ("lower(name)", false),
            "name_desc" => ("lower(name)", true),
            "created_asc" => (
                "printf('%020d',CAST(COALESCE(created_at,'0') AS INTEGER))",
                false,
            ),
            "created_desc" | "" => (
                "printf('%020d',CAST(COALESCE(created_at,'0') AS INTEGER))",
                true,
            ),
            "count_desc" => ("printf('%020d',COALESCE(count,0))", true),
            _ => return Err(Error::invalid("不支持的对象列表排序")),
        };
        let signature = serde_json::to_string(&(
            pid,
            listing.kind,
            search,
            &listing.order,
            &listing.state,
            &listing.subtype,
            listing.include_archived,
        ))
        .map_err(Error::io)?;
        let cursor = listing
            .after
            .as_ref()
            .map(|raw| -> Result<ListCursor> {
                if raw.len() > 4096 {
                    return Err(Error::invalid("列表游标无效"));
                }
                let cursor: ListCursor =
                    serde_json::from_str(raw).map_err(|_| Error::invalid("列表游标无效"))?;
                if cursor.signature != signature {
                    return Err(Error::invalid("列表条件已变化，请返回第一页"));
                }
                Ok(cursor)
            })
            .transpose()?;
        let default_state = match listing.kind {
            ObjectKind::Source => "state='attached'",
            ObjectKind::Artifact => "state!='released'",
            ObjectKind::Query | ObjectKind::Job => "deleted=0",
            _ => "1",
        };
        let visible = match listing.state.as_str() {
            "" => default_state,
            "active" => "state IN ('queued','waiting_input','preparing','running','publishing')",
            "archived" => "archived=1 AND deleted=0",
            _ => "(?3='all' OR state=?3)",
        };
        let direction = if descending { "DESC" } else { "ASC" };
        let comparison = if descending { "<" } else { ">" };
        let sql = format!(
            "WITH objects AS ({view}),ordered AS (SELECT *,{sort} AS sort_key FROM objects) SELECT id,name,notes,revision,state,archived,count,created_at,updated_at,bytes,subtype,sort_key FROM ordered WHERE {visible} AND (?1='' OR name LIKE ?2 ESCAPE '\\' OR notes LIKE ?2 ESCAPE '\\' OR id LIKE ?2 ESCAPE '\\') AND (?4 OR archived=0) AND (?5 IS NULL OR subtype=?5) AND (?6 IS NULL OR (sort_key,id){comparison}(?6,?7)) ORDER BY sort_key {direction},id {direction} LIMIT ?8",
            view = view(listing.kind)?
        );
        let limit = listing.limit.clamp(1, 128);
        let pattern = format!(
            "%{}%",
            search
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let db = p.read()?;
        let mut stmt = db.prepare(&sql).map_err(db_error)?;
        let mut found = stmt
            .query_map(
                params![
                    search,
                    pattern,
                    listing.state,
                    listing.include_archived,
                    listing.subtype,
                    cursor.as_ref().map(|c| &c.value),
                    cursor.as_ref().map(|c| &c.id),
                    limit as i64 + 1
                ],
                |r| Ok((row(listing.kind, r)?, r.get::<_, String>(11)?)),
            )
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        let more = found.len() > limit;
        found.truncate(limit);
        let next_cursor = if more {
            found
                .last()
                .map(|(item, value)| {
                    serde_json::to_string(&ListCursor {
                        signature,
                        value: value.clone(),
                        id: item.id.clone(),
                    })
                    .map_err(Error::io)
                })
                .transpose()?
        } else {
            None
        };
        Ok(ObjectPage {
            items: found.into_iter().map(|(item, _)| item).collect(),
            next_cursor,
        })
    }
    fn object_details(&self, pid: &str, kind: ObjectKind, id: &str) -> Result<ObjectDetails> {
        let p = self.handle(pid)?;
        let db = p.read()?;
        let object = read(&db, &p.project, kind, id)?;
        let incoming = crate::object_links::page(&db, pid, kind, id, true, None, 32)?;
        let outgoing = crate::object_links::page(&db, pid, kind, id, false, None, 32)?;
        let mut paths = Vec::new();
        let provenance = match kind {
            ObjectKind::Workset => db
                .query_row(
                    "SELECT provenance_json FROM collection_scopes WHERE collection_id=?1",
                    [id],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .map_err(db_error)?
                .map(|s| serde_json::from_str(&s).map_err(Error::io))
                .transpose()?
                .unwrap_or(serde_json::Value::Null),
            ObjectKind::Artifact => {
                let a = artifacts::read(&db, pid, id)?;
                paths.extend(a.files.iter().map(|f| {
                    p.project
                        .directory
                        .join(&f.path)
                        .to_string_lossy()
                        .into_owned()
                }));
                serde_json::to_value(a.provenance).map_err(Error::io)?
            }
            ObjectKind::Query => db
                .query_row(
                    "SELECT spec_json FROM query_definitions WHERE id=?1",
                    [id],
                    |r| r.get::<_, String>(0),
                )
                .map_err(db_error)
                .and_then(|s| serde_json::from_str(&s).map_err(Error::io))?,
            ObjectKind::QueryResult => db
                .query_row(
                    "SELECT spec_json FROM query_results WHERE id=?1",
                    [id],
                    |r| r.get::<_, String>(0),
                )
                .map_err(db_error)
                .and_then(|s| serde_json::from_str(&s).map_err(Error::io))?,
            ObjectKind::Job => db
                .query_row(
                    "SELECT provenance_json FROM job_scopes WHERE job_id=?1",
                    [id],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .map_err(db_error)?
                .map(|s| serde_json::from_str(&s).map_err(Error::io))
                .transpose()?
                .unwrap_or(serde_json::Value::Null),
            ObjectKind::Project => {
                paths.push(p.project.directory.to_string_lossy().into_owned());
                let (sources,worksets,artifacts):(u64,u64,u64)=db.query_row("SELECT (SELECT COUNT(*) FROM sources WHERE NOT EXISTS(SELECT 1 FROM object_metadata m WHERE m.kind='source' AND m.id=sources.id AND m.deleted=1)),(SELECT COUNT(*) FROM collections WHERE NOT EXISTS(SELECT 1 FROM evaluation_workset_builds b WHERE b.collection_id=collections.id AND b.state!='ready')),(SELECT COUNT(*) FROM artifacts WHERE status!='released')",[],|r|Ok((unsigned(r,0)?,unsigned(r,1)?,unsigned(r,2)?))).map_err(db_error)?;
                serde_json::json!({"sources":sources,"worksets":worksets,"artifacts":artifacts})
            }
            _ => serde_json::Value::Null,
        };
        let run = match kind {
            ObjectKind::Artifact => artifacts::read(&db, pid, id)?.provenance.run,
            ObjectKind::Job => db
                .query_row("SELECT run_json FROM job_runs WHERE job_id=?1", [id], |r| {
                    r.get::<_, String>(0)
                })
                .optional()
                .map_err(db_error)?
                .map(|s| serde_json::from_str(&s).map_err(Error::io))
                .transpose()?,
            _ => None,
        };
        let remove_reason = removable(&db, pid, &object)?;
        drop(db);
        if kind == ObjectKind::Source {
            let registry = self.registry.lock().map_err(lock_error)?;
            let raw: Option<String> = registry
                .query_row("SELECT json FROM source_locations WHERE id=?1", [id], |r| {
                    r.get(0)
                })
                .optional()
                .map_err(db_error)?;
            if let Some(raw) = raw {
                let source: Source = serde_json::from_str(&raw).map_err(Error::io)?;
                paths.extend(
                    [source.index_root, source.media_root]
                        .into_iter()
                        .flatten()
                        .map(|p| p.to_string_lossy().into_owned()),
                );
            }
        }
        Ok(ObjectDetails {
            object,
            incoming: incoming.items,
            outgoing: outgoing.items,
            incoming_total: incoming.total,
            outgoing_total: outgoing.total,
            incoming_cursor: incoming.next_cursor,
            outgoing_cursor: outgoing.next_cursor,
            provenance,
            paths,
            can_remove: remove_reason.is_none(),
            remove_reason,
            run,
        })
    }
    fn object_links(
        &self,
        pid: &str,
        kind: ObjectKind,
        id: &str,
        incoming: bool,
        after: Option<&str>,
        limit: usize,
    ) -> Result<ObjectLinkPage> {
        let p = self.handle(pid)?;
        let db = p.read()?;
        read(&db, &p.project, kind, id)?;
        crate::object_links::page(&db, pid, kind, id, incoming, after, limit)
    }
    fn edit_object(
        &self,
        pid: &str,
        kind: ObjectKind,
        id: &str,
        edit: EditObject,
    ) -> Result<ManagedObject> {
        if matches!(
            kind,
            ObjectKind::Selection | ObjectKind::SelectionHistory | ObjectKind::QueryResult
        ) {
            return Err(Error::invalid("此对象不能修改名称和备注"));
        }
        let name = validate_name(&edit.name)?;
        if edit.notes.chars().count() > 4000 {
            return Err(Error::invalid("备注最多 4000 个字符"));
        }
        let p = self.handle(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.project_transaction().map_err(db_error)?;
        let old = check(&tx, &p.project, kind, id, edit.expected_revision)?;
        if old.state == "deleted" {
            return Err(Error::new("NOT_FOUND", "对象已删除"));
        }
        named(&tx, kind.key(), id, &name)?;
        tx.execute(
            "UPDATE object_metadata SET notes=?3 WHERE kind=?1 AND id=?2",
            params![kind.key(), id, edit.notes.trim()],
        )
        .map_err(db_error)?;
        if kind == ObjectKind::Query {
            tx.execute(
                "UPDATE query_definitions SET name=?2,revision=revision+1 WHERE id=?1",
                params![id, name],
            )
            .map_err(db_error)?;
        }
        event(&tx, "object.changed", id)?;
        let item = read(&tx, &p.project, kind, id)?;
        tx.commit().map_err(db_error)?;
        drop(db);
        if kind == ObjectKind::Project {
            let project = self.project(pid)?;
            self.registry
                .lock()
                .map_err(lock_error)?
                .execute(
                    "UPDATE projects SET summary=?2 WHERE id=?1",
                    params![pid, serde_json::to_string(&project).map_err(Error::io)?],
                )
                .map_err(db_error)?;
        }
        Ok(item)
    }
    fn remove_object(&self, pid: &str, kind: ObjectKind, id: &str, expected: u64) -> Result<()> {
        if matches!(kind, ObjectKind::Artifact | ObjectKind::QueryResult) {
            return Err(Error::invalid("成果和查询结果通过专用释放服务清理"));
        }
        let p = self.handle(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.project_transaction().map_err(db_error)?;
        let item = check(&tx, &p.project, kind, id, expected)?;
        if let Some(reason) = removable(&tx, pid, &item)? {
            return Err(Error::new("OBJECT_IN_USE", reason));
        }
        match kind {
            ObjectKind::Source | ObjectKind::Query | ObjectKind::Job => {
                created(&tx, kind.key(), id)?;
                tx.execute("UPDATE object_metadata SET deleted=1,revision=revision+1,updated_at=?3 WHERE kind=?1 AND id=?2",params![kind.key(),id,now()]).map_err(db_error)?;
                if kind == ObjectKind::Query {
                    tx.execute("DELETE FROM artifact_references WHERE owner_kind='query_definition' AND owner_id=?1",[id]).map_err(db_error)?;
                    tx.execute("DELETE FROM result_references WHERE owner_kind='query_definition_input' AND owner_id=?1",[id]).map_err(db_error)?;
                }
                if kind == ObjectKind::Job {
                    for table in [
                        "job_input_bases",
                        "job_input_exclusions",
                        "job_input_legacy",
                    ] {
                        tx.execute(&format!("DELETE FROM {table} WHERE job_id=?1"), [id])
                            .map_err(db_error)?;
                    }
                    for table in ["result_references", "artifact_references"] {
                        tx.execute(
                            &format!("DELETE FROM {table} WHERE owner_kind IN ('job','job_input','job_scope') AND owner_id=?1"),
                            [id],
                        )
                        .map_err(db_error)?;
                    }
                }
            }
            ObjectKind::Workset => {
                tx.execute("INSERT OR IGNORE INTO retired_workset_requests SELECT request_id,request_json,collection_id,?2 FROM ranking_workset_requests WHERE collection_id=?1",params![id,now()]).map_err(db_error)?;
                for table in ["result_references", "artifact_references"] {
                    tx.execute(
                        &format!(
                            "DELETE FROM {table} WHERE owner_kind='collection' AND owner_id=?1"
                        ),
                        [id],
                    )
                    .map_err(db_error)?;
                }
                for table in [
                    "ranking_workset_requests",
                    "collection_scopes",
                    "collection_bases",
                    "collection_inclusions",
                    "collection_exclusions",
                    "collection_member_legacy",
                ] {
                    tx.execute(&format!("DELETE FROM {table} WHERE collection_id=?1"), [id])
                        .map_err(db_error)?;
                }
                tx.execute("DELETE FROM collections WHERE id=?1", [id])
                    .map_err(db_error)?;
                // Keep the tiny label tombstone for historical scope descriptions.
                created(&tx, "workset", id)?;
                tx.execute("UPDATE object_metadata SET name=?2,deleted=1,revision=revision+1,updated_at=?3 WHERE kind='workset' AND id=?1",params![id,item.name,now()]).map_err(db_error)?;
            }
            _ => return Err(Error::invalid("此对象不支持删除")),
        }
        crate::cache_cleanup::queue_unreferenced_inputs(&tx)?;
        event(
            &tx,
            if kind == ObjectKind::Source {
                "source.detached"
            } else {
                "object.removed"
            },
            id,
        )?;
        tx.commit().map_err(db_error)
    }
    fn archive_job(
        &self,
        pid: &str,
        id: &str,
        archived: bool,
        expected: u64,
    ) -> Result<ManagedObject> {
        let p = self.handle(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.project_transaction().map_err(db_error)?;
        let old = check(&tx, &p.project, ObjectKind::Job, id, expected)?;
        if !matches!(old.state.as_str(), "succeeded" | "failed" | "cancelled") {
            return Err(Error::invalid("只能整理已结束的任务记录"));
        }
        created(&tx, "job", id)?;
        tx.execute("UPDATE object_metadata SET archived=?2,revision=revision+1,updated_at=?3 WHERE kind='job' AND id=?1",params![id,archived,now()]).map_err(db_error)?;
        event(&tx, "object.changed", id)?;
        let result = read(&tx, &p.project, ObjectKind::Job, id)?;
        tx.commit().map_err(db_error)?;
        Ok(result)
    }
    fn restore_source(&self, pid: &str, id: &str, expected: u64) -> Result<ManagedObject> {
        let p = self.handle(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.project_transaction().map_err(db_error)?;
        check(&tx, &p.project, ObjectKind::Source, id, expected)?;
        tx.execute("UPDATE object_metadata SET deleted=0,revision=revision+1,updated_at=?2 WHERE kind='source' AND id=?1",params![id,now()]).map_err(db_error)?;
        event(&tx, "source.attached", id)?;
        let result = read(&tx, &p.project, ObjectKind::Source, id)?;
        tx.commit().map_err(db_error)?;
        Ok(result)
    }
}
impl SqliteStore {
    pub fn job_result_available(&self, pid: &str, id: &str) -> Result<bool> {
        let p = self.handle(pid)?;
        p.db.lock().map_err(lock_error)?.query_row("SELECT EXISTS(SELECT 1 FROM artifacts WHERE job_id=?1 AND output_id='data' AND status IN ('ready','legacy'))",[id],|r|r.get(0)).map_err(db_error)
    }
}
