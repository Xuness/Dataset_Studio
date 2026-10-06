use super::*;

impl AestheticRepository for EvaluationDb {
    fn stage(&self, id: &str) -> Result<AestheticStage> {
        studio_domain::validate_id(id)?;
        read_stage(&*self.read()?, id)
    }
    fn stages(&self, after: Option<&str>, limit: usize) -> Result<Vec<AestheticStage>> {
        let db = self.read()?;
        let mut stmt = db
            .prepare("SELECT id FROM stages WHERE id>?1 ORDER BY id LIMIT ?2")
            .map_err(db_error)?;
        let ids = stmt
            .query_map(
                params![after.unwrap_or(""), limit.clamp(1, 100) as u32],
                |r| r.get::<_, String>(0),
            )
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        ids.iter().map(|id| read_stage(&db, id)).collect()
    }
    fn batches(&self, id: &str, after: u64, limit: usize) -> Result<Vec<AestheticBatch>> {
        let db = self.read()?;
        let mut stmt=db.prepare("SELECT sequence FROM batches WHERE stage_id=?1 AND sequence>?2 ORDER BY sequence LIMIT ?3").map_err(db_error)?;
        let ids = stmt
            .query_map(params![id, after as i64, limit.clamp(1, 100) as u32], |r| {
                crate::unsigned(r, 0)
            })
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        ids.iter().map(|n| read_batch(&db, id, *n)).collect()
    }
    fn attempts(&self, id: &str, batch: u64) -> Result<Vec<AestheticAttempt>> {
        let db = self.read()?;
        read_batch(&db, id, batch)?;
        let mut stmt=db.prepare("SELECT id,state,created_at,receipt,failure,semantic_request_hash,execution_settings FROM attempts WHERE batch=?1 ORDER BY created_at,id LIMIT 8").map_err(db_error)?;
        let rows = stmt
            .query_map([batch as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, Option<String>>(6)?,
                ))
            })
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        rows.into_iter()
            .map(|r| {
                Ok(AestheticAttempt {
                    id: r.0.clone(),
                    batch,
                    state: r.1,
                    created_at: r.2,
                    receipt: r.3.map(decode).transpose()?,
                    failure: r.4.map(decode).transpose()?,
                    semantic_request_hash: r.5,
                    execution_settings: r.6.map(decode).transpose()?,
                    raw_receipt: self.raw_summary(&r.0)?,
                })
            })
            .collect()
    }
    fn receive(&self, id: &str, attempt: &str, receipt: AestheticReceipt) -> Result<()> {
        let id = id.to_owned();
        let attempt = attempt.to_owned();
        let json = encode(&receipt)?;
        self.writer.submit_named(json.len(), "receipt", &id.clone(), move |db| {
            let (sequence,old):(u64,Option<String>)=db.query_row("SELECT a.batch,a.receipt FROM attempts a JOIN batches b ON a.batch=b.sequence WHERE a.id=?1 AND b.stage_id=?2",params![attempt,id],|r|Ok((crate::unsigned(r,0)?,r.get(1)?))).map_err(db_error)?;
            if let Some(old)=old {
                if old!=json {return Err(Error::new("IDEMPOTENCY_CONFLICT","调用结果已保存且内容不同"));}
                return Ok(());
            }
            let batch=read_batch(db,&id,sequence)?;
            if matches!(batch.state.as_str(),"deferred"|"superseded") {return Err(Error::new("EVALUATION_BATCH_CLOSED","已结束的逻辑批次不能再接受结果"));}
            if batch.attempt_id.as_deref()!=Some(&attempt) {return Err(Error::new("REVISION_CONFLICT","调用不是批次的当前尝试"));}
            db.execute("INSERT INTO receipt_parses(attempt_id,adapter_version,state,normalized,created_at) SELECT ?1,json_extract(metadata,'$.adapter_version'),'decoded',?2,?3 FROM raw_receipts WHERE attempt_id=?1",params![attempt,json,now()]).map_err(db_error)?;
            usage::replace(db, &id, None, &receipt.usage)?;
            db.execute("UPDATE attempts SET state='received',receipt=?2 WHERE id=?1",params![attempt,json]).map_err(db_error)?;
            db.execute("UPDATE batches SET state='received',error=NULL WHERE sequence=?1",[sequence as i64]).map_err(db_error)?;
            db.execute("UPDATE stages SET unknown=unknown-?2,input_tokens=input_tokens+?3,output_tokens=output_tokens+?4,usage_unknown=usage_unknown+?5 WHERE id=?1",params![id,(batch.state=="outcome_unknown") as u32,receipt.usage.input_tokens.unwrap_or(0).min(i64::MAX as u64) as i64,receipt.usage.output_tokens.unwrap_or(0).min(i64::MAX as u64) as i64,(receipt.usage.input_tokens.is_none()||receipt.usage.output_tokens.is_none()) as u32]).map_err(db_error)?;
            Ok(())
        })
    }
    fn parse_received(&self, id: &str) -> Result<u64> {
        let mut applied = 0;
        // Each transaction applies one observation. No long read snapshot or full history scan.
        for _ in 0..128 {
            let next = {
                let db = self.read()?;
                db.query_row("SELECT a.id,a.batch,a.receipt FROM attempts a JOIN batches b ON b.sequence=a.batch WHERE a.state='received' AND b.stage_id=?1 ORDER BY a.batch LIMIT 1",[id],|r|Ok((r.get::<_,String>(0)?,crate::unsigned(r,1)?,r.get::<_,String>(2)?))).optional().map_err(db_error)?
            };
            let Some((attempt, sequence, json)) = next else {
                break;
            };
            let batch = read_batch(&*self.read()?, id, sequence)?;
            let receipt: AestheticReceipt = decode(json)?;
            let parsed = parse_observation(&receipt, &batch.members);
            let id = id.to_owned();
            let (count, halt)=self.writer.submit_named(32*1024, "parse", &id.clone(), move |db| {
                let (state,had_failure):(String,bool)=db.query_row("SELECT state,failure IS NOT NULL FROM attempts WHERE id=?1",[&attempt],|r|Ok((r.get(0)?,r.get(1)?))).map_err(db_error)?;
                if state!="received" {return Ok((0,None));}
                match parsed {
                    Ok(observation)=>{
                        let json=encode(&observation)?;
                        db.execute("INSERT INTO evidence(batch,attempt_id,observation,parser_version,accepted_at) VALUES (?1,?2,?3,1,?4)",params![sequence as i64,attempt,json,now()]).map_err(db_error)?;
                        let judged:std::collections::BTreeSet<_>=observation.tiers.iter().flatten().collect();
                        for member in &batch.members {
                            let valid=judged.contains(&member.label);
                            let reason = observation.unjudgeable.iter().find(|v|v.id==member.label).map(|v|v.reason.clone());
                            let elite=observation.elite_candidates.contains(&member.label);
                            // Read live history in the committing transaction, not the
                            // candidate snapshot frozen when this batch was dispatched.
                            let (was_protected,exposures,streak,disposition):(bool,u32,u32,String)=db.query_row("SELECT protected,exposures,unjudgeable_streak,disposition FROM candidates WHERE stage_id=?1 AND ordinal=?2",params![id,member.candidate.ordinal as i64],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(db_error)?;
                            let threshold=studio_application::aesthetic::UNJUDGEABLE_REVIEW_THRESHOLD;
                            let streak=if valid {0} else {(streak+1).min(threshold)};
                            let needs_review=!valid && exposures==0 && streak>=threshold;
                            let next=if valid {"active"} else if needs_review {"needs_review"} else {disposition.as_str()};
                            let sort_key=if valid {hash(&format!("{id}:{}:{}",member.candidate.ordinal,exposures+1))} else {hash(&format!("{id}:{}:{exposures}:unjudgeable:{sequence}",member.candidate.ordinal))};
                            db.execute("UPDATE candidates SET reserved=0,blocked=?3,blocked_batch=NULL,exposures=exposures+?4,protected=MAX(protected,?5),sort_key=?6,disposition=?7,disposition_reason=?8,unjudgeable_streak=?9 WHERE stage_id=?1 AND ordinal=?2",params![id,member.candidate.ordinal as i64,needs_review,valid as u32,elite,sort_key,next,reason,streak]).map_err(db_error)?;
                            if elite && !was_protected {db.execute("UPDATE stages SET protected=protected+1 WHERE id=?1",[&id]).map_err(db_error)?;}
                        }
                        db.execute("UPDATE batches SET state='accepted',observation=?2,error=NULL WHERE sequence=?1",params![sequence as i64,json]).map_err(db_error)?;
                        db.execute("UPDATE attempts SET state='accepted' WHERE id=?1",[&attempt]).map_err(db_error)?;
                        db.execute("UPDATE stages SET accepted=accepted+1,failure_streak=0 WHERE id=?1",[&id]).map_err(db_error)?;
                        Ok((1,None))
                    },
                    Err(error) if error.code=="EVALUATION_NO_COMPARABLE_EVIDENCE"=>{
                        let mut failure=LlmFailure::new(error.code,&error.message);
                        failure.retryable=true;
                        failure.provider_request_id=receipt.provider_request_id;
                        db.execute("UPDATE attempts SET state='failed',failure=?2 WHERE id=?1",params![attempt,encode(&failure)?]).map_err(db_error)?;
                        db.execute("UPDATE batches SET state='failed',error=?2 WHERE sequence=?1",params![sequence as i64,failure.message]).map_err(db_error)?;
                        db.execute("UPDATE stages SET failure_streak=failure_streak+?2 WHERE id=?1",params![id,!had_failure]).map_err(db_error)?;
                        // This is a batch recovery lock, never a candidate disposition
                        // or an unjudgeable strike. Automatic retries only reserve it.
                        for member in &batch.members {db.execute("UPDATE candidates SET reserved=0,blocked=1,blocked_batch=?3 WHERE stage_id=?1 AND ordinal=?2",params![id,member.candidate.ordinal as i64,sequence as i64]).map_err(db_error)?;}
                        let recovery=super::recovery_policy::schedule_recovery(db,&id,sequence,&failure)?;
                        Ok((0,(recovery=="halt").then_some(failure.message)))
                    },
                    Err(error)=>{
                        db.execute("UPDATE batches SET state='invalid',error=?2 WHERE sequence=?1",params![sequence as i64,error.message]).map_err(db_error)?;
                        db.execute("UPDATE attempts SET state='invalid' WHERE id=?1",[&attempt]).map_err(db_error)?;
                        for member in &batch.members {db.execute("UPDATE candidates SET reserved=0,blocked=1,blocked_batch=?3 WHERE stage_id=?1 AND ordinal=?2",params![id,member.candidate.ordinal as i64,sequence as i64]).map_err(db_error)?;}
                        db.execute("UPDATE stages SET invalid=invalid+1 WHERE id=?1",[&id]).map_err(db_error)?;
                        Ok((0,None))
                    }
                }
            })?;
            applied += count;
            // Return the halt only after committing its receipt and failure.
            if let Some(message) = halt {
                return Err(Error::new("EVALUATION_REMOTE", message));
            }
        }
        Ok(applied)
    }
}

impl EvaluationDb {
    pub fn candidate_page(
        &self,
        id: &str,
        after: Option<u64>,
        protected: bool,
    ) -> Result<Vec<AestheticCandidate>> {
        self.filtered_candidates(id, after, protected, None)
    }
    pub fn candidate(&self, id: &str, ordinal: u64) -> Result<AestheticCandidate> {
        if ordinal > i64::MAX as u64 {
            return Err(Error::invalid("候选序号无效"));
        }
        candidates(
            &*self.read()?,
            "WHERE stage_id=?1 AND ordinal=?2",
            params![id, ordinal as i64],
        )?
        .pop()
        .ok_or_else(|| Error::new("NOT_FOUND", "候选不存在"))
    }
    pub fn filtered_candidates(
        &self,
        id: &str,
        after: Option<u64>,
        protected: bool,
        disposition: Option<AestheticDisposition>,
    ) -> Result<Vec<AestheticCandidate>> {
        if after.is_some_and(|v| v > i64::MAX as u64) {
            return Err(Error::invalid("候选游标无效"));
        }
        if let Some(disposition) = disposition {
            return candidates(
                &*self.read()?,
                "WHERE stage_id=?1 AND disposition=?3 AND ordinal>?2 AND (?4=0 OR protected=1) ORDER BY ordinal LIMIT 64",
                params![
                    id,
                    after.map(|v| v as i64).unwrap_or(-1),
                    disposition.as_str(),
                    protected
                ],
            );
        }
        candidates(
            &*self.read()?,
            if protected {
                "WHERE stage_id=?1 AND protected=1 AND ordinal>?2 ORDER BY ordinal LIMIT 64"
            } else {
                "WHERE stage_id=?1 AND ordinal>?2 ORDER BY ordinal LIMIT 64"
            },
            params![id, after.map(|v| v as i64).unwrap_or(-1)],
        )
    }
}
