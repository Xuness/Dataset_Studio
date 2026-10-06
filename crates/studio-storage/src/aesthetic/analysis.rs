use super::*;
use studio_application::aesthetic_analysis::{self as application, AestheticReplaySource};
use studio_domain::aesthetic_analysis::*;

fn job(db: &Connection, id: &str) -> Result<AestheticAnalysisJob> {
    let row:(String,String,String,u64,u64,String,String,Option<String>,Option<String>,Option<String>)=db.query_row(
        "SELECT created_at,state,phase,progress,total,request_json,input_json,result_json,error,name FROM analysis_jobs WHERE id=?1",[id],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,crate::unsigned(r,3)?,crate::unsigned(r,4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?)))
        .optional().map_err(db_error)?.ok_or_else(||Error::new("NOT_FOUND","离线任务不存在"))?;
    let mut request: AestheticAnalysisCreate = decode(row.5)?;
    // The stored request stays frozen; a user rename only changes the displayed name.
    if let Some(name) = row.9 {
        request.name = name;
    }
    Ok(AestheticAnalysisJob {
        id: id.into(),
        created_at: row.0,
        state: row.1,
        phase: row.2,
        progress: row.3,
        total: row.4,
        request,
        input: decode(row.6)?,
        result: row.7.map(decode).transpose()?,
        error: row.8,
    })
}
fn ready(db: &Connection, id: &str) -> Result<AestheticAnalysisJob> {
    let item = job(db, id)?;
    if item.state != "completed" || !matches!(item.result, Some(AestheticAnalysisSummary::Fit(_))) {
        return Err(Error::new("RESULT_NOT_READY", "需要已发布的排名快照"));
    }
    Ok(item)
}
/// New offline jobs may not read from a removed snapshot; existing results keep their rows.
fn live(db: &Connection, id: &str) -> Result<AestheticAnalysisJob> {
    let deleted: bool = db
        .query_row("SELECT deleted FROM analysis_jobs WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .optional()
        .map_err(db_error)?
        .unwrap_or(false);
    if deleted {
        return Err(Error::new("NOT_FOUND", "排名快照已删除"));
    }
    ready(db, id)
}
fn experiment(db: &Connection, id: &str) -> Result<AestheticExperiment> {
    let (created_at, json, inputs): (String, String, String) = db
        .query_row(
            "SELECT created_at,request_json,inputs_json FROM experiments WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(db_error)?
        .ok_or_else(|| Error::new("NOT_FOUND", "实验不存在"))?;
    Ok(AestheticExperiment {
        id: id.into(),
        created_at,
        request: decode(json)?,
        inputs: decode(inputs)?,
    })
}
fn freeze(db: &Connection, stage_id: &str, watermark: u64) -> Result<AestheticAnalysisInput> {
    let stage = read_stage(db, stage_id)?;
    if stage.frozen != stage.total || stage.total == 0 {
        return Err(Error::new("RESULT_NOT_READY", "候选尚未冻结完成"));
    }
    if stage.total > application::estimator::MAX_CANDIDATES {
        return Err(Error::invalid("单次离线估计最多 10000000 个候选"));
    }
    let observations=db.query_row("SELECT COUNT(*) FROM batches b JOIN evidence e ON e.batch=b.sequence WHERE b.stage_id=?1 AND b.state='accepted' AND e.sequence<=?2",params![stage.id,watermark as i64],|r|crate::unsigned(r,0)).map_err(db_error)?;
    Ok(AestheticAnalysisInput {
        stage_id: stage.id,
        stage_config_hash: stage.config_hash,
        evidence_watermark: watermark,
        observations,
        candidates: stage.total,
        review_watermark: 0,
    })
}
fn mutable(db: &Connection, id: &str) -> Result<()> {
    let state: String = db
        .query_row("SELECT state FROM analysis_jobs WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .map_err(db_error)?;
    if state != "running" {
        return Err(Error::new("CANCELLED", "离线任务已停止"));
    }
    Ok(())
}
pub(super) fn ranking_page_sql(rating: bool) -> &'static str {
    if rating {
        "SELECT data FROM ranking_rows WHERE snapshot_id=?1 AND rating=?3 AND position>?2 ORDER BY position LIMIT ?4"
    } else {
        "SELECT data FROM ranking_rows WHERE snapshot_id=?1 AND position>?2 AND ?3 IS NULL ORDER BY position LIMIT ?4"
    }
}
impl EvaluationDb {
    pub fn latest_stage_snapshot(&self, stage: &str) -> Result<Option<AestheticAnalysisJob>> {
        let db = self.read()?;
        read_stage(&db, stage)?;
        let id:Option<String>=db.query_row("SELECT id FROM analysis_jobs WHERE json_extract(input_json,'$.stage_id')=?1 AND state='completed' AND json_extract(request_json,'$.spec.kind')='fit' AND deleted=0 ORDER BY created_at DESC,id DESC LIMIT 1",[stage],|r|r.get(0)).optional().map_err(db_error)?;
        id.map(|id| job(&db, &id)).transpose()
    }
    pub fn review_watermark(&self) -> Result<u64> {
        self.read()?
            .query_row("SELECT COALESCE(MAX(sequence),0) FROM reviews", [], |r| {
                crate::unsigned(r, 0)
            })
            .map_err(db_error)
    }
    pub fn analysis_create(
        &self,
        request: AestheticAnalysisCreate,
    ) -> Result<AestheticAnalysisJob> {
        application::validate_create(&request)?;
        let json = encode(&request)?;
        self.writer.submit(json.len(),move |db| {
            let id=&request.idempotency_key;
            if let Some(previous)=db.query_row("SELECT request_json FROM analysis_jobs WHERE id=?1",[id],|r|r.get::<_,String>(0)).optional().map_err(db_error)? {
                if previous!=json {return Err(Error::new("IDEMPOTENCY_CONFLICT","离线任务键已用于不同请求"));}
                return job(db,id);
            }
            let mut experiment_id=None;
            let mut input=match &request.spec {
                AestheticAnalysisSpec::Fit{config,experiment_id:exp,variant}=>{
                    if let Some(exp)=exp {
                        let e=experiment(db,exp)?;
                        let index=e.request.variants.iter().position(|v|Some(&v.label)==variant.as_ref() && v.fit==*config).ok_or_else(||Error::invalid("拟合配置与冻结实验变体不一致"))?;
                        experiment_id=Some(exp.clone());
                        e.inputs.get(index).cloned().ok_or_else(||Error::new("EVIDENCE_INVALID","实验输入快照缺失"))?
                    } else {
                        let watermark=db.query_row("SELECT COALESCE(MAX(sequence),0) FROM evidence",[],|r|crate::unsigned(r,0)).map_err(db_error)?;
                        freeze(db,&config.stage_id,watermark)?
                    }
                }
                AestheticAnalysisSpec::Compare{left,right}=>{live(db,right)?;live(db,left)?.input}
                AestheticAnalysisSpec::Derive{snapshot_id,..}|AestheticAnalysisSpec::Preview{snapshot_id,..}=>live(db,snapshot_id)?.input,
            };
            input.review_watermark=db.query_row("SELECT COALESCE(MAX(sequence),0) FROM reviews",[],|r|crate::unsigned(r,0)).map_err(db_error)?;
            if let AestheticAnalysisSpec::Derive{review_watermark:Some(watermark),..}|AestheticAnalysisSpec::Preview{review_watermark:Some(watermark),..}=&request.spec {
                if *watermark>input.review_watermark{return Err(Error::invalid("复核水位不存在"));}
                input.review_watermark = *watermark;
            }
            db.execute("INSERT INTO analysis_jobs(id,created_at,state,phase,request_json,input_json,total,experiment_id) VALUES (?1,?2,'queued','queued',?3,?4,?5,?6)",params![id,now(),json,encode(&input)?,input.candidates as i64,experiment_id]).map_err(db_error)?;
            job(db,id)
        })
    }
    pub fn analysis_job(&self, id: &str) -> Result<AestheticAnalysisJob> {
        studio_domain::validate_id(id)?;
        job(&*self.read()?, id)
    }
    pub fn ranking_snapshot(&self, id: &str) -> Result<AestheticAnalysisJob> {
        ready(&*self.read()?, id)
    }
    pub fn analysis_jobs(
        &self,
        after: &str,
        experiment: Option<&str>,
        limit: usize,
    ) -> Result<Vec<AestheticAnalysisJob>> {
        let db = self.read()?;
        let mut stmt=db.prepare("SELECT id FROM analysis_jobs WHERE id>?1 AND deleted=0 AND (?2 IS NULL OR experiment_id=?2) ORDER BY id LIMIT ?3").map_err(db_error)?;
        let ids = stmt
            .query_map(params![after, experiment, limit.clamp(1, 51) as u32], |r| {
                r.get::<_, String>(0)
            })
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        ids.iter().map(|id| job(&db, id)).collect()
    }
    pub fn analysis_control(&self, id: &str, action: &str) -> Result<AestheticAnalysisJob> {
        let id = id.to_owned();
        let action = action.to_owned();
        self.writer.submit(1024, move |db| {
            let item = job(db, &id)?;
            let state = match (action.as_str(), item.state.as_str()) {
                ("resume", "interrupted" | "failed" | "cancelled" | "queued") => "queued",
                ("cancel", "queued" | "interrupted" | "failed") => "cancelled",
                ("cancel", "running") => "cancelling",
                _ => {
                    return Err(Error::new(
                        "REVISION_CONFLICT",
                        "当前离线任务状态不允许此操作",
                    ));
                }
            };
            db.execute(
                "UPDATE analysis_jobs SET state=?2,error=NULL WHERE id=?1",
                params![id, state],
            )
            .map_err(db_error)?;
            job(db, &id)
        })
    }
    pub fn analysis_rename(&self, id: &str, name: &str) -> Result<AestheticAnalysisJob> {
        studio_domain::validate_id(id)?;
        let name = studio_domain::validate_name(name)?;
        let id = id.to_owned();
        self.writer.submit(name.len() + 1024, move |db| {
            let changed = db
                .execute(
                    "UPDATE analysis_jobs SET name=?2 WHERE id=?1 AND deleted=0",
                    params![id, name],
                )
                .map_err(db_error)?;
            if changed != 1 {
                return Err(Error::new("NOT_FOUND", "离线任务不存在或已删除"));
            }
            job(db, &id)
        })
    }
    /// Tombstones a finished job. Rows stay so dependent comparisons, reviews and
    /// worksets keep their provenance; the job leaves listings and new inputs.
    pub fn analysis_remove(&self, id: &str) -> Result<()> {
        studio_domain::validate_id(id)?;
        let id = id.to_owned();
        self.writer.submit(1024, move |db| {
            let item = job(db, &id)?;
            if matches!(item.state.as_str(), "queued" | "running" | "cancelling") {
                return Err(Error::new(
                    "REVISION_CONFLICT",
                    "请先取消离线任务并等待其停止，再删除",
                ));
            }
            db.execute("UPDATE analysis_jobs SET deleted=1 WHERE id=?1", [&id])
                .map_err(db_error)?;
            Ok(())
        })
    }
    pub fn analysis_start(&self, id: &str) -> Result<AestheticAnalysisJob> {
        let id = id.to_owned();
        self.writer.submit(1024,move |db| {
            if db.execute("UPDATE analysis_jobs SET state='running',phase='loading',progress=0,result_json=NULL,error=NULL WHERE id=?1 AND state='queued'",[&id]).map_err(db_error)?!=1 {
                return Err(Error::new("CANCELLED","任务已被取消或领取"));
            }
            job(db,&id)
        })
    }
    pub fn analysis_progress(
        &self,
        id: &str,
        phase: &str,
        progress: u64,
        total: u64,
    ) -> Result<()> {
        let (id, phase) = (id.to_owned(), phase.to_owned());
        self.writer.submit(1024, move |db| {
            mutable(db, &id)?;
            db.execute(
                "UPDATE analysis_jobs SET phase=?2,progress=?3,total=?4 WHERE id=?1",
                params![id, phase, progress as i64, total as i64],
            )
            .map_err(db_error)?;
            Ok(())
        })
    }
    pub fn analysis_reset_page(&self, id: &str) -> Result<bool> {
        let id = id.to_owned();
        self.writer.submit(1024,move |db| {
            mutable(db,&id)?;
            let a=db.execute("DELETE FROM ranking_rows WHERE snapshot_id=?1 AND position IN (SELECT position FROM ranking_rows WHERE snapshot_id=?1 ORDER BY position LIMIT 256)",[&id]).map_err(db_error)?;
            let b=db.execute("DELETE FROM comparison_rows WHERE job_id=?1 AND position IN (SELECT position FROM comparison_rows WHERE job_id=?1 ORDER BY position LIMIT 256)",[&id]).map_err(db_error)?;
            Ok(a+b>0)
        })
    }
    pub fn analysis_fail(&self, id: &str, state: &str, message: &str) -> Result<()> {
        if !matches!(state, "failed" | "interrupted" | "cancelled") {
            return Err(Error::invalid("无效失败状态"));
        }
        let (id, state, message) = (
            id.to_owned(),
            state.to_owned(),
            message.chars().take(4000).collect::<String>(),
        );
        self.writer.submit(message.len(),move |db| {
            db.execute("UPDATE analysis_jobs SET state=CASE WHEN state='cancelling' THEN 'cancelled' ELSE ?2 END,error=?3 WHERE id=?1 AND state NOT IN ('completed','cancelled')",params![id,state,message]).map_err(db_error)?;Ok(())
        })
    }
    pub fn analysis_finish(&self, id: &str, result: AestheticAnalysisSummary) -> Result<()> {
        let id = id.to_owned();
        let json = encode(&result)?;
        self.writer.submit(json.len(),move |db| {
            let item=job(db,&id)?;
            if item.state=="completed" {return Ok(());}
            if !matches!(result,AestheticAnalysisSummary::Derive{..}) {mutable(db,&id)?;}
            if matches!(result,AestheticAnalysisSummary::Fit(_)|AestheticAnalysisSummary::Compare{..}) {
                let table=if matches!(result,AestheticAnalysisSummary::Fit(_)){"ranking_rows WHERE snapshot_id"}else{"comparison_rows WHERE job_id"};
                let count=db.query_row(&format!("SELECT count(*) FROM {table}=?1"),[&id],|r|crate::unsigned(r,0)).map_err(db_error)?;
                if count!=item.input.candidates {return Err(Error::new("EVIDENCE_INVALID","投影发布成员数不完整"));}
            }
            db.execute("UPDATE analysis_jobs SET state='completed',phase='completed',progress=?3,total=?3,result_json=?2,error=NULL WHERE id=?1",params![id,json,item.input.candidates as i64]).map_err(db_error)?;Ok(())
        })
    }
    pub fn append_ranking_rows(&self, id: &str, rows: Vec<AestheticRankingRow>) -> Result<()> {
        if rows.len() > 256 {
            return Err(Error::invalid("排名写入批次过大"));
        }
        let id = id.to_owned();
        self.writer.submit(encode(&rows)?.len(), move |db| {
            mutable(db, &id)?;
            let mut stmt = db
                .prepare_cached("INSERT INTO ranking_rows VALUES (?1,?2,?3,?4,?5,?6,?7,?8)")
                .map_err(db_error)?;
            for r in rows {
                stmt.execute(params![
                    id,
                    r.position as i64,
                    r.ordinal as i64,
                    r.key.source_id,
                    r.key.asset_id,
                    r.rating,
                    r.content_version,
                    encode(&r)?
                ])
                .map_err(db_error)?;
            }
            Ok(())
        })
    }
    pub fn ranking_page(
        &self,
        id: &str,
        after: u64,
        rating: Option<&str>,
        limit: usize,
    ) -> Result<Vec<AestheticRankingRow>> {
        let db = self.read()?;
        ready(&db, id)?;
        let mut stmt = db
            .prepare(ranking_page_sql(rating.is_some()))
            .map_err(db_error)?;
        let rows = stmt
            .query_map(
                params![id, after as i64, rating, limit.clamp(1, 256) as u32],
                |r| r.get::<_, String>(0),
            )
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        rows.into_iter().map(decode).collect()
    }
    pub fn ranking_candidate(&self, id: &str, ordinal: u64) -> Result<AestheticRankingRow> {
        let db = self.read()?;
        ready(&db, id)?;
        let json = db
            .query_row(
                "SELECT data FROM ranking_rows WHERE snapshot_id=?1 AND ordinal=?2",
                params![id, ordinal as i64],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(db_error)?
            .ok_or_else(|| Error::new("NOT_FOUND", "快照图片不存在"))?;
        decode(json)
    }
    /// Bounded indexed join; no full set of asset identities is loaded by the caller.
    pub fn comparison_input_page(
        &self,
        left: &str,
        right: &str,
        after: u64,
    ) -> Result<Vec<(AestheticRankingRow, Option<AestheticRankingRow>)>> {
        let db = self.read()?;
        ready(&db, left)?;
        ready(&db, right)?;
        let mut stmt=db.prepare("SELECT l.data,r.data FROM ranking_rows l LEFT JOIN ranking_rows r ON r.snapshot_id=?2 AND r.source_id=l.source_id AND r.asset_id=l.asset_id WHERE l.snapshot_id=?1 AND l.position>?3 ORDER BY l.position LIMIT 128").map_err(db_error)?;
        let rows = stmt
            .query_map(params![left, right, after as i64], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
            })
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        rows.into_iter()
            .map(|(a, b)| Ok((decode(a)?, b.map(decode).transpose()?)))
            .collect()
    }
    pub fn append_comparison_rows(
        &self,
        id: &str,
        rows: Vec<AestheticComparisonRow>,
    ) -> Result<()> {
        if rows.len() > 256 {
            return Err(Error::invalid("对照写入批次过大"));
        }
        let id = id.to_owned();
        self.writer.submit(encode(&rows)?.len(), move |db| {
            mutable(db, &id)?;
            for row in rows {
                db.execute(
                    "INSERT INTO comparison_rows VALUES (?1,?2,?3)",
                    params![id, row.position as i64, encode(&row)?],
                )
                .map_err(db_error)?;
            }
            Ok(())
        })
    }
    pub fn comparison_page(
        &self,
        id: &str,
        after: u64,
        limit: usize,
    ) -> Result<Vec<AestheticComparisonRow>> {
        let db = self.read()?;
        let item = job(&db, id)?;
        if item.state != "completed"
            || !matches!(item.result, Some(AestheticAnalysisSummary::Compare { .. }))
        {
            return Err(Error::new("RESULT_NOT_READY", "对照尚未发布"));
        }
        let mut stmt=db.prepare("SELECT data FROM comparison_rows WHERE job_id=?1 AND position>?2 ORDER BY position LIMIT ?3").map_err(db_error)?;
        let rows = stmt
            .query_map(params![id, after as i64, limit.clamp(1, 129) as u32], |r| {
                r.get::<_, String>(0)
            })
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        rows.into_iter().map(decode).collect()
    }
    pub fn experiment_create(
        &self,
        value: AestheticExperimentCreate,
    ) -> Result<AestheticExperiment> {
        application::validate_experiment(&value)?;
        let json = encode(&value)?;
        self.writer.submit(json.len(), move |db| {
            let id = &value.idempotency_key;
            if let Some(old) = db
                .query_row(
                    "SELECT request_json FROM experiments WHERE id=?1",
                    [id],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .map_err(db_error)?
            {
                if old != json {
                    return Err(Error::new("IDEMPOTENCY_CONFLICT", "实验键已用于其他配置"));
                }
            } else {
                let watermark = db
                    .query_row("SELECT COALESCE(MAX(sequence),0) FROM evidence", [], |r| {
                        crate::unsigned(r, 0)
                    })
                    .map_err(db_error)?;
                let inputs = value
                    .variants
                    .iter()
                    .map(|v| freeze(db, &v.fit.stage_id, watermark))
                    .collect::<Result<Vec<_>>>()?;
                db.execute(
                    "INSERT INTO experiments VALUES (?1,?2,?3,?4)",
                    params![id, now(), json, encode(&inputs)?],
                )
                .map_err(db_error)?;
            }
            experiment(db, id)
        })
    }
    pub fn experiment(&self, id: &str) -> Result<AestheticExperiment> {
        experiment(&*self.read()?, id)
    }
    pub fn experiments(&self, after: &str) -> Result<Vec<AestheticExperiment>> {
        let db = self.read()?;
        let mut stmt = db
            .prepare("SELECT id FROM experiments WHERE id>?1 ORDER BY id LIMIT 26")
            .map_err(db_error)?;
        let ids = stmt
            .query_map([after], |r| r.get::<_, String>(0))
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        ids.iter().map(|id| experiment(&db, id)).collect()
    }
    pub fn review_create(&self, value: AestheticReviewCreate) -> Result<AestheticReview> {
        application::validate_review(&value)?;
        let json = encode(&value)?;
        self.writer.submit(json.len(),move |db| {
            ready(db,&value.snapshot_id)?;
            let exists:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM ranking_rows WHERE snapshot_id=?1 AND ordinal=?2)",params![value.snapshot_id,value.ordinal as i64],|r|r.get(0)).map_err(db_error)?;
            if !exists {return Err(Error::new("NOT_FOUND","复核图片不属于该快照"));}
            if let Some(old)=db.query_row("SELECT request_json FROM reviews WHERE request_id=?1",[&value.idempotency_key],|r|r.get::<_,String>(0)).optional().map_err(db_error)? {
                if old!=json {return Err(Error::new("IDEMPOTENCY_CONFLICT","复核键已用于不同决定"));}
            } else {db.execute("INSERT INTO reviews(request_id,snapshot_id,ordinal,created_at,decision,request_json) VALUES (?1,?2,?3,?4,?5,?6)",params![value.idempotency_key,value.snapshot_id,value.ordinal as i64,now(),value.decision,json]).map_err(db_error)?;}
            let (sequence,created_at)=db.query_row("SELECT sequence,created_at FROM reviews WHERE request_id=?1",[&value.idempotency_key],|r|Ok((crate::unsigned(r,0)?,r.get(1)?))).map_err(db_error)?;
            Ok(AestheticReview{sequence,created_at,request:value})
        })
    }
    /// Uses the existing (snapshot_id, ordinal, sequence) index, latest first.
    pub fn candidate_reviews(
        &self,
        snapshot: &str,
        ordinal: u64,
        before: u64,
    ) -> Result<Vec<AestheticReview>> {
        if ordinal > i64::MAX as u64 || before > i64::MAX as u64 {
            return Err(Error::invalid("复核图片或游标无效"));
        }
        self.ranking_candidate(snapshot, ordinal)?;
        let db = self.read()?;
        let mut stmt = db.prepare("SELECT sequence,created_at,request_json FROM reviews WHERE snapshot_id=?1 AND ordinal=?2 AND sequence<?3 ORDER BY sequence DESC LIMIT 65").map_err(db_error)?;
        let rows = stmt
            .query_map(
                params![
                    snapshot,
                    ordinal as i64,
                    if before == 0 { i64::MAX } else { before as i64 }
                ],
                |r| {
                    Ok((
                        crate::unsigned(r, 0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        rows.into_iter()
            .map(|(sequence, created_at, json)| {
                Ok(AestheticReview {
                    sequence,
                    created_at,
                    request: decode(json)?,
                })
            })
            .collect()
    }
    pub fn reviews(&self, snapshot: &str, after: u64) -> Result<Vec<AestheticReview>> {
        let db = self.read()?;
        ready(&db, snapshot)?;
        let mut stmt=db.prepare("SELECT sequence,created_at,request_json FROM reviews WHERE snapshot_id=?1 AND sequence>?2 ORDER BY sequence LIMIT 65").map_err(db_error)?;
        let rows = stmt
            .query_map(params![snapshot, after as i64], |r| {
                Ok((
                    crate::unsigned(r, 0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        rows.into_iter()
            .map(|(sequence, created_at, json)| {
                Ok(AestheticReview {
                    sequence,
                    created_at,
                    request: decode(json)?,
                })
            })
            .collect()
    }
    pub fn effective_protection(
        &self,
        snapshot: &str,
        rows: &[AestheticRankingRow],
        watermark: u64,
    ) -> Result<Vec<bool>> {
        if rows.len() > 256 {
            return Err(Error::invalid("复核查询批次过大"));
        }
        let db = self.read()?;
        let mut stmt=db.prepare("SELECT decision FROM reviews WHERE snapshot_id=?1 AND ordinal=?2 AND sequence<=?3 ORDER BY sequence DESC LIMIT 1").map_err(db_error)?;
        rows.iter()
            .map(|row| {
                let decision = stmt
                    .query_row(
                        params![snapshot, row.ordinal as i64, watermark as i64],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(db_error)?;
                Ok(match decision.as_deref() {
                    Some("protect" | "confirm_elite" | "defer") => true,
                    Some("release") => false,
                    _ => row.protected,
                })
            })
            .collect()
    }
}
impl AestheticReplaySource for EvaluationDb {
    fn replay_candidates(
        &self,
        stage: &str,
        after: Option<u64>,
    ) -> Result<Vec<AestheticCandidate>> {
        self.candidate_page(stage, after, false)
    }
    fn replay_observations(
        &self,
        input: &AestheticAnalysisInput,
        after: u64,
    ) -> Result<Vec<AestheticReplayObservation>> {
        let db = self.read()?;
        let mut stmt=db.prepare("SELECT b.sequence,b.rating,b.members,e.observation,e.parser_version FROM batches b JOIN evidence e ON e.batch=b.sequence WHERE b.stage_id=?1 AND b.state='accepted' AND b.sequence>?2 AND e.sequence<=?3 ORDER BY b.sequence LIMIT 64").map_err(db_error)?;
        let rows = stmt
            .query_map(
                params![
                    input.stage_id,
                    after as i64,
                    input.evidence_watermark as i64
                ],
                |r| {
                    Ok((
                        crate::unsigned(r, 0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, u32>(4)?,
                    ))
                },
            )
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        rows.into_iter()
            .map(|(batch, rating, members, observation, version)| {
                if version != 1 {
                    return Err(Error::new(
                        "FORMAT_UNSUPPORTED",
                        "离线重放不支持此解析器版本",
                    ));
                }
                let members: Vec<AestheticMember> = decode(members)?;
                let observation: AestheticObservation = decode(observation)?;
                studio_application::aesthetic::validate_observation(&observation, &members)?;
                let ordinal = |label: &str| -> Result<u32> {
                    let n = members
                        .iter()
                        .find(|m| m.label == label)
                        .ok_or_else(|| Error::new("EVIDENCE_INVALID", "证据标签不属于批次"))?
                        .candidate
                        .ordinal;
                    u32::try_from(n).map_err(|_| Error::invalid("候选序号过大"))
                };
                Ok(AestheticReplayObservation {
                    batch,
                    rating,
                    tiers: observation
                        .tiers
                        .iter()
                        .map(|tier| tier.iter().map(|s| ordinal(s)).collect())
                        .collect::<Result<_>>()?,
                    elite: observation
                        .elite_candidates
                        .iter()
                        .map(|s| ordinal(s))
                        .collect::<Result<_>>()?,
                    unjudgeable: observation
                        .unjudgeable
                        .iter()
                        .map(|s| ordinal(&s.id))
                        .collect::<Result<_>>()?,
                })
            })
            .collect()
    }
}
