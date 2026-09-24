use super::*;

impl EvaluationDb {
    pub fn decide_candidate(
        &self,
        id: &str,
        ordinal: u64,
        decision: AestheticCandidateDecision,
    ) -> Result<AestheticCandidate> {
        studio_domain::validate_id(&decision.idempotency_key)?;
        if ordinal > i64::MAX as u64
            || decision.reason.trim().is_empty()
            || decision.reason.len() > 1000
        {
            return Err(Error::invalid("候选处置需要有效序号和 1–1000 字节的原因"));
        }
        let id = id.to_owned();
        let json = encode(&decision)?;
        self.writer.submit(json.len(),move |db| {
            let old: Option<(u64,String)> = db.query_row("SELECT ordinal,request_json FROM candidate_decisions WHERE stage_id=?1 AND id=?2",params![id,decision.idempotency_key],|r|Ok((crate::unsigned(r,0)?,r.get(1)?))).optional().map_err(db_error)?;
            let get = || candidates(db,"WHERE stage_id=?1 AND ordinal=?2",params![id,ordinal as i64])?.pop().ok_or_else(||Error::new("NOT_FOUND","候选不存在"));
            if let Some((old_ordinal,old_json)) = old {
                if old_ordinal!=ordinal || old_json!=json { return Err(Error::new("IDEMPOTENCY_CONFLICT","处置键已用于不同决定")); }
                return get();
            }
            let stage = read_stage(db,&id)?;
            if !matches!(stage.state.as_str(),"ready"|"paused"|"needs_attention"|"failed") {
                return Err(Error::new("REVISION_CONFLICT","请等待阶段暂停后处置候选"));
            }
            let candidate = get()?;
            if candidate.disposition != AestheticDisposition::NeedsReview {
                return Err(Error::new("REVISION_CONFLICT","只允许处置待复核候选；既有决定使用同一键重试"));
            }
            let reserved: bool = db.query_row("SELECT reserved FROM candidates WHERE stage_id=?1 AND ordinal=?2",params![id,ordinal as i64],|r|r.get(0)).map_err(db_error)?;
            if reserved { return Err(Error::new("REVISION_CONFLICT","候选仍属于未完成批次")); }
            let next = match decision.action {
                AestheticDispositionAction::Exclude => "excluded",
                AestheticDispositionAction::Rejudge => {
                    if !matches!(candidate.rating.as_str(),"g"|"s"|"q"|"e") { return Err(Error::new("EVALUATION_RATING_UNRESOLVED","Rating 不确定，不能安排跨 Rating 比较")); }
                    if stage.attempts >= stage.call_limit() { return Err(Error::invalid("调用预算已用尽")); }
                    "rejudge"
                }
            };
            db.execute("INSERT INTO candidate_decisions VALUES (?1,?2,?3,?4,?5,?6,?7)",params![id,decision.idempotency_key,ordinal as i64,json,candidate.disposition.as_str(),next,now()]).map_err(db_error)?;
            db.execute("UPDATE candidates SET disposition=?3,disposition_reason=?4,blocked=?5 WHERE stage_id=?1 AND ordinal=?2",params![id,ordinal as i64,next,decision.reason,next=="excluded"]).map_err(db_error)?;
            if let Some(mut status)=sampling::status(db,&id)? {
                status.state="ready".into();status.reason=None;
                db.execute("UPDATE sampling_plans SET status_json=?2 WHERE id=?1",params![status.plan_id,encode(&status)?]).map_err(db_error)?;
            }
            let stage = read_stage(db,&id)?;
            if stage.sampling.is_none() && stage.unresolved==0 && stage.frozen==stage.total && stage.excluded>0 {
                let pending: bool=db.query_row("SELECT EXISTS(SELECT 1 FROM batches WHERE stage_id=?1 AND state!='accepted')",[&id],|r|r.get(0)).map_err(db_error)?;
                if !pending { db.execute("UPDATE stages SET state='completed_with_exclusions',error=NULL WHERE id=?1",[&id]).map_err(db_error)?; }
            }
            get()
        })
    }
}
