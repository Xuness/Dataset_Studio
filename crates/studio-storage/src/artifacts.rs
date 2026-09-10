use crate::*;
use studio_application::ArtifactRepository;

pub(crate) fn read(db: &Connection, pid: &str, id: &str) -> Result<Artifact> {
    validate_id(id)?;
    let (mut item, state, files, provenance) = db.query_row("SELECT id,job_id,output_id,name,kind,schema_version,status,count,created_at,files_json,provenance_json,issue FROM artifacts WHERE id=?1", [id], |r| Ok((Artifact {
        id:r.get(0)?, project_id:pid.into(), job_id:r.get(1)?, output_id:r.get(2)?,name:r.get(3)?,kind:r.get(4)?, schema_version:r.get(5)?,state:ArtifactState::Publishing,count:r.get::<_,Option<i64>>(7)?.map(|v|v as u64),created_at:r.get(8)?,files:vec![],
        provenance: ArtifactProvenance {run:None,input_scope:None,input_sha256:None,attempt:None,input_artifacts:vec![],fields_frozen:false,evidence:String::new()},issue:r.get(11)?
    },r.get::<_,String>(6)?,r.get::<_,String>(9)?,r.get::<_,String>(10)?))).optional().map_err(db_error)?.ok_or_else(|| Error::new("NOT_FOUND", "成果不属于当前项目"))?;
    item.state = serde_json::from_value(serde_json::json!(state)).map_err(Error::io)?;
    item.name = management::display_name(db, "artifact", id, &item.name)?;
    item.files = serde_json::from_str(&files).map_err(Error::io)?;
    item.provenance = serde_json::from_str(&provenance).map_err(Error::io)?;
    Ok(item)
}
pub(crate) fn require_scalar(db: &Connection, pid: &str, id: &str) -> Result<Artifact> {
    let artifact = read(db, pid, id)?;
    if artifact.state != ArtifactState::Ready
        || artifact.kind != "scalar_columns"
        || artifact.schema_version != 1
    {
        return Err(Error::new("ARTIFACT_NOT_READY", "需要已发布的标量成果版本"));
    }
    Ok(artifact)
}
impl ArtifactRepository for SqliteStore {
    fn artifacts(&self, pid: &str, after: Option<&str>, limit: usize) -> Result<Vec<Artifact>> {
        if let Some(id) = after {
            validate_id(id)?;
        }
        let p = self.handle(pid)?;
        let db = p.read()?;
        let before = after
            .map(|id| read(&db, pid, id).map(|a| a.created_at))
            .transpose()?;
        let predicate = if after.is_some() {
            "WHERE (created_at,id)<(?1,?2)"
        } else {
            ""
        };
        let mut stmt = db
            .prepare(&format!(
                "SELECT id FROM artifacts {predicate} ORDER BY created_at DESC,id DESC LIMIT ?3"
            ))
            .map_err(db_error)?;
        let ids = stmt
            .query_map(
                params![
                    before.unwrap_or_default(),
                    after.unwrap_or(""),
                    limit.clamp(1, 129) as u32
                ],
                |r| r.get::<_, String>(0),
            )
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        ids.iter().map(|id| read(&db, pid, id)).collect()
    }
    fn artifact(&self, pid: &str, id: &str) -> Result<Artifact> {
        let p = self.handle(pid)?;
        read(&*p.read()?, pid, id)
    }
    fn artifact_page(
        &self,
        pid: &str,
        id: &str,
        after: Option<&AssetKey>,
        limit: usize,
    ) -> Result<ArtifactPage> {
        let p = self.handle(pid)?;
        let db = p.read()?;
        let artifact = read(&db, pid, id)?;
        if artifact.state != ArtifactState::Ready {
            return Err(Error::new(
                "ARTIFACT_NOT_READY",
                "成果尚未完整发布、已释放或校验失败",
            ));
        }
        let limit = limit.clamp(1, 128);
        let (source, asset) = after
            .map(|k| (k.source_id.as_str(), k.asset_id.as_str()))
            .unwrap_or(("", ""));
        let mut stmt=db.prepare("SELECT row_json FROM artifact_rows WHERE artifact_id=?1 AND (source_id,asset_id)>(?2,?3) ORDER BY source_id,asset_id LIMIT ?4").map_err(db_error)?;
        let rows = stmt
            .query_map(params![id, source, asset, limit as u32 + 1], |r| {
                r.get::<_, String>(0)
            })
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        let mut items = rows
            .into_iter()
            .map(|r| serde_json::from_str::<ArtifactRow>(&r).map_err(Error::io))
            .collect::<Result<Vec<_>>>()?;
        let more = items.len() > limit;
        items.truncate(limit);
        Ok(ArtifactPage {
            next: if more {
                items.last().map(|r| r.key.clone())
            } else {
                None
            },
            items,
        })
    }
    fn artifact_scalar(&self, pid: &str, id: &str, key: &AssetKey) -> Result<ScalarValue> {
        let p = self.handle(pid)?;
        let db = p.read()?;
        require_scalar(&db, pid, id)?;
        scalar(&db, id, key)
    }
}
fn scalar(db: &Connection, id: &str, key: &AssetKey) -> Result<ScalarValue> {
    let row:Option<String>=db.query_row("SELECT row_json FROM artifact_rows WHERE artifact_id=?1 AND source_id=?2 AND asset_id=?3",params![id,key.source_id,key.asset_id],|r|r.get(0)).optional().map_err(db_error)?;
    row.map(|s| {
        serde_json::from_str::<ArtifactRow>(&s)
            .map_err(Error::io)?
            .scalar
            .ok_or_else(|| Error::new("ARTIFACT_INVALID", "标量行缺少状态"))
    })
    .unwrap_or_else(|| {
        Ok(ScalarValue::Uncomputed {
            reason: "outside_artifact_coverage".into(),
        })
    })
}
impl SqliteStore {
    /// A publishing artifact is never readable. Re-entry rebuilds its bounded index.
    pub fn begin_artifact(&self, pid: &str, item: &Artifact) -> Result<Artifact> {
        let p = self.handle(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.transaction().map_err(db_error)?;
        let existing: Option<String> = tx
            .query_row(
                "SELECT id FROM artifacts WHERE job_id=?1 AND output_id=?2",
                params![item.job_id, item.output_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?;
        let id = existing.unwrap_or_else(|| item.id.clone());
        if let Ok(old) = read(&tx, pid, &id)
            && matches!(old.state, ArtifactState::Ready | ArtifactState::Released)
        {
            return Ok(old);
        }
        tx.execute("INSERT INTO artifacts(id,job_id,output_id,name,kind,schema_version,status,count,created_at,files_json,provenance_json,issue) VALUES (?1,?2,?3,?4,?5,?6,'publishing',?7,?8,?9,?10,NULL) ON CONFLICT(id) DO UPDATE SET status='publishing',count=excluded.count,files_json=excluded.files_json,provenance_json=excluded.provenance_json,issue=NULL",params![id,item.job_id,item.output_id,item.name,item.kind,item.schema_version,item.count.map(|v|v as i64),item.created_at,serde_json::to_string(&item.files).map_err(Error::io)?,serde_json::to_string(&item.provenance).map_err(Error::io)?]).map_err(db_error)?;
        tx.execute("DELETE FROM artifact_rows WHERE artifact_id=?1", [&id])
            .map_err(db_error)?;
        for input in &item.provenance.input_artifacts {
            if read(&tx, pid, input)?.state != ArtifactState::Ready {
                return Err(Error::new("ARTIFACT_NOT_READY", "输入成果已不可用"));
            }
            tx.execute(
                "INSERT OR IGNORE INTO artifact_references VALUES ('artifact',?1,?2)",
                params![id, input],
            )
            .map_err(db_error)?;
        }
        let item = read(&tx, pid, &id)?;
        tx.commit().map_err(db_error)?;
        Ok(item)
    }
    pub fn append_artifact(&self, pid: &str, id: &str, rows: &[ArtifactRow]) -> Result<()> {
        if rows.len() > 256 {
            return Err(Error::invalid("成果批次最多 256 行"));
        }
        let p = self.handle(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.transaction().map_err(db_error)?;
        if read(&tx, pid, id)?.state != ArtifactState::Publishing {
            return Err(Error::new("ARTIFACT_NOT_WRITABLE", "成果已发布或已释放"));
        }
        for row in rows {
            let value = row
                .scalar
                .as_ref()
                .map(ScalarValue::as_integer)
                .transpose()?
                .flatten();
            let status = row
                .scalar
                .as_ref()
                .map(|v| {
                    serde_json::to_value(v).map(|v| v["status"].as_str().unwrap_or("").to_owned())
                })
                .transpose()
                .map_err(Error::io)?;
            tx.execute(
                "INSERT INTO artifact_rows VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    id,
                    row.key.source_id,
                    row.key.asset_id,
                    row.ordinal as i64,
                    value,
                    status,
                    serde_json::to_string(row).map_err(Error::io)?
                ],
            )
            .map_err(db_error)?;
        }
        tx.commit().map_err(db_error)
    }
    /// All outputs and the logical task become visible in a single transaction.
    pub fn finish_artifacts(
        &self,
        pid: &str,
        job_id: &str,
        ids: &[String],
        job_path: Option<&str>,
    ) -> Result<()> {
        let p = self.handle(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.transaction().map_err(db_error)?;
        let job = read_job(&tx, pid, job_id)?;
        if job.status == "cancelled" {
            return Err(Error::new("CANCELLED", "任务已取消，成果未发布"));
        }
        for id in ids {
            let item = read(&tx, pid, id)?;
            if item.job_id != job_id
                || !matches!(item.state, ArtifactState::Publishing | ArtifactState::Ready)
            {
                return Err(Error::new("ARTIFACT_INVALID", "成果不属于本次发布"));
            }
            let count: u64 = if item.kind == RANKING_KIND {
                tx.query_row(
                    "SELECT row_count FROM artifact_tables WHERE artifact_id=?1",
                    [id],
                    |r| unsigned(r, 0),
                )
                .map_err(db_error)?
            } else {
                tx.query_row(
                    "SELECT COUNT(*) FROM artifact_rows WHERE artifact_id=?1",
                    [id],
                    |r| unsigned(r, 0),
                )
                .map_err(db_error)?
            };
            if item.count != Some(count) {
                return Err(Error::new("ARTIFACT_INVALID", "成果索引行数不完整"));
            }
            tx.execute(
                "UPDATE artifacts SET status='ready',issue=NULL WHERE id=?1",
                [id],
            )
            .map_err(db_error)?;
            event(&tx, "artifact.changed", id)?;
        }
        if let Some(path) = job_path {
            tx.execute("UPDATE jobs SET status='succeeded',completed=total,error=NULL,artifact=?2 WHERE id=?1",params![job_id,path]).map_err(db_error)?;
            job_telemetry::finish(&tx, job_id)?;
            event(&tx, "job.changed", job_id)?;
        }
        // Lazy legacy registration leaves pre-existing project revisions/events intact.
        tx.commit().map_err(db_error)
    }
    pub fn artifact_unavailable(&self, pid: &str, id: &str, message: &str) -> Result<()> {
        let p = self.handle(pid)?;
        p.db.lock().map_err(lock_error)?.execute("UPDATE artifacts SET status='unavailable',issue=?2 WHERE id=?1 AND status!='released'",params![id,message]).map_err(db_error)?;
        Ok(())
    }
    pub fn release_artifact(&self, pid: &str, id: &str) -> Result<Artifact> {
        let p = self.handle(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.transaction().map_err(db_error)?;
        let item = read(&tx, pid, id)?;
        if item.state == ArtifactState::Publishing {
            return Err(Error::new("ARTIFACT_NOT_READY", "发布中的成果不能释放"));
        }
        let count: u64 = tx
            .query_row(
                "SELECT COUNT(*) FROM artifact_references WHERE artifact_id=?1",
                [id],
                |r| unsigned(r, 0),
            )
            .map_err(db_error)?;
        if count > 0 {
            return Err(Error::new(
                "ARTIFACT_IN_USE",
                format!("成果仍被 {count} 个工作集、任务、查询或成果引用"),
            ));
        }
        tx.execute("UPDATE artifacts SET status='released' WHERE id=?1", [id])
            .map_err(db_error)?;
        tx.execute("DELETE FROM artifact_rows WHERE artifact_id=?1", [id])
            .map_err(db_error)?;
        tx.execute("DELETE FROM artifact_tables WHERE artifact_id=?1", [id])
            .map_err(db_error)?;
        tx.execute(
            "DELETE FROM artifact_references WHERE owner_kind='artifact' AND owner_id=?1",
            [id],
        )
        .map_err(db_error)?;
        event(&tx, "artifact.released", id)?;
        let item = read(&tx, pid, id)?;
        tx.commit().map_err(db_error)?;
        Ok(item)
    }
}
