use super::*;
use studio_application::aesthetic::{retryable_failure, stage_failure};

pub(super) fn clock_ms() -> i64 {
    now().parse().unwrap_or(i64::MAX)
}
pub(super) fn change_failure_count(
    db: &Connection,
    id: &str,
    state: &str,
    sign: i32,
) -> Result<()> {
    db.execute(
        "UPDATE stages SET invalid=invalid+?2,unknown=unknown+?3 WHERE id=?1",
        params![
            id,
            sign * i32::from(state == "invalid"),
            sign * i32::from(state == "outcome_unknown")
        ],
    )
    .map_err(db_error)?;
    Ok(())
}
pub(super) fn defer_batch(db: &Connection, id: &str, sequence: u64, reason: &str) -> Result<()> {
    let batch = read_batch(db, id, sequence)?;
    if !matches!(
        batch.state.as_str(),
        "failed" | "invalid" | "outcome_unknown" | "retry_wait"
    ) {
        return Err(Error::invalid("只能暂缓已停靠的异常批次"));
    }
    change_failure_count(db, id, &batch.state, -1)?;
    db.execute(
        "UPDATE batches SET state='deferred',retry_at=NULL,disposition_reason=?2 WHERE sequence=?1",
        params![sequence as i64, reason],
    )
    .map_err(db_error)?;
    for m in &batch.members {
        db.execute("UPDATE candidates SET reserved=0,blocked=CASE WHEN disposition IN ('active','rejudge') THEN 0 ELSE blocked END,blocked_batch=NULL WHERE stage_id=?1 AND ordinal=?2 AND (blocked_batch=?3 OR blocked_batch IS NULL)",params![id,m.candidate.ordinal as i64,sequence as i64]).map_err(db_error)?;
    }
    Ok(())
}
pub(super) fn retry_batch(db: &Connection, stage: &AestheticStage, sequence: u64) -> Result<()> {
    if stage.attempts >= stage.call_limit() {
        return Err(Error::invalid("调用预算已用尽，请先配置追加评审"));
    }
    let batch = read_batch(db, &stage.id, sequence)?;
    if !matches!(
        batch.state.as_str(),
        "failed" | "invalid" | "outcome_unknown" | "retry_wait"
    ) {
        return Err(Error::invalid("此批次无需付费重试"));
    }
    if batch.attempt_count >= 8 {
        return Err(Error::invalid(
            "一个逻辑批次最多保留 8 次调用尝试，请复制为新阶段",
        ));
    }
    change_failure_count(db, &stage.id, &batch.state, -1)?;
    // Explicit retries start a new bounded recovery window, retaining every old attempt.
    db.execute("UPDATE batches SET state='queued',error=NULL,retry_at=NULL,recovery_deadline=NULL,recovery_attempt_base=?2 WHERE sequence=?1",params![sequence as i64,batch.attempt_count]).map_err(db_error)?;
    for m in batch.members {
        db.execute(
            "UPDATE candidates SET reserved=1,blocked_batch=?3 WHERE stage_id=?1 AND ordinal=?2",
            params![stage.id, m.candidate.ordinal as i64, sequence as i64],
        )
        .map_err(db_error)?;
    }
    Ok(())
}
pub(super) fn expire_retries(db: &Connection, stage: &AestheticStage) -> Result<()> {
    let ids=db.prepare("SELECT sequence FROM batches WHERE stage_id=?1 AND state IN ('queued','retry_wait') AND recovery_deadline<=?2 ORDER BY sequence LIMIT 64").map_err(db_error)?
        .query_map(params![stage.id,clock_ms()],|r|crate::unsigned(r,0)).map_err(db_error)?
        .collect::<std::result::Result<Vec<_>,_>>().map_err(db_error)?;
    for sequence in ids {
        let batch = read_batch(db, &stage.id, sequence)?;
        if stage.execution_settings.as_ref().is_some_and(|s| {
            s.policy.exhausted == "defer"
                && batch
                    .last_failure
                    .as_ref()
                    .is_some_and(|f| retryable_failure(f, &s.policy))
        }) {
            // A queued retry has no active HTTP request, but must pass through an exception state.
            if batch.state == "queued" {
                db.execute(
                    "UPDATE batches SET state='failed' WHERE sequence=?1",
                    [sequence as i64],
                )
                .map_err(db_error)?;
            }
            defer_batch(db, &stage.id, sequence, "recovery_time_budget")?;
        } else {
            let state = match batch.last_failure.as_ref() {
                Some(f) if f.code == "EVALUATION_INVALID_OUTPUT" => "invalid",
                Some(f) if f.outcome_unknown => "outcome_unknown",
                _ => "failed",
            };
            change_failure_count(db, &stage.id, state, 1)?;
            db.execute("UPDATE batches SET state=?2,retry_at=NULL,error='批次恢复时限已到；可明确重试以开启新的恢复窗口' WHERE sequence=?1",params![sequence as i64,state]).map_err(db_error)?;
            for m in batch.members {
                db.execute("UPDATE candidates SET reserved=0,blocked=1,blocked_batch=?3 WHERE stage_id=?1 AND ordinal=?2",params![stage.id,m.candidate.ordinal as i64,sequence as i64]).map_err(db_error)?;
            }
        }
    }
    Ok(())
}

/// May share the receipt parser's transaction, so a crash cannot strand a
/// semantic failure between recording it and scheduling its bounded retry.
pub(super) fn schedule_recovery(
    db: &Connection,
    id: &str,
    sequence: u64,
    failure: &LlmFailure,
) -> Result<String> {
    if stage_failure(failure) {
        return Ok("halt".into());
    }
    let stage = read_stage(db, id)?;
    let streak: u32 = db
        .query_row("SELECT failure_streak FROM stages WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .map_err(db_error)?;
    let threshold = stage.execution_settings.as_ref().map_or_else(
        || {
            studio_domain::aesthetic::default_failure_halt_threshold(
                stage.config.request.concurrency,
            )
        },
        |s| s.policy.failure_halt_threshold(),
    );
    if streak >= threshold {
        return Ok("halt".into());
    }
    let Some(settings) = &stage.execution_settings else {
        return Ok("unresolved".into());
    };
    let p = &settings.policy;
    if !retryable_failure(failure, p) {
        return Ok("unresolved".into());
    }
    let batch = read_batch(db, id, sequence)?;
    if !matches!(
        batch.state.as_str(),
        "failed" | "invalid" | "outcome_unknown"
    ) {
        return Ok("unresolved".into());
    }
    let base: u32 = db
        .query_row(
            "SELECT recovery_attempt_base FROM batches WHERE sequence=?1",
            [sequence as i64],
            |r| r.get(0),
        )
        .map_err(db_error)?;
    let attempts = batch.attempt_count.saturating_sub(base);
    let retry_after: Option<String> = db
        .query_row("SELECT json_extract(metadata,'$.headers.retry-after') FROM raw_receipts WHERE attempt_id=?1", [&batch.attempt_id], |r| r.get(0))
        .optional().map_err(db_error)?.flatten();
    let jitter = (sequence.wrapping_mul(31) + u64::from(attempts) * 137) % 751;
    let delay = retry_after
        .and_then(|v| v.parse::<u64>().ok())
        .map(|v| v.saturating_mul(1000))
        .unwrap_or(if attempts <= 1 { 3000 } else { 10000 })
        .max(1000)
        .saturating_add(jitter);
    let due = clock_ms().saturating_add(delay.min(i64::MAX as u64) as i64);
    let deadline = batch
        .recovery_deadline
        .as_ref()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    if p.max_retries > 0
        && attempts <= p.max_retries
        && batch.attempt_count < 8
        && stage.attempts < stage.call_limit()
        && due < deadline
    {
        change_failure_count(db, id, &batch.state, -1)?;
        db.execute(
            "UPDATE batches SET state='retry_wait',retry_at=?2 WHERE sequence=?1",
            params![sequence as i64, due],
        )
        .map_err(db_error)?;
        let semantic = matches!(
            failure.code.as_str(),
            "EVALUATION_NO_COMPARABLE_EVIDENCE" | "EVALUATION_INVALID_OUTPUT"
        );
        for m in batch.members {
            db.execute("UPDATE candidates SET reserved=1,blocked=CASE WHEN ?3 THEN 0 ELSE blocked END WHERE stage_id=?1 AND ordinal=?2",
                params![id, m.candidate.ordinal as i64, semantic]).map_err(db_error)?;
        }
        Ok("retry_wait".into())
    } else if p.exhausted == "defer" {
        defer_batch(db, id, sequence, "automatic_recovery_exhausted")?;
        Ok("deferred".into())
    } else {
        Ok("unresolved".into())
    }
}

impl EvaluationDb {
    pub fn attempt_exists(&self, id: &str, attempt: &str) -> Result<bool> {
        self.read()?.query_row("SELECT EXISTS(SELECT 1 FROM attempts a JOIN batches b ON b.sequence=a.batch WHERE a.id=?1 AND b.stage_id=?2)",params![attempt,id],|r|r.get(0)).map_err(db_error)
    }
    pub fn batch_deadline(&self, id: &str, sequence: u64) -> Result<Option<u64>> {
        self.read()?
            .query_row(
                "SELECT recovery_deadline FROM batches WHERE stage_id=?1 AND sequence=?2",
                params![id, sequence as i64],
                |r| r.get::<_, Option<i64>>(0),
            )
            .map(|v| v.map(|v| v as u64))
            .map_err(db_error)
    }
    pub fn next_retry_at(&self, id: &str) -> Result<Option<u64>> {
        self.read()?
            .query_row(
                "SELECT MIN(retry_at) FROM batches WHERE stage_id=?1 AND state='retry_wait'",
                [id],
                |r| r.get::<_, Option<i64>>(0),
            )
            .map(|v| v.map(|v| v as u64))
            .map_err(db_error)
    }
    /// Called after the failure is durable. It never sends a request or accepts evidence.
    pub fn schedule_recovery(
        &self,
        id: &str,
        sequence: u64,
        failure: LlmFailure,
    ) -> Result<String> {
        let id = id.to_owned();
        self.writer.submit(4096, move |db| {
            schedule_recovery(db, &id, sequence, &failure)
        })
    }
    pub fn batch_action(
        &self,
        id: &str,
        value: AestheticBatchAction,
    ) -> Result<AestheticBatchActionResult> {
        studio_domain::validate_id(&value.idempotency_key)?;
        if !matches!(value.action.as_str(), "retry" | "defer")
            || value.batches.len() > 100
            || value
                .batches
                .iter()
                .any(|v| *v == 0 || *v > i64::MAX as u64)
            || value.reason.len() > 1000
            || (value.action == "defer" && value.reason.trim().is_empty())
            || !value.acknowledge_possible_charge
        {
            return Err(Error::invalid(
                "请选择有效批次操作并确认可能已发生的费用；暂缓需要填写原因",
            ));
        }
        let id = id.to_owned();
        let json = encode(&value)?;
        self.writer.submit(64*1024,move|db| {
            let previous:Option<(String,String)>=db.query_row("SELECT stage_id,request_json FROM batch_actions WHERE id=?1",[&value.idempotency_key],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(db_error)?;
            if let Some((stage,old))=previous {
                if stage!=id||old!=json {return Err(Error::new("IDEMPOTENCY_CONFLICT","批量操作键已用于其他请求"));}
            }else{
                let upper:u64=db.query_row("SELECT COALESCE(MAX(sequence),0) FROM batches WHERE stage_id=?1",[&id],|r|crate::unsigned(r,0)).map_err(db_error)?;
                db.execute("INSERT INTO batch_actions(id,stage_id,request_json,upper_sequence) VALUES(?1,?2,?3,?4)",params![value.idempotency_key,id,json,upper as i64]).map_err(db_error)?;
            }
            let mut result=read_action(db,&value.idempotency_key)?;
            if result.completed {return Ok(result);}
            let stage=read_stage(db,&id)?;
            if !matches!(stage.state.as_str(),"ready"|"paused"|"needs_attention"|"failed") {return Err(Error::new("REVISION_CONFLICT","请先暂停阶段再处理异常批次"));}
            let (after,upper):(u64,u64)=db.query_row("SELECT after_sequence,upper_sequence FROM batch_actions WHERE id=?1",[&value.idempotency_key],|r|Ok((crate::unsigned(r,0)?,crate::unsigned(r,1)?))).map_err(db_error)?;
            let mut ids=if value.batches.is_empty() {
                db.prepare("SELECT sequence FROM batches WHERE stage_id=?1 AND sequence>?2 AND sequence<=?3 AND state IN ('failed','invalid','outcome_unknown','retry_wait') ORDER BY sequence LIMIT 33").map_err(db_error)?
                    .query_map(params![id,after as i64,upper as i64],|r|crate::unsigned(r,0)).map_err(db_error)?.collect::<std::result::Result<Vec<_>,_>>().map_err(db_error)?
            }else{let mut ids=value.batches.iter().copied().filter(|v|*v>after).collect::<Vec<_>>();ids.sort_unstable();ids.dedup();ids.truncate(33);ids};
            let more=ids.len()>32;ids.truncate(32);result.items.clear();
            let next=ids.last().copied().unwrap_or(after);
            for sequence in ids {
                let local=db.query_row("SELECT stage_sequence FROM batches WHERE stage_id=?1 AND sequence=?2",params![id,sequence as i64],|r|crate::unsigned(r,0)).optional().map_err(db_error)?.unwrap_or(0);
                // Each item uses a savepoint: a rejected item cannot leave partial counter changes.
                db.execute_batch("SAVEPOINT batch_action_item").map_err(db_error)?;
                let outcome=if value.action=="retry" {retry_batch(db,&stage,sequence)}else{defer_batch(db,&id,sequence,&value.reason)};
                let (state,error)=match outcome {
                    Ok(())=>{db.execute_batch("RELEASE batch_action_item").map_err(db_error)?;result.succeeded+=1;(if value.action=="retry" {"queued"}else{"deferred"}.to_owned(),None)},
                    Err(error)=>{db.execute_batch("ROLLBACK TO batch_action_item; RELEASE batch_action_item").map_err(db_error)?;result.failed+=1;("rejected".into(),Some(error.to_string()))},
                };
                result.processed+=1;result.items.push(AestheticBatchActionItem{sequence,stage_sequence:local,state,error});
            }
            result.completed = !more;
            db.execute("UPDATE batch_actions SET after_sequence=?2,processed=?3,succeeded=?4,failed=?5,completed=?6,last_items=?7 WHERE id=?1",params![result.id,next as i64,result.processed as i64,result.succeeded as i64,result.failed as i64,result.completed,encode(&result.items)?]).map_err(db_error)?;
            Ok(result)
        })
    }
}
fn read_action(db: &Connection, id: &str) -> Result<AestheticBatchActionResult> {
    let (completed, processed, succeeded, failed, items): (bool, u64, u64, u64, String) = db
        .query_row(
            "SELECT completed,processed,succeeded,failed,last_items FROM batch_actions WHERE id=?1",
            [id],
            |r| {
                Ok((
                    r.get(0)?,
                    crate::unsigned(r, 1)?,
                    crate::unsigned(r, 2)?,
                    crate::unsigned(r, 3)?,
                    r.get(4)?,
                ))
            },
        )
        .map_err(db_error)?;
    Ok(AestheticBatchActionResult {
        id: id.into(),
        completed,
        processed,
        succeeded,
        failed,
        items: decode(items)?,
    })
}
