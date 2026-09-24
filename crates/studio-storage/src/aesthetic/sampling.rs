//! Immutable policy revisions and atomic round publication. No network access.
use super::*;
use std::collections::BTreeSet;
use studio_application::aesthetic::sampling as scheduler;
use studio_domain::aesthetic_analysis::AestheticAnalysisInput;

pub(super) fn status(db: &Connection, stage: &str) -> Result<Option<AestheticSamplingStatus>> {
    let json: Option<String> = db.query_row("SELECT p.status_json FROM stages s JOIN sampling_plans p ON p.id=s.sampling_plan_id WHERE s.id=?1",[stage],|r|r.get(0)).optional().map_err(db_error)?;
    json.map(decode).transpose()
}

fn initial_status(
    id: String,
    policy: AestheticSamplingPolicy,
    limit: u64,
    watermark: u64,
) -> AestheticSamplingStatus {
    AestheticSamplingStatus {
        plan_id: id,
        previous_plan_id: None,
        version: scheduler::VERSION.into(),
        policy,
        call_limit: limit,
        round: 0,
        evidence_watermark: watermark,
        state: "ready".into(),
        reason: None,
        eligible: 0,
        covered: 0,
        stable: 0,
        components: 0,
        unresolved: 0,
    }
}

pub(super) fn initialize(db: &Connection, config: &AestheticConfig, total: u64) -> Result<()> {
    if let Some(policy) = &config.request.sampling {
        scheduler::validate(policy, total)?;
        let id = &config.request.idempotency_key;
        let status = initial_status(
            id.clone(),
            policy.clone(),
            u64::from(config.request.max_calls),
            0,
        );
        db.execute(
            "INSERT INTO sampling_plans VALUES(?1,?1,?2,?3,0,?4)",
            params![id, encode(policy)?, now(), encode(&status)?],
        )
        .map_err(db_error)?;
        db.execute("UPDATE stages SET sampling_plan_id=?1 WHERE id=?1", [id])
            .map_err(db_error)?;
    }
    Ok(())
}

pub(super) fn claim(db: &Connection, stage: &AestheticStage) -> Result<Option<AestheticBatch>> {
    let plan = stage.sampling.as_ref().expect("sampling stage");
    let row: Option<(u32,String)> = db.query_row("SELECT slot,members FROM sampling_queue WHERE plan_id=?1 AND round=?2 AND batch IS NULL ORDER BY slot LIMIT 1",params![plan.plan_id,plan.round],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(db_error)?;
    let Some((slot, json)) = row else {
        return Ok(None);
    };
    let reasons: Vec<AestheticSamplingMemberReason> = decode(json)?;
    if reasons.len() < 2
        || reasons.len() > 16
        || reasons
            .iter()
            .map(|r| r.ordinal)
            .collect::<BTreeSet<_>>()
            .len()
            != reasons.len()
    {
        return Err(Error::new(
            "EVIDENCE_INVALID",
            "采样槽位含重复或非法数量的图片",
        ));
    }
    let mut members = Vec::new();
    for reason in &reasons {
        let candidate = candidates(db,"WHERE stage_id=?1 AND ordinal=?2 AND blocked=0 AND reserved=0 AND disposition IN ('active','rejudge')",params![stage.id,reason.ordinal as i64])?.pop().ok_or_else(||Error::new("REVISION_CONFLICT","冻结轮次的候选已不可派发，请先处置未完成批次"))?;
        if candidate.exposures >= plan.policy.max_exposures {
            return Err(Error::new("EVIDENCE_INVALID", "轮次超过单图曝光上限"));
        }
        members.push(AestheticMember {
            label: format!("img{:02}", members.len() + 1),
            candidate,
            image_sha256: None,
        });
    }
    if members.len() < 2
        || members.len() > 16
        || members
            .iter()
            .any(|m| m.candidate.rating != members[0].candidate.rating)
    {
        return Err(Error::new("EVIDENCE_INVALID", "采样计划成员无效"));
    }
    let audit = AestheticBatchSampling {
        plan_id: plan.plan_id.clone(),
        round: plan.round,
        evidence_watermark: plan.evidence_watermark,
        members: reasons,
    };
    db.execute("INSERT INTO batches(stage_id,rating,state,members,sampling) VALUES(?1,?2,'preparing',?3,?4)",params![stage.id,members[0].candidate.rating,encode(&members)?,encode(&audit)?]).map_err(db_error)?;
    let sequence = db.last_insert_rowid();
    db.execute("UPDATE sampling_queue SET batch=?4 WHERE plan_id=?1 AND round=?2 AND slot=?3 AND batch IS NULL",params![plan.plan_id,plan.round,slot,sequence]).map_err(db_error)?;
    for m in members {
        db.execute(
            "UPDATE candidates SET reserved=1 WHERE stage_id=?1 AND ordinal=?2",
            params![stage.id, m.candidate.ordinal as i64],
        )
        .map_err(db_error)?;
    }
    read_batch(db, &stage.id, sequence as u64).map(Some)
}

impl EvaluationDb {
    /// Append an explicit scheduling/budget revision, retaining the original
    /// transport configuration, accepted receipts and published snapshots.
    /// This only prepares a plan; the existing start command authorizes dispatch.
    pub fn configure_sampling(
        &self,
        id: &str,
        request: AestheticSamplingRequest,
    ) -> Result<AestheticStage> {
        studio_domain::validate_id(&request.idempotency_key)?;
        if !(1..=10_000_000).contains(&request.additional_calls) {
            return Err(Error::invalid("追加调用上限须为 1–10000000"));
        }
        let id = id.to_owned();
        let json = encode(&request)?;
        self.writer.submit(json.len(),move|db|{
            let existing:Option<(String,String)>=db.query_row("SELECT stage_id,request_json FROM sampling_plans WHERE id=?1",[&request.idempotency_key],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(db_error)?;
            if let Some((stage,old))=existing {
                if stage!=id || old!=json {return Err(Error::new("IDEMPOTENCY_CONFLICT","采样计划键已用于不同请求"));}
                return read_stage(db,&id);
            }
            let stage=read_stage(db,&id)?;
            scheduler::validate(&request.policy,stage.total)?;
            if stage.frozen!=stage.total || !matches!(stage.state.as_str(),"ready"|"paused"|"needs_attention"|"failed"|"completed"|"completed_with_exclusions") {
                return Err(Error::new("REVISION_CONFLICT","请等待评审阶段停靠且候选冻结完成"));
            }
            if request.policy.min_exposures < stage.config.request.exposures {
                return Err(Error::invalid("追加计划不能降低原阶段的基础曝光要求"));
            }
            let pending:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM batches WHERE stage_id=?1 AND state IN ('preparing','sent','received'))",[&id],|r|r.get(0)).map_err(db_error)?;
            if pending {return Err(Error::new("REVISION_CONFLICT","仍有正在准备、发送或待解析的批次，请先等待其停靠"));}
            let watermark:u64=db.query_row("SELECT COALESCE(MAX(e.sequence),0) FROM evidence e JOIN batches b ON b.sequence=e.batch WHERE b.stage_id=?1",[&id],|r|crate::unsigned(r,0)).map_err(db_error)?;
            let mut status=initial_status(request.idempotency_key.clone(),request.policy,stage.attempts+u64::from(request.additional_calls),watermark);
            status.previous_plan_id=stage.sampling.as_ref().and_then(|s|if s.round>0 {Some(s.plan_id.clone())} else {s.previous_plan_id.clone()});
            db.execute("INSERT INTO sampling_plans VALUES(?1,?2,?3,?4,?5,?6)",params![request.idempotency_key,id,json,now(),watermark as i64,encode(&status)?]).map_err(db_error)?;
            db.execute("UPDATE stages SET sampling_plan_id=?2,state='ready',error=NULL WHERE id=?1",params![id,request.idempotency_key]).map_err(db_error)?;
            read_stage(db,&id)
        })
    }

    pub fn sampling_diagnostic(
        &self,
        id: &str,
        ordinal: u64,
    ) -> Result<Option<AestheticSamplingDiagnostic>> {
        studio_domain::validate_id(id)?;
        if ordinal > i64::MAX as u64 {
            return Err(Error::invalid("候选序号无效"));
        }
        let db = self.read()?;
        let Some(status) = status(&db, id)? else {
            return Ok(None);
        };
        let json:Option<String>=db.query_row("SELECT data FROM sampling_diagnostics WHERE plan_id=?1 AND round=?2 AND ordinal=?3",params![status.plan_id,status.round,ordinal as i64],|r|r.get(0)).optional().map_err(db_error)?;
        json.map(decode).transpose()
    }

    /// Called only at a drained round boundary. Read/compute outside the writer;
    /// publish the complete round in one bounded transaction after revalidation.
    pub fn plan_sampling(&self, id: &str, check: &dyn Fn() -> Result<()>) -> Result<bool> {
        check()?;
        let stage = self.stage(id)?;
        let Some(status) = stage.sampling.clone() else {
            return Ok(false);
        };
        if stage.state != "running" {
            return Ok(false);
        }
        scheduler::validate(&status.policy, stage.total)?;
        let (input, previous, available, pending, unclaimed) = {
            let db = self.read()?;
            let pending:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM batches WHERE stage_id=?1 AND state NOT IN ('accepted','superseded'))",[id],|r|r.get(0)).map_err(db_error)?;
            let unclaimed:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sampling_queue WHERE plan_id=?1 AND round=?2 AND batch IS NULL)",params![status.plan_id,status.round],|r|r.get(0)).map_err(db_error)?;
            let (watermark,observations):(u64,u64)=db.query_row("SELECT COALESCE(MAX(e.sequence),0),COUNT(*) FROM evidence e JOIN batches b ON b.sequence=e.batch WHERE b.stage_id=?1",[id],|r|Ok((crate::unsigned(r,0)?,crate::unsigned(r,1)?))).map_err(db_error)?;
            let input = AestheticAnalysisInput {
                stage_id: id.into(),
                stage_config_hash: stage.config_hash.clone(),
                evidence_watermark: watermark,
                observations,
                candidates: stage.total,
                review_watermark: 0,
            };
            let inherited = if status.round == 0 {
                status.previous_plan_id.as_ref().map(|plan| {
                    let json:String=db.query_row("SELECT status_json FROM sampling_plans WHERE id=?1 AND stage_id=?2",params![plan,id],|r|r.get(0)).map_err(db_error)?;
                    decode::<AestheticSamplingStatus>(json)
                }).transpose()?
            } else {
                None
            };
            let checkpoint = inherited.as_ref().unwrap_or(&status);
            let mut previous=db.prepare("SELECT data FROM sampling_diagnostics WHERE plan_id=?1 AND round=?2 ORDER BY ordinal LIMIT 10000").map_err(db_error)?.query_map(params![checkpoint.plan_id,checkpoint.round],|r|r.get::<_,String>(0)).map_err(db_error)?.collect::<std::result::Result<Vec<_>,_>>().map_err(db_error)?.into_iter().map(decode).collect::<Result<Vec<AestheticSamplingDiagnostic>>>()?;
            if checkpoint.policy.rank_tolerance > status.policy.rank_tolerance {
                for row in &mut previous {
                    row.stable_rounds = 0;
                }
            }
            let available=db.prepare("SELECT ordinal FROM candidates WHERE stage_id=?1 AND blocked=0 AND reserved=0 AND disposition IN ('active','rejudge') AND rating IN ('g','s','q','e') ORDER BY ordinal LIMIT 10000").map_err(db_error)?.query_map([id],|r|crate::unsigned(r,0)).map_err(db_error)?.collect::<std::result::Result<BTreeSet<_>,_>>().map_err(db_error)?;
            (input, previous, available, pending, unclaimed)
        };
        if pending {
            let mut waiting = status.clone();
            waiting.state = "waiting".into();
            waiting.reason = Some(
                if stage.attempts >= status.call_limit {
                    "call_budget"
                } else {
                    "pending_outcomes"
                }
                .into(),
            );
            let stage_id = id.to_owned();
            return self.writer.submit(4096, move |db| {
                let current = read_stage(db, &stage_id)?;
                if current
                    .sampling
                    .as_ref()
                    .is_some_and(|s| s.plan_id == waiting.plan_id)
                {
                    db.execute(
                        "UPDATE sampling_plans SET status_json=?2 WHERE id=?1",
                        params![waiting.plan_id, encode(&waiting)?],
                    )
                    .map_err(db_error)?;
                }
                Ok(false)
            });
        }
        if unclaimed && stage.attempts < status.call_limit {
            return Ok(true);
        }
        if status.state == "satisfied"
            || (status.state == "limited" && status.evidence_watermark == input.evidence_watermark)
        {
            return Ok(false);
        }
        let round = scheduler::plan(
            self,
            &input,
            status.clone(),
            &previous,
            &available,
            status.call_limit.saturating_sub(stage.attempts),
            check,
        )?;
        check()?;
        let id = id.to_owned();
        let bytes = encode(&round.diagnostics)?.len() + encode(&round.batches)?.len() + 4096;
        self.writer.submit(bytes,move|db|{
            let current=read_stage(db,&id)?;
            if current.state!="running" || current.attempts!=stage.attempts || current.sampling.as_ref().is_none_or(|s|s.plan_id!=status.plan_id || s.round!=status.round) {
                return Err(Error::new("CANCELLED","采样计划发布前阶段已变化"));
            }
            let watermark:u64=db.query_row("SELECT COALESCE(MAX(e.sequence),0) FROM evidence e JOIN batches b ON b.sequence=e.batch WHERE b.stage_id=?1",[&id],|r|crate::unsigned(r,0)).map_err(db_error)?;
            if watermark!=input.evidence_watermark {return Err(Error::new("REVISION_CONFLICT","采样证据水位已变化"));}
            let mut next=round.status;
            // A final diagnostic is also an immutable checkpoint, not an edit
            // to the round that was used for previous paid dispatches.
            next.round=status.round+1;
            let summary=encode(&next)?;
            db.execute("INSERT INTO sampling_rounds VALUES(?1,?2,?3,?4)",params![next.plan_id,next.round,next.evidence_watermark as i64,summary]).map_err(db_error)?;
            {
                let mut insert=db.prepare("INSERT INTO sampling_diagnostics VALUES(?1,?2,?3,?4)").map_err(db_error)?;
                for row in round.diagnostics {
                    if row.reason=="no_comparison_peer" {
                        db.execute("UPDATE candidates SET disposition='needs_review',disposition_reason='no_comparison_peer',blocked=1 WHERE stage_id=?1 AND ordinal=?2 AND reserved=0 AND disposition IN ('active','rejudge')",params![id,row.ordinal as i64]).map_err(db_error)?;
                    }
                    insert.execute(params![next.plan_id,next.round,row.ordinal as i64,encode(&row)?]).map_err(db_error)?;
                }
            }
            let has_batches=!round.batches.is_empty();
            {
                let mut insert=db.prepare("INSERT INTO sampling_queue(plan_id,round,slot,members) VALUES(?1,?2,?3,?4)").map_err(db_error)?;
                for (slot,group) in round.batches.into_iter().enumerate(){insert.execute(params![next.plan_id,next.round,slot as u32,encode(&group)?]).map_err(db_error)?;}
            }
            db.execute("UPDATE sampling_plans SET status_json=?2 WHERE id=?1",params![next.plan_id,summary]).map_err(db_error)?;
            Ok(has_batches)
        })
    }
}
