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
        project_stage(&tx, stage)?;
        crate::event(&tx, "evaluation.changed", &stage.id)?;
        crate::faults::check("project_sync_before_commit", &stage.id)?;
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
        if !refs {
            return Ok(None);
        }
        let unsafe_missing: bool = project.query_row("SELECT EXISTS(SELECT 1 FROM evaluation_stage_refs r LEFT JOIN evaluation_creation_intents i ON i.stage_id=r.id WHERE i.state IS NULL OR i.state IN ('materialized','cancelled'))",[],|r|r.get(0)).map_err(db_error)?;
        if unsafe_missing {
            return Err(Error::new(
                "EVALUATION_MISSING",
                "项目缺少付费评审账本 evaluation.sqlite，请恢复完整备份",
            ));
        }
    }
    if path.exists() && path.canonicalize().map_err(Error::io)?.parent() != Some(directory) {
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
        let intent = super::creation::read_intent(project, &id)?;
        match db.stage(&id) {
            Ok(stage) => {
                if intent
                    .as_ref()
                    .is_some_and(|i| matches!(i.state.as_str(), "abandoned" | "cancelled"))
                    && stage.state != "cancelled"
                {
                    return Err(Error::new(
                        "EVALUATION_RECONCILIATION_FAILED",
                        "已放弃的创建意图与账本不一致",
                    ));
                }
                let tx = project.unchecked_transaction().map_err(db_error)?;
                project_stage(&tx, &stage)?;
                tx.commit().map_err(db_error)?;
            }
            Err(e) if e.code == "NOT_FOUND" => {
                match intent {
                    Some(intent) if intent.state == "pending" => {
                        let tx = project.unchecked_transaction().map_err(db_error)?;
                        super::creation::restore_references(&tx, &intent)?;
                        let stage = db.create(intent.config, intent.total)?;
                        db.control(&stage.id, "pause")?;
                        let stage = db.settle(&stage.id, None)?;
                        project_stage(&tx, &stage)?;
                        tx.commit().map_err(db_error)?;
                    }
                    Some(intent) if intent.state == "abandoned" => {}
                    Some(_) => {
                        return Err(Error::new(
                            "EVALUATION_MISSING",
                            "已建成阶段缺少账本记录，请恢复完整备份",
                        ));
                    }
                    None => {
                        let cancelled: bool = project
                            .query_row(
                                "SELECT state='cancelled' FROM evaluation_stage_refs WHERE id=?1",
                                [&id],
                                |r| r.get(0),
                            )
                            .map_err(db_error)?;
                        if cancelled {
                            continue;
                        }
                        // Old versions did not store frozen configuration. Keep protection;
                        // do not invent a replacement paid stage or release its references.
                        project.execute("UPDATE evaluation_stage_refs SET state='needs_attention' WHERE id=?1",[&id]).map_err(db_error)?;
                    }
                }
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

pub(super) fn project_stage(project: &Connection, stage: &AestheticStage) -> Result<()> {
    let released = studio_application::aesthetic::terminal(&stage.state);
    let (request, total): (String, u64) = project
        .query_row(
            "SELECT request_json,total FROM evaluation_stage_refs WHERE id=?1",
            [&stage.id],
            |r| Ok((r.get(0)?, crate::unsigned(r, 1)?)),
        )
        .map_err(db_error)?;
    studio_application::aesthetic::same_request(&decode(request)?, &stage.config.request)?;
    if total != stage.total {
        return Err(Error::new(
            "EVALUATION_RECONCILIATION_FAILED",
            "两库的阶段总量不一致",
        ));
    }
    if !released {
        if stage.frozen < stage.total {
            super::creation::restore_references(
                project,
                &AestheticCreationIntent {
                    config: stage.config.clone(),
                    total: stage.total,
                    state: "pending".into(),
                },
            )?;
        }
        for source in &stage.config.sources {
            project
                .execute(
                    "INSERT OR IGNORE INTO evaluation_source_refs VALUES (?1,?2)",
                    params![stage.id, source.source_id],
                )
                .map_err(db_error)?;
        }
    }
    project.execute("UPDATE evaluation_stage_refs SET state=?2,collection_id=CASE WHEN ?3 THEN NULL ELSE ?4 END WHERE id=?1",params![stage.id,stage.state,stage.frozen==stage.total||stage.state=="cancelled",stage.config.request.collection_id]).map_err(db_error)?;
    if released {
        project
            .execute(
                "DELETE FROM evaluation_source_refs WHERE stage_id=?1",
                [&stage.id],
            )
            .map_err(db_error)?;
    }
    project.execute("INSERT INTO evaluation_creation_intents(stage_id,config,state,created_at) VALUES (?1,?2,?3,?4) ON CONFLICT(stage_id) DO UPDATE SET state=excluded.state",params![stage.id,encode(&stage.config)?,if stage.state=="cancelled" {"cancelled"} else {"materialized"},now()]).map_err(db_error)?;
    Ok(())
}
