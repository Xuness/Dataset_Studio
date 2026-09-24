use super::*;

impl EvaluationDb {
    pub fn claim(&self, id: &str) -> Result<Option<AestheticBatch>> {
        let id = id.to_owned();
        self.writer.submit(32*1024, move |db| {
            let stage=read_stage(db,&id)?;
            if stage.state!="running" || stage.attempts>=stage.call_limit() { return Ok(None); }
            let queued: Option<u64>=db.query_row("SELECT sequence FROM batches WHERE stage_id=?1 AND state='queued' ORDER BY sequence LIMIT 1",[&id],|r|crate::unsigned(r,0)).optional().map_err(db_error)?;
            if let Some(sequence)=queued {
                db.execute("UPDATE batches SET state='preparing' WHERE sequence=?1",[sequence as i64]).map_err(db_error)?;
                return read_batch(db,&id,sequence).map(Some);
            }
            if stage.sampling.is_some() { return sampling::claim(db, &stage); }
            for _ in 0..4 {
            let first=candidates(db,"WHERE stage_id=?1 AND blocked=0 AND reserved=0 AND disposition IN ('active','rejudge') AND (exposures<?2 OR disposition='rejudge') ORDER BY exposures,sort_key LIMIT 1",params![id,stage.config.request.exposures])?.pop();
            let Some(first)=first else { return Ok(None) };
            let mut rows=candidates(db,"WHERE stage_id=?1 AND rating=?2 AND blocked=0 AND reserved=0 AND disposition IN ('active','rejudge') AND (exposures<?3 OR disposition='rejudge') ORDER BY exposures,sort_key LIMIT 64",params![id,first.rating,stage.config.request.exposures])?;
            // Baseline only: retain a mixed-year tail, do not partition years into isolated ranks.
            let mut picked=Vec::new();
            for same_year in [true,false] {
                let mut index=0;
                while index<rows.len() && picked.len()<if same_year {12} else {16} {
                    if !same_year || rows[index].year==first.year { picked.push(rows.remove(index)); }
                    else { index+=1; }
                }
            }
            if picked.len() < 2 {
                // A new observation needs a same-Rating comparator, even when
                // every other candidate has already reached the exposure target.
                let anchors=candidates(db,"WHERE stage_id=?1 AND rating=?2 AND disposition='active' AND blocked=0 AND reserved=0 AND exposures>=?3 ORDER BY exposures,sort_key LIMIT 16",params![id,first.rating,stage.config.request.exposures])?;
                for anchor in anchors { if picked.iter().all(|v|v.ordinal!=anchor.ordinal) && picked.len()<16 { picked.push(anchor); } }
                if picked.len()<2 {
                    let waiting:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM candidates WHERE stage_id=?1 AND rating=?2 AND reserved=1)",params![id,first.rating],|r|r.get(0)).map_err(db_error)?;
                    if !waiting { db.execute("UPDATE candidates SET blocked=1,disposition='needs_review',disposition_reason='no_comparison_peer' WHERE stage_id=?1 AND ordinal=?2",params![id,first.ordinal as i64]).map_err(db_error)?; }
                    if waiting { return Ok(None); }
                    continue;
                }
            }
            picked.sort_by_key(|c|hash(&format!("{id}:{}:{}:display",c.ordinal,c.exposures)));
            let members: Vec<_>=picked.into_iter().enumerate().map(|(n,c)|AestheticMember {label:format!("img{:02}",n+1),candidate:c,image_sha256:None}).collect();
            for member in &members {
                db.execute("UPDATE candidates SET reserved=1 WHERE stage_id=?1 AND ordinal=?2",params![id,member.candidate.ordinal as i64]).map_err(db_error)?;
            }
            db.execute("INSERT INTO batches(stage_id,rating,state,members) VALUES (?1,?2,'preparing',?3)",params![id,first.rating,encode(&members)?]).map_err(db_error)?;
            return read_batch(db,&id,db.last_insert_rowid() as u64).map(Some);
            }
            Ok(None)
        })
    }
    pub fn begin_attempt(
        &self,
        id: &str,
        sequence: u64,
        members: Vec<AestheticMember>,
        attempt: String,
        semantic_request_hash: String,
    ) -> Result<bool> {
        let id = id.to_owned();
        self.writer.submit_named(encode(&members)?.len(), "dispatch", &id.clone(), move |db| {
            let s=read_stage(db,&id)?;
            studio_application::aesthetic::validate_capacity(s.total)?;
            studio_application::aesthetic::validate_execution(&s.config)?;
            if semantic_request_hash.len()!=64 || !semantic_request_hash.bytes().all(|v|v.is_ascii_hexdigit()) { return Err(Error::invalid("缺少语义请求摘要")); }
            let batch=read_batch(db,&id,sequence)?;
            if batch.state!="preparing" { return Err(Error::new("REVISION_CONFLICT","批次已被领取")); }
            if s.state!="running" || s.attempts>=s.call_limit() {
                db.execute("UPDATE batches SET state='queued' WHERE sequence=?1",[sequence as i64]).map_err(db_error)?;
                return Ok(false);
            }
            if members.len()!=batch.members.len() || members.iter().zip(&batch.members).any(|(a,b)|a.label!=b.label || a.candidate.ordinal!=b.candidate.ordinal || a.image_sha256.as_ref().is_none_or(|h| h.len()!=64)) {
                return Err(Error::invalid("发送图片映射与固定批次不一致"));
            }
            db.execute("INSERT INTO attempts(id,batch,state,created_at,semantic_request_hash) VALUES (?1,?2,'sent',?3,?4)",params![attempt,sequence as i64,now(),semantic_request_hash]).map_err(db_error)?;
            db.execute("UPDATE batches SET state='sent',attempt_id=?2,members=?3,error=NULL WHERE sequence=?1",params![sequence as i64,attempt,encode(&members)?]).map_err(db_error)?;
            db.execute("UPDATE stages SET attempts=attempts+1 WHERE id=?1",[&id]).map_err(db_error)?;
            Ok(true)
        })
    }
    pub fn preparation_failed(&self, id: &str, sequence: u64, error: String) -> Result<()> {
        let id = id.to_owned();
        self.writer.submit(error.len(), move |db| {
            let batch = read_batch(db, &id, sequence)?;
            if batch.state != "preparing" {
                return Ok(());
            }
            db.execute(
                "UPDATE batches SET state='failed',error=?2 WHERE sequence=?1",
                params![sequence as i64, error],
            )
            .map_err(db_error)?;
            for m in batch.members {
                db.execute(
                    "UPDATE candidates SET reserved=0,blocked=1 WHERE stage_id=?1 AND ordinal=?2",
                    params![id, m.candidate.ordinal as i64],
                )
                .map_err(db_error)?;
            }
            Ok(())
        })
    }
    pub fn fail_attempt(&self, id: &str, attempt: &str, failure: LlmFailure) -> Result<()> {
        let id = id.to_owned();
        let attempt = attempt.to_owned();
        let json = encode(&failure)?;
        self.writer.submit_named(json.len(),"failure",&id.clone(),move |db|{
            let sequence:u64=db.query_row("SELECT a.batch FROM attempts a JOIN batches b ON b.sequence=a.batch WHERE a.id=?1 AND b.stage_id=?2",params![attempt,id],|r|crate::unsigned(r,0)).map_err(db_error)?;
            let batch=read_batch(db,&id,sequence)?;
            let next=if failure.outcome_unknown {"outcome_unknown"}else{"failed"};
            let changed=db.execute("UPDATE attempts SET state=?2,failure=?3 WHERE id=?1 AND state='sent'",params![attempt,next,json]).map_err(db_error)?;
            if changed==0 {return Ok(());}
            db.execute("INSERT INTO receipt_parses(attempt_id,adapter_version,state,error,created_at) SELECT ?1,'native_json_v1','failed',?2,?3 WHERE EXISTS(SELECT 1 FROM raw_receipts WHERE attempt_id=?1)",params![attempt,json,now()]).map_err(db_error)?;
            db.execute("UPDATE batches SET state=?2,error=?3 WHERE sequence=?1",params![sequence as i64,next,failure.message]).map_err(db_error)?;
            db.execute("UPDATE stages SET unknown=unknown+?2 WHERE id=?1",params![id,failure.outcome_unknown as u32]).map_err(db_error)?;
            for m in batch.members { db.execute("UPDATE candidates SET reserved=0,blocked=1 WHERE stage_id=?1 AND ordinal=?2",params![id,m.candidate.ordinal as i64]).map_err(db_error)?; }
            Ok(())
        })
    }
    /// An explicit user operation; retries never create a second accepted observation.
    pub fn retry_batch(&self, id: &str, sequence: u64) -> Result<()> {
        let id = id.to_owned();
        self.writer.submit(1024, move |db| {
            let s = read_stage(db, &id)?;
            if !matches!(
                s.state.as_str(),
                "paused" | "needs_attention" | "failed" | "ready"
            ) {
                return Err(Error::new("REVISION_CONFLICT", "请先等待阶段暂停后重试"));
            }
            if s.attempts >= s.call_limit() {
                return Err(Error::invalid("调用预算已用尽"));
            }
            let batch = read_batch(db, &id, sequence)?;
            if !matches!(
                batch.state.as_str(),
                "invalid" | "failed" | "outcome_unknown"
            ) {
                return Err(Error::invalid("此批次无需付费重试"));
            }
            let attempts: u64 = db
                .query_row(
                    "SELECT count(*) FROM attempts WHERE batch=?1",
                    [sequence as i64],
                    |r| crate::unsigned(r, 0),
                )
                .map_err(db_error)?;
            if attempts >= 8 {
                return Err(Error::invalid(
                    "一个逻辑批次最多保留 8 次调用尝试，请创建新阶段",
                ));
            }
            db.execute(
                "UPDATE stages SET invalid=invalid-?2,unknown=unknown-?3 WHERE id=?1",
                params![
                    id,
                    (batch.state == "invalid") as u32,
                    (batch.state == "outcome_unknown") as u32
                ],
            )
            .map_err(db_error)?;
            db.execute(
                "UPDATE batches SET state='queued',error=NULL WHERE sequence=?1",
                [sequence as i64],
            )
            .map_err(db_error)?;
            for m in batch.members {
                db.execute(
                    "UPDATE candidates SET reserved=1 WHERE stage_id=?1 AND ordinal=?2",
                    params![id, m.candidate.ordinal as i64],
                )
                .map_err(db_error)?;
            }
            Ok(())
        })
    }
}
