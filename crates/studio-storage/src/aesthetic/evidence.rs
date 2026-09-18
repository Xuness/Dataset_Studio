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
        let mut stmt=db.prepare("SELECT id,state,created_at,receipt,failure FROM attempts WHERE batch=?1 ORDER BY created_at,id LIMIT 8").map_err(db_error)?;
        let rows = stmt
            .query_map([batch as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        rows.into_iter()
            .map(|r| {
                Ok(AestheticAttempt {
                    id: r.0,
                    batch,
                    state: r.1,
                    created_at: r.2,
                    receipt: r.3.map(decode).transpose()?,
                    failure: r.4.map(decode).transpose()?,
                })
            })
            .collect()
    }
    fn receive(&self, id: &str, attempt: &str, receipt: AestheticReceipt) -> Result<()> {
        let id = id.to_owned();
        let attempt = attempt.to_owned();
        let json = encode(&receipt)?;
        self.writer.submit(json.len(),move |db| {
            let (sequence,old):(u64,Option<String>)=db.query_row("SELECT a.batch,a.receipt FROM attempts a JOIN batches b ON a.batch=b.sequence WHERE a.id=?1 AND b.stage_id=?2",params![attempt,id],|r|Ok((crate::unsigned(r,0)?,r.get(1)?))).map_err(db_error)?;
            if let Some(old)=old {
                if old!=json {return Err(Error::new("IDEMPOTENCY_CONFLICT","调用结果已保存且内容不同"));}
                return Ok(());
            }
            let batch=read_batch(db,&id,sequence)?;
            if batch.attempt_id.as_deref()!=Some(&attempt) {return Err(Error::new("REVISION_CONFLICT","调用不是批次的当前尝试"));}
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
            applied+=self.writer.submit(32*1024,move |db| {
                let state:String=db.query_row("SELECT state FROM attempts WHERE id=?1",[&attempt],|r|r.get(0)).map_err(db_error)?;
                if state!="received" {return Ok(0);}
                match parsed {
                    Ok(observation)=>{
                        let json=encode(&observation)?;
                        db.execute("INSERT INTO evidence(batch,attempt_id,observation,parser_version,accepted_at) VALUES (?1,?2,?3,1,?4)",params![sequence as i64,attempt,json,now()]).map_err(db_error)?;
                        let judged:std::collections::BTreeSet<_>=observation.tiers.iter().flatten().collect();
                        for member in &batch.members {
                            let valid=judged.contains(&member.label);
                            let elite=observation.elite_candidates.contains(&member.label);
                            let was_protected:bool=db.query_row("SELECT protected FROM candidates WHERE stage_id=?1 AND ordinal=?2",params![id,member.candidate.ordinal as i64],|r|r.get(0)).map_err(db_error)?;
                            db.execute("UPDATE candidates SET reserved=0,blocked=?3,exposures=exposures+?4,protected=MAX(protected,?5),sort_key=?6 WHERE stage_id=?1 AND ordinal=?2",params![id,member.candidate.ordinal as i64,!valid,valid as u32,elite,hash(&format!("{id}:{}:{}",member.candidate.ordinal,member.candidate.exposures+u32::from(valid)))]).map_err(db_error)?;
                            if elite && !was_protected {db.execute("UPDATE stages SET protected=protected+1 WHERE id=?1",[&id]).map_err(db_error)?;}
                        }
                        db.execute("UPDATE batches SET state='accepted',observation=?2,error=NULL WHERE sequence=?1",params![sequence as i64,json]).map_err(db_error)?;
                        db.execute("UPDATE attempts SET state='accepted' WHERE id=?1",[&attempt]).map_err(db_error)?;
                        db.execute("UPDATE stages SET accepted=accepted+1 WHERE id=?1",[&id]).map_err(db_error)?;
                        Ok(1)
                    },
                    Err(error)=>{
                        db.execute("UPDATE batches SET state='invalid',error=?2 WHERE sequence=?1",params![sequence as i64,error.message]).map_err(db_error)?;
                        db.execute("UPDATE attempts SET state='invalid' WHERE id=?1",[&attempt]).map_err(db_error)?;
                        for member in &batch.members {db.execute("UPDATE candidates SET reserved=0,blocked=1 WHERE stage_id=?1 AND ordinal=?2",params![id,member.candidate.ordinal as i64]).map_err(db_error)?;}
                        db.execute("UPDATE stages SET invalid=invalid+1 WHERE id=?1",[&id]).map_err(db_error)?;
                        Ok(0)
                    }
                }
            })?;
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
