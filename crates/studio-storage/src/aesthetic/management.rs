use super::*;
use std::collections::BTreeMap;

pub(super) fn progress(db: &Connection, stage: &AestheticStage) -> Result<AestheticStageProgress> {
    let mut p = AestheticStageProgress::default();
    let counts = db
        .prepare("SELECT state,count FROM batch_state_counts WHERE stage_id=?1")
        .map_err(db_error)?
        .query_map([&stage.id], |r| {
            Ok((r.get::<_, String>(0)?, crate::unsigned(r, 1)?))
        })
        .map_err(db_error)?
        .collect::<std::result::Result<BTreeMap<_, _>, _>>()
        .map_err(db_error)?;
    let count = |key: &str| counts.get(key).copied().unwrap_or(0);
    p.preparing = count("preparing");
    p.in_flight = count("sent") + count("received");
    p.queued = count("queued");
    p.retry_waiting = count("retry_wait");
    p.failed = count("failed") + count("invalid") + count("outcome_unknown");
    p.deferred = count("deferred");
    let min = stage
        .sampling
        .as_ref()
        .map_or(stage.config.request.exposures, |s| s.policy.min_exposures);
    (p.blocked,p.covered,p.exposed_once)=db.query_row(
        "SELECT COALESCE(SUM(CASE WHEN blocked=1 AND disposition!='excluded' THEN count ELSE 0 END),0),COALESCE(SUM(CASE WHEN exposures>=?2 AND disposition!='excluded' THEN count ELSE 0 END),0),COALESCE(SUM(CASE WHEN exposures>0 AND disposition!='excluded' THEN count ELSE 0 END),0) FROM exposure_counts WHERE stage_id=?1",
        params![stage.id,min],|r|Ok((crate::unsigned(r,0)?,crate::unsigned(r,1)?,crate::unsigned(r,2)?))).map_err(db_error)?;
    if let Some(s) = &stage.sampling {
        if let Some((planned, unclaimed)) = db
            .query_row(
                "SELECT planned,unclaimed FROM sampling_rounds WHERE plan_id=?1 AND round=?2",
                params![s.plan_id, s.round],
                |r| Ok((crate::unsigned(r, 0)?, crate::unsigned(r, 1)?)),
            )
            .optional()
            .map_err(db_error)?
        {
            p.round_planned = planned;
            p.round_unclaimed = unclaimed;
        }
        p.round_accepted=db.query_row("SELECT count FROM batch_round_counts WHERE plan_id=?1 AND round=?2 AND state='accepted'",params![s.plan_id,s.round],|r|crate::unsigned(r,0)).optional().map_err(db_error)?.unwrap_or(0);
    }
    Ok(p)
}

impl EvaluationDb {
    pub fn execution_update_applied(
        &self,
        id: &str,
        value: &AestheticExecutionUpdate,
    ) -> Result<bool> {
        let old: Option<String> = self
            .read()?
            .query_row(
                "SELECT request_json FROM execution_updates WHERE stage_id=?1 AND id=?2",
                params![id, value.idempotency_key],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?;
        if let Some(old) = old {
            if old != encode(value)? {
                return Err(Error::new(
                    "IDEMPOTENCY_CONFLICT",
                    "执行设置保存键已用于其他内容",
                ));
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
    /// UI listing uses chronological keyset pagination. Internal reconciliation keeps its ID cursor.
    pub fn recent_stages(
        &self,
        after: Option<&str>,
        limit: usize,
        archived: bool,
        search: &str,
        state: Option<&str>,
    ) -> Result<Vec<AestheticStage>> {
        if search.len() > 480 {
            return Err(Error::invalid("搜索内容过长"));
        }
        let db = self.read()?;
        let before = after
            .map(|id| {
                db.query_row("SELECT created_at,id FROM stages WHERE id=?1", [id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .optional()
                .map_err(db_error)
            })
            .transpose()?
            .flatten();
        if after.is_some() && before.is_none() {
            return Err(Error::invalid("阶段分页位置不存在，请返回首页"));
        }
        let ids=db.prepare("SELECT id FROM stages WHERE archived=?1 AND (?2='' OR instr(lower(name),lower(?2))>0) AND (?3 IS NULL OR state=?3) AND (?4 IS NULL OR (created_at,id)<(?4,?5)) ORDER BY created_at DESC,id DESC LIMIT ?6").map_err(db_error)?
            .query_map(params![archived,search,state,before.as_ref().map(|v|&v.0),before.as_ref().map(|v|&v.1),limit.clamp(1,100) as u32],|r|r.get::<_,String>(0)).map_err(db_error)?
            .collect::<std::result::Result<Vec<_>,_>>().map_err(db_error)?;
        ids.iter().map(|id| read_stage(&db, id)).collect()
    }
    pub fn stage_metadata(
        &self,
        id: &str,
        value: AestheticStageMetadata,
    ) -> Result<AestheticStage> {
        studio_domain::validate_name(&value.name)?;
        let id = id.to_owned();
        self.writer.submit(1024, move |db| {
            let stage = read_stage(db, &id)?;
            if value.archived
                && matches!(
                    stage.state.as_str(),
                    "preparing" | "running" | "pausing" | "cancelling"
                )
            {
                return Err(Error::new(
                    "REVISION_CONFLICT",
                    "请先暂停阶段并等待在途请求结束，再归档",
                ));
            }
            db.execute(
                "UPDATE stages SET name=?2,archived=?3 WHERE id=?1",
                params![id, value.name.trim(), value.archived],
            )
            .map_err(db_error)?;
            read_stage(db, &id)
        })
    }
    pub fn configure_execution(
        &self,
        id: &str,
        value: AestheticExecutionUpdate,
        provider_revision: u64,
        model_revision: u64,
    ) -> Result<AestheticStage> {
        studio_domain::validate_id(&value.idempotency_key)?;
        studio_application::aesthetic::validate_execution_policy(&value.policy)?;
        let request = encode(&value)?;
        let id = id.to_owned();
        self.writer.submit(request.len()+4096,move|db| {
            let previous:Option<String>=db.query_row("SELECT request_json FROM execution_updates WHERE stage_id=?1 AND id=?2",params![id,value.idempotency_key],|r|r.get(0)).optional().map_err(db_error)?;
            if let Some(previous)=previous {
                if previous!=request {return Err(Error::new("IDEMPOTENCY_CONFLICT","执行设置保存键已用于其他内容"));}
                return read_stage(db,&id);
            }
            let stage=read_stage(db,&id)?;
            if !matches!(stage.state.as_str(),"ready"|"paused"|"needs_attention"|"failed"|"completed"|"completed_with_exclusions") {
                return Err(Error::new("REVISION_CONFLICT","请先暂停阶段，等待在途请求结束后调整执行设置"));
            }
            if stage.execution_settings.as_ref().map_or(0,|s|s.revision)!=value.expected_revision {return Err(Error::new("REVISION_CONFLICT","执行设置已变化，请重新载入"));}
            let active:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM batches WHERE stage_id=?1 AND state IN ('preparing','sent','received'))",[&id],|r|r.get(0)).map_err(db_error)?;
            if active {return Err(Error::new("REVISION_CONFLICT","阶段仍有在途或待解析批次"));}
            let settings=AestheticExecutionSettings{revision:value.expected_revision+1,updated_at:now(),provider_revision,model_revision,policy:value.policy};
            let settings=encode(&settings)?;
            db.execute("INSERT INTO execution_updates VALUES(?1,?2,?3,?4,?5)",params![id,value.idempotency_key,request,settings,now()]).map_err(db_error)?;
            db.execute("UPDATE stages SET execution_settings=?2 WHERE id=?1",params![id,settings]).map_err(db_error)?;
            read_stage(db,&id)
        })
    }
    pub fn filtered_batches(
        &self,
        id: &str,
        after: u64,
        limit: usize,
        state: Option<&str>,
        sequence: Option<u64>,
    ) -> Result<Vec<AestheticBatch>> {
        if state.is_some_and(|s| {
            !matches!(
                s,
                "issues"
                    | "accepted"
                    | "queued"
                    | "preparing"
                    | "sent"
                    | "received"
                    | "failed"
                    | "invalid"
                    | "outcome_unknown"
                    | "retry_wait"
                    | "deferred"
                    | "superseded"
            )
        }) {
            return Err(Error::invalid("批次状态无效"));
        }
        let db = self.read()?;
        let ids=db.prepare("SELECT sequence FROM batches WHERE stage_id=?1 AND sequence>?2 AND (?3 IS NULL OR sequence=?3) AND (?4 IS NULL OR state=?4 OR (?4='issues' AND state IN ('failed','invalid','outcome_unknown','retry_wait'))) ORDER BY sequence LIMIT ?5").map_err(db_error)?
            .query_map(params![id,after as i64,sequence.map(|s|s as i64),state,limit.clamp(1,100) as u32],|r|crate::unsigned(r,0)).map_err(db_error)?
            .collect::<std::result::Result<Vec<_>,_>>().map_err(db_error)?;
        ids.iter().map(|s| read_batch(&db, id, *s)).collect()
    }
    pub fn blocked_candidates(
        &self,
        id: &str,
        after: Option<u64>,
    ) -> Result<Vec<AestheticCandidate>> {
        candidates(
            &*self.read()?,
            "WHERE stage_id=?1 AND blocked=1 AND ordinal>?2 AND disposition!='excluded' ORDER BY ordinal LIMIT 64",
            params![id, after.map_or(-1, |v| v as i64)],
        )
    }
}
