use super::*;

impl EvaluationDb {
    /// Replace only unsent preparation. Old membership remains immutable for audit.
    pub fn regroup(
        &self,
        stage: &str,
        sequence: u64,
        groups: Vec<Vec<AestheticMember>>,
        rejected: Vec<(u64, String)>,
        reason: &str,
    ) -> Result<()> {
        let stage = stage.to_owned();
        let reason = reason.to_owned();
        self.writer.submit_named(64*1024,"regroup",&stage.clone(),move|db|{
            let parent=read_batch(db,&stage,sequence)?;
            let sent:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM attempts WHERE batch=?1)",[sequence as i64],|r|r.get(0)).map_err(db_error)?;
            if parent.state!="preparing"||sent{return Err(Error::new("REVISION_CONFLICT","已发送批次不可重组"));}
            let expected:std::collections::BTreeSet<_>=parent.members.iter().map(|m|m.candidate.ordinal).collect();
            let actual:Vec<_>=groups.iter().flatten().map(|m|m.candidate.ordinal).chain(rejected.iter().map(|v|v.0)).collect();
            if actual.len()!=expected.len() || actual.iter().copied().collect::<std::collections::BTreeSet<_>>()!=expected || groups.iter().flatten().any(|m|!parent.members.iter().any(|old|old.candidate.key==m.candidate.key && old.candidate.ordinal==m.candidate.ordinal)) {
                return Err(Error::invalid("重组必须完整保留原批次图片身份"));
            }
            db.execute("UPDATE batches SET state='superseded',error=?2 WHERE sequence=?1",params![sequence as i64,match reason.as_str(){"image_preflight_failed"=>"存在无法发送的图片，已隔离异常候选并重组",_=>"超过阶段请求体预算，已在发送前拆分"}]).map_err(db_error)?;
            for (ordinal,message) in rejected {
                db.execute("UPDATE candidates SET reserved=0,blocked=1,disposition='needs_review',disposition_reason=?3 WHERE stage_id=?1 AND ordinal=?2",params![stage,ordinal as i64,message]).map_err(db_error)?;
            }
            for mut group in groups {
                if group.len()<2 {
                    for m in group {db.execute("UPDATE candidates SET reserved=0 WHERE stage_id=?1 AND ordinal=?2",params![stage,m.candidate.ordinal as i64]).map_err(db_error)?;}
                    continue;
                }
                for (i,m) in group.iter_mut().enumerate(){m.label=format!("img{:02}",i+1);}
                let audit = parent.sampling.clone().map(|mut s| { s.members.retain(|r| group.iter().any(|m|m.candidate.ordinal==r.ordinal)); s });
                db.execute("INSERT INTO batches(stage_id,rating,state,members,sampling) VALUES(?1,?2,'queued',?3,?4)",params![stage,parent.rating,encode(&group)?,audit.as_ref().map(encode).transpose()?]).map_err(db_error)?;
                db.execute("INSERT INTO batch_replacements VALUES(?1,?2,?3,?4)",params![sequence as i64,db.last_insert_rowid(),reason,now()]).map_err(db_error)?;
            }
            Ok(())
        })
    }
}
