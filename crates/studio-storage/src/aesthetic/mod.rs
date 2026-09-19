//! Project-owned paid evidence. Never part of the disposable query/preview caches.
use crate::{now, read_pool::ReadPool};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use studio_application::aesthetic::{AestheticRepository, parse_observation};
use studio_domain::{Error, Result, aesthetic::*, llm::LlmFailure};
pub mod analysis;
mod analysis_project;
mod creation;
mod decisions;
mod dispatch;
mod evidence;
mod project;
mod writer;
pub(crate) use project::{recover, reference_reason};
#[cfg(test)]
mod tests;

pub struct EvaluationDb {
    path: PathBuf,
    writer: writer::Writer,
    reads: ReadPool,
}
fn db_error(error: rusqlite::Error) -> Error {
    let code = match error.sqlite_error_code() {
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) => {
            "EVALUATION_BUSY"
        }
        Some(rusqlite::ErrorCode::DiskFull) => "EVALUATION_STORAGE_FULL",
        Some(rusqlite::ErrorCode::SystemIoFailure | rusqlite::ErrorCode::CannotOpen) => {
            "EVALUATION_STORAGE_IO"
        }
        Some(rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase) => {
            "EVALUATION_CORRUPT"
        }
        _ => "DATABASE_ERROR",
    };
    Error::new(code, error.to_string())
}
fn encode(value: &impl Serialize) -> Result<String> {
    serde_json::to_string(value).map_err(Error::io)
}
fn decode<T: DeserializeOwned>(value: String) -> Result<T> {
    serde_json::from_str(&value).map_err(Error::io)
}
fn hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

impl EvaluationDb {
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self {
            path: path.to_owned(),
            writer: writer::Writer::open(path)?,
            reads: Default::default(),
        })
    }
    fn read(&self) -> Result<crate::read_pool::ReadGuard<'_>> {
        self.reads.read(&self.path)
    }
    pub fn metrics(&self) -> Result<(u64, u64)> {
        let q = self.writer.stats.lock().map_err(crate::lock_error)?;
        Ok((q.bytes as u64, q.peak as u64))
    }
    pub fn create(&self, config: AestheticConfig, total: u64) -> Result<AestheticStage> {
        let json = encode(&config)?;
        let request = encode(&config.request)?;
        let id = config.request.idempotency_key.clone();
        self.writer.submit(json.len(), move |db| {
            let old: Option<String> = db.query_row("SELECT request_json FROM stages WHERE id=?1", [&id], |r| r.get(0)).optional().map_err(db_error)?;
            if let Some(old) = old {
                if old != request { return Err(Error::new("IDEMPOTENCY_CONFLICT", "评审创建键已被不同请求使用")); }
                let saved=read_stage(db,&id)?;
                if saved.total!=total || serde_json::to_value(&saved.config).map_err(Error::io)?!=serde_json::to_value(&config).map_err(Error::io)? {
                    return Err(Error::new("EVALUATION_RECONCILIATION_FAILED","两库的冻结配置或总量不一致"));
                }
            } else {
                studio_application::aesthetic::validate_capacity(total)?;
                db.execute("INSERT INTO stages(id,name,state,created_at,config,config_hash,request_json,total) VALUES (?1,?2,'preparing',?3,?4,?5,?6,?7)",
                    params![id,config.request.name,now(),json,hash(&json),request,total as i64]).map_err(db_error)?;
            }
            read_stage(db, &id)
        })
    }
    pub fn append_candidates(&self, id: &str, rows: Vec<AestheticCandidate>) -> Result<()> {
        let id = id.to_owned();
        if rows.len() > 128 {
            return Err(Error::invalid("候选冻结批次过大"));
        }
        self.writer.submit(encode(&rows)?.len(), move |db| {
            let stage = read_stage(db, &id)?;
            if stage.state != "preparing" { return Err(Error::new("CANCELLED", "候选冻结已暂停")); }
            if stage.frozen + rows.len() as u64 > stage.total { return Err(Error::new("SOURCE_CHANGED", "候选数超过创建时冻结的工作集总量")); }
            for (offset, row) in rows.iter().enumerate() {
                if row.ordinal != stage.frozen + offset as u64 { return Err(Error::invalid("候选冻结游标不连续")); }
                let eligible = matches!(row.rating.as_str(), "g"|"s"|"q"|"e");
                db.execute("INSERT INTO candidates(stage_id,ordinal,source_id,asset_id,rating,year,basis,content_version,bytes,blocked,sort_key) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                    params![id,row.ordinal as i64,row.key.source_id,row.key.asset_id,row.rating,row.year,row.basis,row.content_version,row.bytes as i64,!eligible,hash(&format!("{id}:{}:0",row.ordinal))]).map_err(db_error)?;
                db.execute("UPDATE stages SET frozen=frozen+1,eligible=eligible+?2 WHERE id=?1", params![id,eligible as u32]).map_err(db_error)?;
                if !eligible {
                    db.execute("UPDATE candidates SET disposition='needs_review',disposition_reason='rating_unresolved' WHERE stage_id=?1 AND ordinal=?2", params![id,row.ordinal as i64]).map_err(db_error)?;
                }
            }
            Ok(())
        })
    }
    pub fn last_candidate(&self, id: &str) -> Result<Option<AestheticCandidate>> {
        let db = self.read()?;
        let mut rows = candidates(
            &db,
            "WHERE stage_id=?1 ORDER BY ordinal DESC LIMIT 1",
            rusqlite::params![id],
        )?;
        Ok(rows.pop())
    }
    pub fn finish_freeze(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.writer.submit(1024, move |db| {
            let s = read_stage(db, &id)?;
            if s.state != "preparing" {
                return Err(Error::new("CANCELLED", "候选冻结已暂停"));
            }
            if s.frozen != s.total {
                return Err(Error::new("SOURCE_CHANGED", "工作集成员与冻结计数不一致"));
            }
            db.execute(
                "UPDATE stages SET state='ready',error=NULL WHERE id=?1",
                [&id],
            )
            .map_err(db_error)?;
            Ok(())
        })
    }
    pub fn control(&self, id: &str, action: &str) -> Result<AestheticStage> {
        let id = id.to_owned();
        let action = action.to_owned();
        self.writer.submit(1024, move |db| {
            let s = read_stage(db, &id)?;
            let next = studio_application::aesthetic::control_state(&s, &action)?;
            db.execute(
                "UPDATE stages SET state=?2,error=NULL WHERE id=?1",
                params![id, next],
            )
            .map_err(db_error)?;
            read_stage(db, &id)
        })
    }
    pub fn settle(&self, id: &str, error: Option<String>) -> Result<AestheticStage> {
        let id = id.to_owned();
        self.writer.submit_named(1024, "settle", &id.clone(), move |db| {
            db.execute("UPDATE batches SET state='queued' WHERE stage_id=?1 AND state='preparing'", [&id]).map_err(db_error)?;
            db.execute("UPDATE candidates SET reserved=0,blocked=1 WHERE stage_id=?1 AND ordinal IN (SELECT json_extract(m.value,'$.candidate.ordinal') FROM batches b,json_each(b.members) m WHERE b.stage_id=?1 AND b.state='sent')",[&id]).map_err(db_error)?;
            db.execute("UPDATE attempts SET state='outcome_unknown' WHERE state='sent' AND batch IN (SELECT sequence FROM batches WHERE stage_id=?1)",[&id]).map_err(db_error)?;
            db.execute("UPDATE batches SET state='outcome_unknown',error='执行中断，上游结果尚未确认' WHERE stage_id=?1 AND state='sent'",[&id]).map_err(db_error)?;
            db.execute("UPDATE stages SET unknown=(SELECT count(*) FROM batches WHERE stage_id=?1 AND state='outcome_unknown') WHERE id=?1",[&id]).map_err(db_error)?;
            let s = read_stage(db,&id)?;
            let pending: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM batches WHERE stage_id=?1 AND state!='accepted')",[&id],|r|r.get(0)).map_err(db_error)?;
            let next = studio_application::aesthetic::settled_state(&s, pending, error.is_some());
            db.execute("UPDATE stages SET state=?2,error=COALESCE(?3,error) WHERE id=?1",params![id,next,error]).map_err(db_error)?;
            read_stage(db,&id)
        })
    }
    pub fn health_check(&self) -> Result<()> {
        self.writer.submit(1024, |db| {
            db.execute(
                "UPDATE writer_health SET probe=(probe+1)%1000000 WHERE id=1",
                [],
            )
            .map_err(db_error)?;
            Ok(())
        })
    }
    /// A consistent evidence backup, including WAL contents; destination must not exist.
    pub fn backup(&self, destination: &Path) -> Result<()> {
        if destination.exists() {
            return Err(Error::invalid("备份目标已存在"));
        }
        let source = self.read()?;
        let mut target = Connection::open(destination).map_err(db_error)?;
        let backup = rusqlite::backup::Backup::new(&source, &mut target).map_err(db_error)?;
        backup
            .run_to_completion(128, std::time::Duration::from_millis(10), None)
            .map_err(db_error)?;
        drop(backup);
        target
            .execute_batch("PRAGMA journal_mode=DELETE")
            .map_err(db_error)?;
        drop(target);
        std::fs::OpenOptions::new()
            .write(true)
            .open(destination)
            .map_err(Error::io)?
            .sync_all()
            .map_err(Error::io)
    }
}

fn read_stage(db: &Connection, id: &str) -> Result<AestheticStage> {
    db.query_row("SELECT id,name,state,created_at,config,config_hash,total,frozen,eligible,attempts,accepted,invalid,unknown,protected,input_tokens,output_tokens,usage_unknown,error,comparable,excluded,unresolved FROM stages WHERE id=?1",[id],|r| {
        let config: String = r.get(4)?;
        let config = serde_json::from_str(&config).map_err(|e| rusqlite::Error::FromSqlConversionFailure(4,rusqlite::types::Type::Text,Box::new(e)))?;
        Ok(AestheticStage { id:r.get(0)?,name:r.get(1)?,state:r.get(2)?,created_at:r.get(3)?,config,config_hash:r.get(5)?,total:crate::unsigned(r,6)?,frozen:crate::unsigned(r,7)?,eligible:crate::unsigned(r,8)?,attempts:crate::unsigned(r,9)?,accepted:crate::unsigned(r,10)?,invalid:crate::unsigned(r,11)?,unknown:crate::unsigned(r,12)?,protected:crate::unsigned(r,13)?,input_tokens:crate::unsigned(r,14)?,output_tokens:crate::unsigned(r,15)?,usage_unknown:crate::unsigned(r,16)?,error:r.get(17)?,comparable:crate::unsigned(r,18)?,excluded:crate::unsigned(r,19)?,unresolved:crate::unsigned(r,20)? })
    }).optional().map_err(db_error)?.ok_or_else(|| Error::new("NOT_FOUND","评审阶段不存在"))
}
fn candidates(
    db: &Connection,
    tail: &str,
    parameters: impl rusqlite::Params,
) -> Result<Vec<AestheticCandidate>> {
    let mut statement = db.prepare(&format!("SELECT ordinal,source_id,asset_id,rating,year,basis,content_version,bytes,exposures,protected,disposition,disposition_reason FROM candidates {tail}")).map_err(db_error)?;
    statement
        .query_map(parameters, |r| {
            Ok(AestheticCandidate {
                ordinal: crate::unsigned(r, 0)?,
                key: studio_domain::AssetKey {
                    source_id: r.get(1)?,
                    asset_id: r.get(2)?,
                },
                rating: r.get(3)?,
                year: r.get(4)?,
                basis: r.get(5)?,
                content_version: r.get(6)?,
                bytes: crate::unsigned(r, 7)?,
                exposures: r.get(8)?,
                protected: r.get(9)?,
                disposition: serde_json::from_value(serde_json::Value::String(r.get(10)?))
                    .map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            10,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?,
                disposition_reason: r.get(11)?,
            })
        })
        .map_err(db_error)?
        .collect::<std::result::Result<_, _>>()
        .map_err(db_error)
}
fn read_batch(db: &Connection, id: &str, sequence: u64) -> Result<AestheticBatch> {
    let row: (String,String,String,Option<String>,Option<String>,Option<String>) = db.query_row("SELECT rating,state,members,attempt_id,error,observation FROM batches WHERE stage_id=?1 AND sequence=?2",params![id,sequence as i64],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional().map_err(db_error)?.ok_or_else(||Error::new("NOT_FOUND","评审批次不存在"))?;
    Ok(AestheticBatch {
        sequence,
        stage_id: id.into(),
        rating: row.0,
        state: row.1,
        members: decode(row.2)?,
        attempt_id: row.3,
        error: row.4,
        observation: row.5.map(decode).transpose()?,
    })
}
