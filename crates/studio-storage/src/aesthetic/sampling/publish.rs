//! Bounded staging writes; only the final checkpoint pointer makes slots visible.
use super::*;
const PAGE: usize = 256;

#[derive(Clone)]
struct Fence {
    stage: String,
    plan: String,
    previous_round: u32,
    round: u32,
    attempts: u64,
    accepted: u64,
    watermark: u64,
    staging: String,
}
impl Fence {
    fn current(&self, db: &Connection) -> Result<()> {
        let stage = read_stage(db, &self.stage)?;
        if stage.state != "running"
            || stage.attempts != self.attempts
            || stage.accepted != self.accepted
            || stage
                .sampling
                .as_ref()
                .is_none_or(|s| s.plan_id != self.plan || s.round != self.previous_round)
        {
            return Err(Error::new("CANCELLED", "采样计划发布前阶段已变化"));
        }
        Ok(())
    }
    fn owned(&self, db: &Connection) -> Result<()> {
        self.current(db)?;
        let valid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sampling_rounds WHERE plan_id=?1 AND round=?2 AND summary_json=?3)",params![self.plan,self.round,self.staging],|r|r.get(0)).map_err(db_error)?;
        if !valid {
            return Err(Error::new("CANCELLED", "采样暂存已由其他规划替代"));
        }
        Ok(())
    }
}

fn clear_rows(
    store: &EvaluationDb,
    fence: &Fence,
    plan: &str,
    round: u32,
    check: &dyn Fn() -> Result<()>,
) -> Result<()> {
    for sql in [
        "DELETE FROM sampling_queue WHERE rowid IN (SELECT rowid FROM sampling_queue WHERE plan_id=?1 AND round=?2 LIMIT ?3)",
        "DELETE FROM sampling_diagnostics WHERE rowid IN (SELECT rowid FROM sampling_diagnostics WHERE plan_id=?1 AND round=?2 LIMIT ?3)",
    ] {
        loop {
            check()?;
            let owned = fence.clone();
            let plan = plan.to_owned();
            let deleted = store.writer.submit(4096, move |db| {
                owned.owned(db)?;
                db.execute(sql, params![plan, round, PAGE as u32])
                    .map_err(db_error)
            })?;
            if deleted < PAGE {
                break;
            }
        }
    }
    Ok(())
}

fn clear_replaced_staging(
    store: &EvaluationDb,
    fence: &Fence,
    check: &dyn Fn() -> Result<()>,
) -> Result<()> {
    loop {
        check()?;
        let abandoned:Option<(String,u32)>=store.read()?.query_row("SELECT r.plan_id,r.round FROM sampling_rounds r JOIN sampling_plans p ON p.id=r.plan_id WHERE p.stage_id=?1 AND r.plan_id<>?2 AND r.round>json_extract(p.status_json,'$.round') AND json_extract(r.summary_json,'$.state')='staging' ORDER BY r.plan_id,r.round LIMIT 1",params![fence.stage,fence.plan],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(db_error)?;
        let Some((plan, round)) = abandoned else {
            break;
        };
        let claimed:bool=store.read()?.query_row("SELECT EXISTS(SELECT 1 FROM sampling_queue WHERE plan_id=?1 AND round=?2 AND batch IS NOT NULL)",params![plan,round],|r|r.get(0)).map_err(db_error)?;
        if claimed {
            return Err(Error::new(
                "EVIDENCE_INVALID",
                "未发布轮次含已派发批次，不能清理",
            ));
        }
        clear_rows(store, fence, &plan, round, check)?;
        let owned = fence.clone();
        store.writer.submit(4096,move|db|{owned.owned(db)?;db.execute("DELETE FROM sampling_rounds WHERE plan_id=?1 AND round=?2 AND json_extract(summary_json,'$.state')='staging'",params![plan,round]).map_err(db_error)?;Ok(())})?;
    }
    Ok(())
}

pub(in crate::aesthetic) fn round(
    store: &EvaluationDb,
    stage: &AestheticStage,
    status: &AestheticSamplingStatus,
    input: &AestheticAnalysisInput,
    round: scheduler::Round,
    check: &dyn Fn() -> Result<()>,
) -> Result<bool> {
    let expected_diagnostics = round.diagnostics.len() as u64;
    let expected_batches = round.batches.len() as u64;
    if expected_diagnostics != stage.total {
        return Err(Error::new("EVIDENCE_INVALID", "采样诊断数量不完整"));
    }
    let mut next = round.status;
    next.round = status
        .round
        .checked_add(1)
        .ok_or_else(|| Error::invalid("采样轮次序号超限"))?;
    let mut building = next.clone();
    building.state = "staging".into();
    building.reason = Some(studio_domain::new_id());
    let fence = Fence {
        stage: stage.id.clone(),
        plan: status.plan_id.clone(),
        previous_round: status.round,
        round: next.round,
        attempts: stage.attempts,
        accepted: stage.accepted,
        watermark: input.evidence_watermark,
        staging: encode(&building)?,
    };
    let owned = fence.clone();
    store.writer.submit(4096,move|db|{
        owned.current(db)?;
        let existing:Option<String>=db.query_row("SELECT summary_json FROM sampling_rounds WHERE plan_id=?1 AND round=?2",params![owned.plan,owned.round],|r|r.get(0)).optional().map_err(db_error)?;
        if let Some(json)=existing{
            let summary:AestheticSamplingStatus=decode(json)?;
            let claimed:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sampling_queue WHERE plan_id=?1 AND round=?2 AND batch IS NOT NULL)",params![owned.plan,owned.round],|r|r.get(0)).map_err(db_error)?;
            if summary.state!="staging"||claimed{return Err(Error::new("EVIDENCE_INVALID","不能覆盖已发布的采样轮次"));}
            db.execute("UPDATE sampling_rounds SET evidence_watermark=?3,summary_json=?4 WHERE plan_id=?1 AND round=?2",params![owned.plan,owned.round,owned.watermark as i64,owned.staging]).map_err(db_error)?;
        }else{
            db.execute("INSERT INTO sampling_rounds VALUES(?1,?2,?3,?4)",params![owned.plan,owned.round,owned.watermark as i64,owned.staging]).map_err(db_error)?;
        }
        Ok(())
    })?;
    // A cancelled/crashed build never became claimable. Remove only its unpublished
    // rows, with short transactions so receipt writes retain their priority.
    clear_rows(store, &fence, &fence.plan, fence.round, check)?;
    clear_replaced_staging(store, &fence, check)?;
    let mut no_peers = Vec::new();
    let mut diagnostics = round.diagnostics.into_iter();
    loop {
        check()?;
        let mut page = Vec::with_capacity(PAGE);
        for row in diagnostics.by_ref().take(PAGE) {
            if row.reason == "no_comparison_peer" {
                no_peers.push(row.ordinal);
            }
            page.push((row.ordinal, encode(&row)?));
        }
        if page.is_empty() {
            break;
        }
        let bytes = page.iter().map(|(_, json)| json.len() + 64).sum::<usize>() + 4096;
        let owned = fence.clone();
        store.writer.submit(bytes, move |db| {
            owned.owned(db)?;
            let mut stmt = db
                .prepare("INSERT INTO sampling_diagnostics VALUES(?1,?2,?3,?4)")
                .map_err(db_error)?;
            for (ordinal, json) in page {
                stmt.execute(params![owned.plan, owned.round, ordinal as i64, json])
                    .map_err(db_error)?;
            }
            Ok(())
        })?;
    }
    let mut batches = round.batches.into_iter().enumerate();
    loop {
        check()?;
        let page = batches
            .by_ref()
            .take(PAGE)
            .map(|(slot, group)| Ok((slot, encode(&group)?)))
            .collect::<Result<Vec<_>>>()?;
        if page.is_empty() {
            break;
        }
        let bytes = page.iter().map(|(_, json)| json.len() + 64).sum::<usize>() + 4096;
        let owned = fence.clone();
        store.writer.submit(bytes, move |db| {
            owned.owned(db)?;
            let mut stmt = db
                .prepare(
                    "INSERT INTO sampling_queue(plan_id,round,slot,members) VALUES(?1,?2,?3,?4)",
                )
                .map_err(db_error)?;
            for (slot, json) in page {
                stmt.execute(params![owned.plan, owned.round, slot as u32, json])
                    .map_err(db_error)?;
            }
            Ok(())
        })?;
    }
    check()?;
    store.writer.submit(4096+no_peers.len()*8,move|db|{
        fence.owned(db)?;
        let watermark:u64=db.query_row("SELECT COALESCE(MAX(e.sequence),0) FROM evidence e JOIN batches b ON b.sequence=e.batch WHERE b.stage_id=?1",[&fence.stage],|r|crate::unsigned(r,0)).map_err(db_error)?;
        if watermark!=fence.watermark{return Err(Error::new("REVISION_CONFLICT","采样证据水位已变化"));}
        let counts:(u64,u64)=db.query_row("SELECT (SELECT COUNT(*) FROM sampling_diagnostics WHERE plan_id=?1 AND round=?2),(SELECT COUNT(*) FROM sampling_queue WHERE plan_id=?1 AND round=?2)",params![fence.plan,fence.round],|r|Ok((crate::unsigned(r,0)?,crate::unsigned(r,1)?))).map_err(db_error)?;
        if counts!=(expected_diagnostics,expected_batches){return Err(Error::new("EVIDENCE_INVALID","暂存轮次不完整，尚未发布"));}
        for ordinal in no_peers{
            db.execute("UPDATE candidates SET disposition='needs_review',disposition_reason='no_comparison_peer',blocked=1 WHERE stage_id=?1 AND ordinal=?2 AND reserved=0 AND disposition IN ('active','rejudge')",params![fence.stage,ordinal as i64]).map_err(db_error)?;
        }
        let summary=encode(&next)?;
        db.execute("UPDATE sampling_rounds SET summary_json=?3 WHERE plan_id=?1 AND round=?2",params![fence.plan,fence.round,summary]).map_err(db_error)?;
        db.execute("UPDATE sampling_plans SET status_json=?2 WHERE id=?1",params![fence.plan,summary]).map_err(db_error)?;
        Ok(expected_batches>0)
    })
}
