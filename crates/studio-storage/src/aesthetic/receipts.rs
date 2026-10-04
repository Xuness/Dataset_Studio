use super::*;
use studio_domain::llm::LlmRawReceipt;

impl EvaluationDb {
    pub fn save_raw(&self, stage: &str, attempt: &str, receipt: LlmRawReceipt) -> Result<()> {
        if receipt.body.len() > studio_domain::llm::LLM_RECEIPT_LIMIT {
            return Err(Error::invalid("原始回执超限"));
        }
        let metadata = encode(&receipt)?;
        let digest = hex::encode(Sha256::digest(&receipt.body));
        let stage = stage.to_owned();
        let attempt = attempt.to_owned();
        self.writer.submit_named(receipt.body.len()+metadata.len(), "raw_receipt", &stage.clone(), move |db| {
            let exists: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM attempts a JOIN batches b ON b.sequence=a.batch WHERE a.id=?1 AND b.stage_id=?2)",params![attempt,stage],|r|r.get(0)).map_err(db_error)?;
            if !exists { return Err(Error::new("NOT_FOUND","调用不属于该阶段")); }
            let old:Option<(String,String)> = db.query_row("SELECT metadata,sha256 FROM raw_receipts WHERE attempt_id=?1",[&attempt],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(db_error)?;
            if let Some(old)=old {
                if old!=(metadata,digest) { return Err(Error::new("IDEMPOTENCY_CONFLICT","原始回执不可覆盖")); }
            } else {
                db.execute("INSERT INTO raw_receipts VALUES(?1,?2,?3,?4,?5)",params![attempt,metadata,receipt.body,digest,now()]).map_err(db_error)?;
            }
            Ok(())
        })
    }
    pub fn raw_receipt(&self, stage: &str, attempt: &str) -> Result<Option<LlmRawReceipt>> {
        let db = self.read()?;
        let row:Option<(String,Vec<u8>,String)>=db.query_row("SELECT r.metadata,r.body,r.sha256 FROM raw_receipts r JOIN attempts a ON a.id=r.attempt_id JOIN batches b ON b.sequence=a.batch WHERE a.id=?1 AND b.stage_id=?2",params![attempt,stage],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(db_error)?;
        row.map(|(meta, body, digest)| {
            if hex::encode(Sha256::digest(&body)) != digest {
                return Err(Error::new("EVALUATION_CORRUPT", "原始回执摘要校验失败"));
            }
            let mut receipt: LlmRawReceipt = decode(meta)?;
            receipt.body = body;
            Ok(receipt)
        })
        .transpose()
    }
    pub fn raw_summary(&self, attempt: &str) -> Result<Option<AestheticRawSummary>> {
        let db = self.read()?;
        db.query_row("SELECT sha256,length(body),json_extract(metadata,'$.complete'),json_extract(metadata,'$.http_status') FROM raw_receipts WHERE attempt_id=?1",[attempt],|r|Ok(AestheticRawSummary{sha256:r.get(0)?,bytes:crate::unsigned(r,1)?,complete:r.get(2)?,http_status:r.get(3)?})).optional().map_err(db_error)
    }
    pub fn record_parse(&self, stage: &str, attempt: &str, error: Option<String>) -> Result<()> {
        let stage = stage.to_owned();
        let attempt = attempt.to_owned();
        self.writer.submit_named(2048,"receipt_parse",&stage.clone(),move|db|{
            db.execute("INSERT INTO receipt_parses(attempt_id,adapter_version,state,error,created_at) SELECT a.id,COALESCE((SELECT json_extract(metadata,'$.adapter_version') FROM raw_receipts WHERE attempt_id=a.id),'native_json_v1'),?3,?4,?5 FROM attempts a JOIN batches b ON b.sequence=a.batch WHERE a.id=?1 AND b.stage_id=?2",params![attempt,stage,if error.is_some(){"failed"}else{"decoded"},error,now()]).map_err(db_error)?;
            Ok(())
        })
    }
}

impl EvaluationDb {
    pub fn apply_reparsed(
        &self,
        stage: &str,
        attempt: &str,
        receipt: AestheticReceipt,
    ) -> Result<()> {
        let stage = stage.to_owned();
        let attempt = attempt.to_owned();
        let json = encode(&receipt)?;
        self.writer.submit_named(json.len(),"receipt_parse",&stage.clone(),move|db|{
            let batch:u64=db.query_row("SELECT b.sequence FROM batches b WHERE b.stage_id=?1 AND b.attempt_id=?2",params![stage,attempt],|r|crate::unsigned(r,0)).optional().map_err(db_error)?.ok_or_else(||Error::new("REVISION_CONFLICT","此调用已被另一尝试替代"))?;
            if db.query_row("SELECT EXISTS(SELECT 1 FROM evidence WHERE batch=?1)",[batch as i64],|r|r.get::<_,bool>(0)).map_err(db_error)? {return Ok(());}
            if db.query_row("SELECT state IN ('deferred','superseded') FROM batches WHERE sequence=?1",[batch as i64],|r|r.get::<_,bool>(0)).map_err(db_error)? {return Err(Error::new("EVALUATION_BATCH_CLOSED","已结束的逻辑批次不再接受迟到结果；原始回执保留"));}
            let state:String=db.query_row("SELECT state FROM stages WHERE id=?1",[&stage],|r|r.get(0)).map_err(db_error)?;
            if matches!(state.as_str(),"running"|"preparing"|"pausing"|"cancelling") {return Err(Error::new("REVISION_CONFLICT","阶段正在执行"));}
            let (old,state):(Option<String>,String)=db.query_row("SELECT a.receipt,b.state FROM attempts a JOIN batches b ON b.sequence=a.batch WHERE a.id=?1",[&attempt],|r|Ok((r.get(0)?,r.get(1)?))).map_err(db_error)?;
            let old_usage=old.as_ref().map(|s|decode::<AestheticReceipt>(s.clone())).transpose()?.map(|r|r.usage);
            if let Some(old)=old {db.execute("INSERT INTO receipt_parses(attempt_id,adapter_version,state,normalized,created_at) VALUES(?1,'historical','previous_normalization',?2,?3)",params![attempt,old,now()]).map_err(db_error)?;}
            db.execute("INSERT INTO receipt_parses(attempt_id,adapter_version,state,normalized,created_at) SELECT ?1,COALESCE((SELECT json_extract(metadata,'$.adapter_version') FROM raw_receipts WHERE attempt_id=?1),'native_json_v1'),'decoded',?2,?3",params![attempt,json,now()]).map_err(db_error)?;
            let bound=|v:Option<u64>|v.unwrap_or(0).min(i64::MAX as u64) as i64;
            let input=bound(receipt.usage.input_tokens)-old_usage.as_ref().map_or(0,|u|bound(u.input_tokens));
            let output=bound(receipt.usage.output_tokens)-old_usage.as_ref().map_or(0,|u|bound(u.output_tokens));
            let unknown=(receipt.usage.input_tokens.is_none()||receipt.usage.output_tokens.is_none()) as i64-old_usage.as_ref().map_or(0,|u|(u.input_tokens.is_none()||u.output_tokens.is_none()) as i64);
            db.execute("UPDATE stages SET invalid=invalid-?2,unknown=unknown-?3,input_tokens=input_tokens+?4,output_tokens=output_tokens+?5,usage_unknown=usage_unknown+?6 WHERE id=?1",params![stage,(state=="invalid") as u32,(state=="outcome_unknown") as u32,input,output,unknown]).map_err(db_error)?;
            db.execute("UPDATE attempts SET state='received',receipt=?2 WHERE id=?1",params![attempt,json]).map_err(db_error)?;
            db.execute("UPDATE batches SET state='received',error=NULL WHERE sequence=?1",[batch as i64]).map_err(db_error)?;
            Ok(())
        })
    }
}
