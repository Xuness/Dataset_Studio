use super::*;
use crate::{SqliteStore, lock_error};
use std::sync::Arc;

impl SqliteStore {
    pub fn evaluation(&self, pid: &str) -> Result<Arc<EvaluationDb>> {
        let p = self.handle(pid)?;
        let mut slot = p.evaluation.lock().map_err(lock_error)?;
        if let Some(db) = slot.as_ref() {
            return Ok(db.clone());
        }
        let path = p.project.directory.join("evaluation.sqlite");
        if path.exists()
            && path.canonicalize().map_err(Error::io)?.parent()
                != Some(p.project.directory.as_path())
        {
            return Err(Error::invalid("评审数据库必须位于项目目录内"));
        }
        let db = Arc::new(EvaluationDb::open(&path)?);
        *slot = Some(db.clone());
        Ok(db)
    }
    pub fn register_evaluation(&self, pid: &str, request: &AestheticCreate) -> Result<u64> {
        studio_application::aesthetic::validate_create(request)?;
        let p = self.handle(pid)?;
        self.mark_background(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.transaction().map_err(db_error)?;
        let json = encode(request)?;
        let previous: Option<(String, u64)> = tx
            .query_row(
                "SELECT request_json,total FROM evaluation_stage_refs WHERE id=?1",
                [&request.idempotency_key],
                |r| Ok((r.get(0)?, crate::unsigned(r, 1)?)),
            )
            .optional()
            .map_err(db_error)?;
        if let Some((old, total)) = previous {
            if old != json {
                return Err(Error::new(
                    "IDEMPOTENCY_CONFLICT",
                    "评审创建键已被不同请求使用",
                ));
            }
            return Ok(total);
        }
        if crate::management::removed(&tx, "workset", &request.collection_id)? {
            return Err(Error::new("OBJECT_REMOVED", "工作集已删除"));
        }
        let total: u64 = tx
            .query_row(
                "SELECT count FROM collections WHERE id=?1 AND NOT EXISTS(SELECT 1 FROM evaluation_workset_builds b WHERE b.collection_id=collections.id AND b.state!='ready')",
                [&request.collection_id],
                |r| crate::unsigned(r, 0),
            )
            .optional()
            .map_err(db_error)?
            .ok_or_else(|| Error::new("NOT_FOUND", "工作集不存在"))?;
        if total == 0 {
            return Err(Error::invalid("评审需要非空工作集"));
        }
        tx.execute(
            "INSERT INTO evaluation_stage_refs VALUES (?1,?2,'preparing',?3,?4)",
            params![
                request.idempotency_key,
                request.collection_id,
                json,
                total as i64
            ],
        )
        .map_err(db_error)?;
        // Number of project sources is bounded; never DISTINCT-scan all workset members.
        tx.execute("INSERT INTO evaluation_source_refs SELECT ?1,s.id FROM sources s WHERE EXISTS(SELECT 1 FROM collection_members c WHERE c.collection_id=?2 AND c.source_id=s.id)",params![request.idempotency_key,request.collection_id]).map_err(db_error)?;
        crate::event(&tx, "evaluation.created", &request.idempotency_key)?;
        tx.commit().map_err(db_error)?;
        Ok(total)
    }
    pub fn sync_evaluation(&self, pid: &str, stage: &AestheticStage) -> Result<()> {
        let p = self.handle(pid)?;
        if matches!(
            stage.state.as_str(),
            "preparing" | "running" | "pausing" | "cancelling"
        ) {
            self.mark_background(pid)?;
        }
        let mut db = p.db.lock().map_err(lock_error)?;
        // Read the authoritative state while holding the projection writer, so a delayed
        // HTTP completion cannot overwrite a more recent executor transition.
        let current = self.evaluation(pid)?.stage(&stage.id)?;
        let stage = &current;
        let tx = db.transaction().map_err(db_error)?;
        tx.execute("UPDATE evaluation_stage_refs SET state=?2,collection_id=CASE WHEN ?3 THEN NULL ELSE collection_id END WHERE id=?1",params![stage.id,stage.state,stage.frozen==stage.total||stage.state=="cancelled"]).map_err(db_error)?;
        if matches!(stage.state.as_str(), "completed" | "cancelled") {
            tx.execute(
                "DELETE FROM evaluation_source_refs WHERE stage_id=?1",
                [&stage.id],
            )
            .map_err(db_error)?;
        }
        crate::event(&tx, "evaluation.changed", &stage.id)?;
        tx.commit().map_err(db_error)
    }
}

pub(crate) fn recover(directory: &Path, project: &Connection) -> Result<Option<Arc<EvaluationDb>>> {
    let path = directory.join("evaluation.sqlite");
    let refs: bool = project
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM evaluation_stage_refs)",
            [],
            |r| r.get(0),
        )
        .map_err(db_error)?;
    if !path.exists() {
        if refs {
            return Err(Error::new(
                "EVALUATION_MISSING",
                "项目缺少付费评审账本 evaluation.sqlite，请恢复完整备份",
            ));
        }
        return Ok(None);
    }
    if path.canonicalize().map_err(Error::io)?.parent() != Some(directory) {
        return Err(Error::invalid("评审数据库必须位于项目目录内"));
    }
    let db = Arc::new(EvaluationDb::open(&path)?);
    let mut stmt = project
        .prepare("SELECT id FROM evaluation_stage_refs")
        .map_err(db_error)?;
    let ids = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(db_error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_error)?;
    for id in ids {
        match db.stage(&id) {
            Ok(stage) => {
                project.execute("UPDATE evaluation_stage_refs SET state=?2,collection_id=CASE WHEN ?3 THEN NULL ELSE collection_id END WHERE id=?1",params![id,stage.state,stage.frozen==stage.total||stage.state=="cancelled"]).map_err(db_error)?;
                if matches!(stage.state.as_str(), "cancelled" | "completed") {
                    project
                        .execute(
                            "DELETE FROM evaluation_source_refs WHERE stage_id=?1",
                            [&id],
                        )
                        .map_err(db_error)?;
                }
            }
            Err(e) if e.code == "NOT_FOUND" => {
                project.execute("UPDATE evaluation_stage_refs SET state='failed',collection_id=NULL WHERE id=?1",[&id]).map_err(db_error)?;
                project
                    .execute(
                        "DELETE FROM evaluation_source_refs WHERE stage_id=?1",
                        [&id],
                    )
                    .map_err(db_error)?;
            }
            Err(e) => return Err(e),
        }
    }
    let mut after = String::new();
    loop {
        let jobs = db.analysis_jobs(&after, None, 50)?;
        if jobs.is_empty() {
            break;
        }
        for item in jobs {
            project.execute("INSERT INTO evaluation_analysis_refs VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET state=excluded.state",params![item.id,item.state]).map_err(db_error)?;
            after = item.id;
        }
    }
    Ok(Some(db))
}

pub(crate) fn reference_reason(
    db: &Connection,
    kind: studio_domain::ObjectKind,
    id: &str,
) -> Result<bool> {
    let sql = match kind {
        studio_domain::ObjectKind::Workset => {
            "SELECT EXISTS(SELECT 1 FROM evaluation_stage_refs WHERE collection_id=?1)"
        }
        studio_domain::ObjectKind::Source => {
            "SELECT EXISTS(SELECT 1 FROM evaluation_source_refs WHERE source_id=?1)"
        }
        _ => return Ok(false),
    };
    db.query_row(sql, [id], |r| r.get(0)).map_err(db_error)
}
