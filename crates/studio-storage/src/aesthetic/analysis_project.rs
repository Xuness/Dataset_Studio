use super::*;
use crate::{SqliteStore, lock_error};
use studio_domain::{Collection, ScopeRef, ScopeTarget, aesthetic_analysis::*};

impl SqliteStore {
    pub fn sync_analysis(&self, pid: &str, id: &str) -> Result<()> {
        let p = self.handle(pid)?;
        self.mark_background(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let item = self.evaluation(pid)?.analysis_job(id)?;
        if matches!(item.state.as_str(), "queued" | "running" | "cancelling") {
            p.background
                .store(true, std::sync::atomic::Ordering::Release);
        }
        let tx = db.transaction().map_err(db_error)?;
        tx.execute("INSERT INTO evaluation_analysis_refs VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET state=excluded.state",params![id,item.state]).map_err(db_error)?;
        crate::event(&tx, "evaluation.analysis_changed", id)?;
        tx.commit().map_err(db_error)
    }
    /// Returns committed progress. The collection remains hidden until publish.
    pub fn begin_evaluation_workset(
        &self,
        pid: &str,
        item: &AestheticAnalysisJob,
    ) -> Result<(String, u64, u64, bool)> {
        let p = self.handle(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.transaction().map_err(db_error)?;
        let json = encode(&(&item.request, &item.input))?;
        let old:Option<(Option<String>,String,String,u64,u64)>=tx.query_row("SELECT collection_id,request_json,state,after_position,count FROM evaluation_workset_builds WHERE job_id=?1",[&item.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,crate::unsigned(r,3)?,crate::unsigned(r,4)?))).optional().map_err(db_error)?;
        if let Some((id, old, state, after, count)) = old {
            if old != json {
                return Err(Error::new("IDEMPOTENCY_CONFLICT", "派生工作集配置不一致"));
            }
            let id = id.ok_or_else(|| {
                Error::new("OBJECT_REMOVED", "此请求的工作集已经删除，请使用新的请求键")
            })?;
            return Ok((id, after, count, state == "ready"));
        }
        let id = studio_domain::new_id();
        tx.execute(
            "INSERT INTO collections VALUES (?1,?2,0)",
            params![id, item.request.name],
        )
        .map_err(db_error)?;
        tx.execute("INSERT INTO evaluation_workset_builds(job_id,collection_id,request_json,state) VALUES (?1,?2,?3,'building')",params![item.id,id,json]).map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        Ok((id, 0, 0, false))
    }
    pub fn append_evaluation_workset(
        &self,
        pid: &str,
        job: &str,
        expected_after: u64,
        after: u64,
        keys: Vec<studio_domain::AssetKey>,
    ) -> Result<()> {
        if keys.len() > 256 || after <= expected_after {
            return Err(Error::invalid("工作集写入批次或游标无效"));
        }
        let p = self.handle(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.transaction().map_err(db_error)?;
        let (id,state,previous):(Option<String>,String,u64)=tx.query_row("SELECT collection_id,state,after_position FROM evaluation_workset_builds WHERE job_id=?1",[job],|r|Ok((r.get(0)?,r.get(1)?,crate::unsigned(r,2)?))).map_err(db_error)?;
        if state != "building" || previous != expected_after {
            return Err(Error::new("REVISION_CONFLICT", "派生工作集游标已变化"));
        }
        let id = id.ok_or_else(|| Error::new("OBJECT_REMOVED", "派生工作集已删除"))?;
        for key in &keys {
            tx.execute(
                "INSERT INTO collection_members VALUES (?1,?2,?3)",
                params![id, key.source_id, key.asset_id],
            )
            .map_err(db_error)?;
        }
        tx.execute(
            "UPDATE evaluation_workset_builds SET after_position=?2,count=count+?3 WHERE job_id=?1",
            params![job, after as i64, keys.len() as i64],
        )
        .map_err(db_error)?;
        tx.commit().map_err(db_error)
    }
    pub fn publish_evaluation_workset(
        &self,
        pid: &str,
        item: &AestheticAnalysisJob,
    ) -> Result<Collection> {
        let p = self.handle(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.transaction().map_err(db_error)?;
        let (id, state, count, after): (Option<String>, String, u64, u64) = tx
            .query_row(
                "SELECT collection_id,state,count,after_position FROM evaluation_workset_builds WHERE job_id=?1",
                [&item.id],
                |r| Ok((r.get(0)?, r.get(1)?, crate::unsigned(r, 2)?, crate::unsigned(r,3)?)),
            )
            .map_err(db_error)?;
        let id = id.ok_or_else(|| Error::new("OBJECT_REMOVED", "派生工作集已删除"))?;
        if state != "ready" {
            if after != item.input.candidates {
                return Err(Error::new("RESULT_NOT_READY", "尚未完成全部快照成员筛选"));
            }
            if count == 0 {
                return Err(Error::invalid("筛选结果为空，未发布工作集"));
            }
            tx.execute(
                "UPDATE collections SET count=?2 WHERE id=?1",
                params![id, count as i64],
            )
            .map_err(db_error)?;
            let scope = ScopeRef {
                project_id: pid.into(),
                target: ScopeTarget::Workset {
                    collection_id: id.clone(),
                },
            };
            let provenance = serde_json::json!({"version":1,"aesthetic_analysis_job":item.id,"selection":item.request.spec,"frozen_input":item.input,"boundary":"include_score_ties","members":"fixed"});
            tx.execute(
                "INSERT INTO collection_scopes VALUES (?1,?2,?3)",
                params![id, encode(&scope)?, encode(&provenance)?],
            )
            .map_err(db_error)?;
            tx.execute(
                "UPDATE evaluation_workset_builds SET state='ready' WHERE job_id=?1",
                [&item.id],
            )
            .map_err(db_error)?;
            crate::management::created(&tx, "workset", &id)?;
            crate::event(&tx, "collection.created", &id)?;
        }
        let name = crate::management::display_name(&tx, "workset", &id, &item.request.name)?;
        tx.commit().map_err(db_error)?;
        Ok(Collection { id, name, count })
    }
}
