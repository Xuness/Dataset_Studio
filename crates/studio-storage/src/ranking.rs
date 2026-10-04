use crate::*;
use serde_json::Value;
#[cfg(test)]
#[path = "ranking_write_tests.rs"]
mod tests;

impl SqliteStore {
    pub fn ranking_snapshot_candidates(
        &self,
        pid: &str,
        jid: &str,
        versions: &[QuerySourceVersion],
        total: u64,
    ) -> Result<Vec<Artifact>> {
        let project = self.handle(pid)?;
        let db = project.read()?;
        let mut stmt=db.prepare("SELECT a.id FROM artifacts a JOIN job_runs r ON r.job_id=a.job_id JOIN job_scopes s ON s.job_id=a.job_id WHERE a.kind=?1 AND a.status='ready' AND a.job_id!=?2 AND a.count=?3 AND r.versions_json=?4 AND json_extract(s.provenance_json,'$.membership')='ranking_input' ORDER BY a.created_at DESC,a.id DESC LIMIT 16").map_err(db_error)?;
        let ids = stmt
            .query_map(
                params![
                    RANKING_KIND,
                    jid,
                    total as i64,
                    serde_json::to_string(versions).map_err(Error::io)?
                ],
                |r| r.get::<_, String>(0),
            )
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        ids.into_iter()
            .map(|id| artifacts::read(&db, pid, &id))
            .collect()
    }
    pub fn job_scope_artifacts(&self, pid: &str, jid: &str) -> Result<Vec<String>> {
        let p = self.handle(pid)?;
        let db = p.read()?;
        let mut stmt=db.prepare("SELECT artifact_id FROM artifact_references WHERE owner_kind='job_scope' AND owner_id=?1 ORDER BY artifact_id").map_err(db_error)?;
        stmt.query_map([jid], |r| r.get(0))
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)
    }
    pub fn ranking_job_artifact(&self, pid: &str, jid: &str) -> Result<Artifact> {
        let p = self.handle(pid)?;
        let db = p.read()?;
        read_job(&db, pid, jid)?;
        let id:String=db.query_row("SELECT id FROM artifacts WHERE job_id=?1 AND kind=?2 AND output_id='data' AND status='ready'",params![jid,RANKING_KIND],|r|r.get(0)).optional().map_err(db_error)?.ok_or_else(||Error::new("ARTIFACT_NOT_READY","排名任务尚未发布成果"))?;
        artifacts::read(&db, pid, &id)
    }
    pub fn job_stage(&self, pid: &str, jid: &str, stage: &JobStage) -> Result<()> {
        let p = self.handle(pid)?;
        let db = p.db.lock().map_err(lock_error)?;
        job_telemetry::record(&db, jid, stage)
    }
    pub fn ranking_job_bases(&self, pid: &str, jid: &str) -> Result<Vec<RankingBasis>> {
        let p = self.handle(pid)?;
        let db = p.read()?;
        read_job(&db, pid, jid)?;
        let raw: String = db
            .query_row(
                "SELECT provenance_json FROM job_scopes WHERE job_id=?1",
                [jid],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        let value: Value = serde_json::from_str(&raw).map_err(Error::io)?;
        let mut bases = Vec::new();
        for item in value["queries"].as_array().into_iter().flatten() {
            let query: QueryResult = serde_json::from_value(item.clone()).map_err(Error::io)?;
            let mut spec = native_spec(&query.spec);
            spec.input_scope = None;
            spec.conditions
                .retain(|c| !c.field.starts_with("stored.") && c.field != "asset.id");
            if spec.conditions.is_empty() && spec.observation_rule == ObservationRule::CurrentPost {
                continue;
            }
            bases.push(RankingBasis {
                index: bases.len() as u32 + 1,
                result_id: Some(query.id),
                spec,
            });
        }
        if bases.len() > 64 {
            return Err(Error::new(
                "RANKING_SCOPE_COMPLEX",
                "排名输入的查询依据超过 64 份，请先整理输入范围",
            ));
        }
        Ok(bases)
    }
    /// Match immutable task members to their preserved query branches in bounded batches.
    pub fn ranking_member_bases(
        &self,
        pid: &str,
        jid: &str,
        keys: &[AssetKey],
        bases: &[RankingBasis],
    ) -> Result<Vec<Vec<u32>>> {
        if keys.len() > 512 {
            return Err(Error::invalid("排名依据批次超出范围"));
        }
        let p = self.handle(pid)?;
        let db = p.read()?;
        read_job(&db, pid, jid)?;
        let mut out = vec![Vec::new(); keys.len()];
        let mut stmt=db.prepare_cached("SELECT EXISTS(SELECT 1 FROM result_members WHERE result_id=?1 AND source_id=?2 AND asset_id=?3)").map_err(db_error)?;
        for b in bases {
            let Some(id) = &b.result_id else {
                continue;
            };
            for (i, key) in keys.iter().enumerate() {
                if b.spec.source_ids.contains(&key.source_id)
                    && stmt
                        .query_row(params![id, key.source_id, key.asset_id], |r| {
                            r.get::<_, bool>(0)
                        })
                        .map_err(db_error)?
                {
                    out[i].push(b.index);
                }
            }
        }
        for indices in &mut out {
            if indices.is_empty() {
                indices.push(0);
            }
        }
        Ok(out)
    }
    pub fn register_ranking_table(
        &self,
        pid: &str,
        aid: &str,
        table_sha: &str,
        input_sha: &str,
        summary: &RankingSummary,
    ) -> Result<()> {
        let p = self.handle(pid)?;
        let db = p.db.lock().map_err(lock_error)?;
        let item = artifacts::read(&db, pid, aid)?;
        if item.kind != RANKING_KIND
            || item.state != ArtifactState::Publishing
            || item.count != Some(summary.input_count)
        {
            return Err(Error::new(
                "ARTIFACT_INVALID",
                "排名成果发布状态或数量不一致",
            ));
        }
        db.execute("INSERT INTO artifact_tables VALUES (?1,?2,?3,?4,?5) ON CONFLICT(artifact_id) DO UPDATE SET row_count=excluded.row_count,table_sha256=excluded.table_sha256,input_sha256=excluded.input_sha256,summary_json=excluded.summary_json",params![aid,summary.input_count as i64,table_sha,input_sha,serde_json::to_string(summary).map_err(Error::io)?]).map_err(db_error)?;
        Ok(())
    }
    pub fn ranking_summary(&self, pid: &str, aid: &str) -> Result<RankingSummary> {
        let p = self.handle(pid)?;
        let db = p.read()?;
        let item = artifacts::read(&db, pid, aid)?;
        if item.kind != RANKING_KIND || item.state != ArtifactState::Ready {
            return Err(Error::new(
                "ARTIFACT_NOT_READY",
                "排名成果尚未完整发布或已不可用",
            ));
        }
        let raw: String = db
            .query_row(
                "SELECT summary_json FROM artifact_tables WHERE artifact_id=?1",
                [aid],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        serde_json::from_str(&raw).map_err(Error::io)
    }
    /// A committed table result becomes a regular project workset without frontend ID enumeration.
    pub fn ranking_workset(
        &self,
        pid: &str,
        aid: &str,
        key: &str,
        name: &str,
        filter: &RankingFilter,
        paths: (&Path, &Path),
    ) -> Result<Collection> {
        let (table, input) = paths;
        validate_id(key)?;
        let name = validate_name(name)?;
        filter.validate()?;
        let request = serde_json::to_string(&(aid, &name, filter)).map_err(Error::io)?;
        let p = self.handle(pid)?;
        let operation = self.begin_member_write(pid, key)?;
        let cancelled = operation.cancelled();
        let mut db = p.write_cancelled(&cancelled)?;
        let retired: Option<String> = db
            .query_row(
                "SELECT request_json FROM retired_workset_requests WHERE request_id=?1",
                [key],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?;
        if let Some(retired) = retired {
            return Err(if retired == request {
                Error::new(
                    "OBJECT_REMOVED",
                    "此请求生成的工作集已删除，请使用新的保存请求",
                )
            } else {
                Error::new("IDEMPOTENCY_CONFLICT", "工作集请求键已用于其他条件")
            });
        }
        let old:Option<(String,String,String,u64)>=db.query_row("SELECT r.request_json,c.id,c.name,c.count FROM ranking_workset_requests r JOIN collections c ON c.id=r.collection_id WHERE r.request_id=?1",[key],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,unsigned(r,3)?))).optional().map_err(db_error)?;
        if let Some((previous, id, name, count)) = old {
            if previous != request {
                return Err(Error::new(
                    "IDEMPOTENCY_CONFLICT",
                    "工作集请求键已用于其他条件",
                ));
            }
            let name = management::display_name(&db, "workset", &id, &name)?;
            return operation.finish(Ok(Collection { id, name, count }));
        }
        let artifact = artifacts::read(&db, pid, aid)?;
        if artifact.state != ArtifactState::Ready || artifact.kind != RANKING_KIND {
            return Err(Error::new("ARTIFACT_NOT_READY", "需要完整的排名成果"));
        }
        for path in [table, input] {
            let canonical = path.canonicalize().map_err(Error::io)?;
            if !canonical.starts_with(self.directory(pid)?.join("artifacts")) {
                return Err(Error::invalid("排名文件不在本项目成果目录"));
            }
        }
        let scores = ranking_tables::RankingResultTable::open(table)?;
        scores.cancel_reads(cancelled.clone())?;
        operation.update(0, scores.known_count(filter)?);
        let result = (|| {
            let count = scores.filtered_count(filter)?;
            studio_application::read_cancelled(&cancelled)?;
            if count == 0 {
                return Err(Error::invalid("当前排名过滤结果为空"));
            }
            let tx = db.project_transaction().map_err(db_error)?;
            let id = new_id();
            let recipe = crate::ranking_memberships::create_recipe(
                &tx,
                &p.project.directory,
                aid,
                &id,
                filter,
                (input, table),
                count,
            )?;
            let versions: Option<String> = tx
                .query_row(
                    "SELECT versions_json FROM job_runs WHERE job_id=?1",
                    [&artifact.job_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(db_error)?;
            let versions = versions
                .map(|raw| serde_json::from_str::<Vec<QuerySourceVersion>>(&raw).map_err(Error::io))
                .transpose()?
                .unwrap_or_default();
            let spec = QuerySpec {
                version: 3,
                source_ids: recipe.source_ids.clone(),
                conditions: Vec::new(),
                observation_rule: ObservationRule::CurrentPost,
                order: QueryOrder::AssetKeyAsc,
                input_scope: None,
            };
            let base = crate::ranking_memberships::insert_result(
                &tx, pid, &recipe, &spec, &versions, None, true,
            )?;
            tx.execute(
                "INSERT INTO collections VALUES (?1,?2,?3)",
                params![id, name, count as i64],
            )
            .map_err(db_error)?;
            tx.execute(
                "INSERT INTO collection_bases VALUES (?1,?2)",
                params![id, base.id],
            )
            .map_err(db_error)?;
            tx.execute(
                "INSERT INTO result_references VALUES ('collection',?1,?2)",
                params![id, base.id],
            )
            .map_err(db_error)?;
            operation.update(count, Some(count));
            let provenance = serde_json::json!({"version":2,"ranking_artifact":aid,"filter":filter,"input_scope":artifact.provenance.input_scope});
            tx.execute(
                "INSERT INTO collection_scopes VALUES (?1,?2,?3)",
                params![
                    id,
                    serde_json::to_string(&artifact.provenance.input_scope).map_err(Error::io)?,
                    provenance.to_string()
                ],
            )
            .map_err(db_error)?;
            tx.execute(
                "INSERT INTO artifact_references VALUES ('collection',?1,?2)",
                params![id, aid],
            )
            .map_err(db_error)?;
            tx.execute("INSERT OR IGNORE INTO result_references SELECT 'collection',?1,result_id FROM result_references WHERE owner_kind='job' AND owner_id=?2",params![id,artifact.job_id]).map_err(db_error)?;
            tx.execute(
                "INSERT INTO ranking_workset_requests VALUES (?1,?2,?3,?4)",
                params![key, aid, request, id],
            )
            .map_err(db_error)?;
            management::created(&tx, "workset", &id)?;
            event(&tx, "collection.created", &id)?;
            studio_application::read_cancelled(&cancelled)?;
            tx.commit().map_err(db_error)?;
            Ok(Collection {
                id,
                name: name.clone(),
                count,
            })
        })();
        operation.finish(result)
    }
}
