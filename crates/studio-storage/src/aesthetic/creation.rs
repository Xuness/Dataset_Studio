use super::*;
use crate::ProjectTransaction;
use crate::{SqliteStore, lock_error};
use studio_application::aesthetic::{same_request, validate_capacity};

/// Worksets are immutable. The token also binds count and project source locations.
fn input(db: &Connection, collection: &str) -> Result<(u64, String)> {
    if crate::management::removed(db, "workset", collection)? {
        return Err(Error::new("OBJECT_REMOVED", "工作集已删除"));
    }
    let total = db.query_row("SELECT count FROM collections WHERE id=?1 AND NOT EXISTS(SELECT 1 FROM evaluation_workset_builds b WHERE b.collection_id=collections.id AND b.state!='ready')",[collection],|r|crate::unsigned(r,0)).optional().map_err(db_error)?
        .ok_or_else(||Error::new("NOT_FOUND","工作集不存在"))?;
    let mut statement = db.prepare("SELECT s.json FROM sources s WHERE EXISTS(SELECT 1 FROM collection_members c WHERE c.collection_id=?1 AND c.source_id=s.id) ORDER BY s.id").map_err(db_error)?;
    let sources = statement
        .query_map([collection], |r| r.get::<_, String>(0))
        .map_err(db_error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_error)?;
    Ok((total, hash(&encode(&(collection, total, sources))?)))
}

pub(super) fn read_intent(db: &Connection, id: &str) -> Result<Option<AestheticCreationIntent>> {
    let row: Option<(String,u64,String)> = db.query_row("SELECT i.config,r.total,i.state FROM evaluation_creation_intents i JOIN evaluation_stage_refs r ON r.id=i.stage_id WHERE i.stage_id=?1",[id],|r|Ok((r.get(0)?,crate::unsigned(r,1)?,r.get(2)?))).optional().map_err(db_error)?;
    row.map(|(config, total, state)| {
        Ok(AestheticCreationIntent {
            config: decode(config)?,
            total,
            state,
        })
    })
    .transpose()
}

pub(super) fn restore_references(db: &Connection, intent: &AestheticCreationIntent) -> Result<()> {
    let request = &intent.config.request;
    let (total, _) = input(db, &request.collection_id)?;
    if total != intent.total {
        return Err(Error::new("SOURCE_CHANGED", "创建意图的工作集总量已变化"));
    }
    db.execute(
        "UPDATE evaluation_stage_refs SET collection_id=?2,state='preparing' WHERE id=?1",
        params![request.idempotency_key, request.collection_id],
    )
    .map_err(db_error)?;
    // References are restored from the frozen source list, never from an untrusted retry.
    for source in &intent.config.sources {
        db.execute(
            "INSERT OR IGNORE INTO evaluation_source_refs(stage_id,source_id) VALUES (?1,?2)",
            params![request.idempotency_key, source.source_id],
        )
        .map_err(db_error)?;
    }
    Ok(())
}

impl SqliteStore {
    pub fn materialize_evaluation(&self, pid: &str, id: &str) -> Result<AestheticStage> {
        let p = self.handle(pid)?;
        let ledger = self.evaluation(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.project_transaction().map_err(db_error)?;
        let intent = read_intent(&tx, id)?
            .ok_or_else(|| Error::new("EVALUATION_CREATION_INCOMPLETE", "缺少冻结创建意图"))?;
        if matches!(intent.state.as_str(), "abandoned" | "cancelled") {
            return Err(Error::new("OBJECT_REMOVED", "创建意图已放弃"));
        }
        if intent.state == "pending" {
            restore_references(&tx, &intent)?;
        } else if let Err(e) = ledger.stage(id) {
            return Err(if e.code == "NOT_FOUND" {
                Error::new("EVALUATION_MISSING", "已建成阶段缺少账本记录")
            } else {
                e
            });
        }
        let stage = ledger.create(intent.config, intent.total)?;
        crate::faults::check("create_ledger_committed", id)?;
        super::project::project_stage(&tx, &stage)?;
        crate::event(&tx, "evaluation.changed", id)?;
        tx.commit().map_err(db_error)?;
        if stage.state == "cancelled" {
            return Err(Error::new("OBJECT_REMOVED", "阶段已取消，请使用新创建键"));
        }
        Ok(stage)
    }
    pub fn evaluation_input(&self, pid: &str, collection: &str) -> Result<(u64, String)> {
        let p = self.handle(pid)?;
        input(&*p.db.lock().map_err(lock_error)?, collection)
    }
    pub fn evaluation_intent(
        &self,
        pid: &str,
        id: &str,
    ) -> Result<Option<AestheticCreationIntent>> {
        let p = self.handle(pid)?;
        read_intent(&*p.db.lock().map_err(lock_error)?, id)
    }
    pub fn register_evaluation(
        &self,
        pid: &str,
        config: AestheticConfig,
        project_version: &str,
    ) -> Result<AestheticCreationIntent> {
        let request = &config.request;
        studio_application::aesthetic::validate_create(request)?;
        let p = self.handle(pid)?;
        self.mark_background(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.project_transaction().map_err(db_error)?;
        if let Some(intent) = read_intent(&tx, &request.idempotency_key)? {
            same_request(&intent.config.request, request)?;
            if matches!(intent.state.as_str(), "abandoned" | "cancelled") {
                return Err(Error::new(
                    "OBJECT_REMOVED",
                    "创建意图已明确放弃，请使用新创建键",
                ));
            }
            return Ok(intent);
        }
        let legacy: Option<String> = tx
            .query_row(
                "SELECT request_json FROM evaluation_stage_refs WHERE id=?1",
                [&request.idempotency_key],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?;
        if let Some(legacy) = legacy {
            same_request(&decode(legacy)?, request)?;
            let cancelled: bool = tx
                .query_row(
                    "SELECT state='cancelled' FROM evaluation_stage_refs WHERE id=?1",
                    [&request.idempotency_key],
                    |r| r.get(0),
                )
                .map_err(db_error)?;
            if cancelled {
                return Err(Error::new("OBJECT_REMOVED", "历史创建键已明确放弃"));
            }
            return Err(Error::new(
                "EVALUATION_CREATION_INCOMPLETE",
                "历史创建缺少冻结配置；请恢复账本，或明确取消此创建键后新建阶段",
            ));
        }
        let (total, version) = input(&tx, &request.collection_id)?;
        validate_capacity(total)?;
        if version != project_version {
            return Err(Error::new(
                "EVALUATION_INPUT_CHANGED",
                "预检后项目输入已变化，请重新预检",
            ));
        }
        tx.execute("INSERT INTO evaluation_stage_refs(id,collection_id,state,request_json,total) VALUES (?1,?2,'preparing',?3,?4)",params![request.idempotency_key,request.collection_id,encode(request)?,total as i64]).map_err(db_error)?;
        tx.execute("INSERT INTO evaluation_creation_intents(stage_id,config,state,created_at) VALUES (?1,?2,'pending',?3)",params![request.idempotency_key,encode(&config)?,now()]).map_err(db_error)?;
        let intent = AestheticCreationIntent {
            config,
            total,
            state: "pending".into(),
        };
        restore_references(&tx, &intent)?;
        crate::event(
            &tx,
            "evaluation.created",
            &intent.config.request.idempotency_key,
        )?;
        tx.commit().map_err(db_error)?;
        crate::faults::check(
            "create_project_committed",
            &intent.config.request.idempotency_key,
        )?;
        Ok(intent)
    }
    /// Only used when a ledger stage is absent. Materialized records require restoration.
    pub fn abandon_evaluation_creation(&self, pid: &str, id: &str) -> Result<()> {
        let p = self.handle(pid)?;
        let ledger = self.evaluation(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.project_transaction().map_err(db_error)?;
        match ledger.stage(id) {
            Ok(_) => {
                return Err(Error::new(
                    "REVISION_CONFLICT",
                    "阶段已建成，请使用阶段取消操作",
                ));
            }
            Err(e) if e.code == "NOT_FOUND" => {}
            Err(e) => return Err(e),
        }
        if let Some(intent) = read_intent(&tx, id)?
            && intent.state == "materialized"
        {
            return Err(Error::new(
                "EVALUATION_MISSING",
                "已建成阶段缺少账本，请恢复备份",
            ));
        }
        let changed = tx
            .execute(
                "UPDATE evaluation_stage_refs SET state='cancelled',collection_id=NULL WHERE id=?1",
                [id],
            )
            .map_err(db_error)?;
        if changed == 0 {
            return Err(Error::new("NOT_FOUND", "创建意图不存在"));
        }
        tx.execute(
            "UPDATE evaluation_creation_intents SET state='abandoned' WHERE stage_id=?1",
            [id],
        )
        .map_err(db_error)?;
        tx.execute("DELETE FROM evaluation_source_refs WHERE stage_id=?1", [id])
            .map_err(db_error)?;
        crate::event(&tx, "evaluation.abandoned", id)?;
        tx.commit().map_err(db_error)
    }
}
