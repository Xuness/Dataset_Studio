use crate::*;

impl SqliteStore {
    pub fn retry_job(&self, pid: &str, id: &str) -> Result<Job> {
        let p = self.handle(pid)?;
        self.mark_background(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.transaction().map_err(db_error)?;
        let old = read_job(&tx, pid, id)?;
        if !matches!(old.status.as_str(), "failed" | "cancelled") || old.total == 0 {
            return Err(Error::new(
                "JOB_NOT_RETRYABLE",
                "只有已固定非空成员的失败或取消任务可以重试",
            ));
        }
        tx.execute(
            "UPDATE jobs SET status='queued',error=NULL WHERE id=?1",
            [id],
        )
        .map_err(db_error)?;
        event(&tx, "job.changed", id)?;
        let job = read_job(&tx, pid, id)?;
        tx.commit().map_err(db_error)?;
        Ok(job)
    }
    pub fn scope_source_ids(&self, pid: &str, scope: &ScopeRef) -> Result<Vec<String>> {
        scope.validate_project(pid)?;
        if let ScopeTarget::Source { source_id, .. } = &scope.target {
            self.source(pid, source_id)?;
            return Ok(vec![source_id.clone()]);
        }
        let p = self.handle(pid)?;
        let db = p.db.lock().map_err(lock_error)?;
        let resolved = scopes::resolve(&db, pid, scope)?;
        let mut stmt = db
            .prepare(&format!(
                "SELECT DISTINCT source_id FROM ({}) ORDER BY source_id",
                resolved.sql
            ))
            .map_err(db_error)?;
        stmt.query_map([], |r| r.get(0))
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)
    }
    pub fn retry_scope_job(
        &self,
        pid: &str,
        key: &str,
        scope: &ScopeRef,
        delay_ms: u64,
    ) -> Result<Option<Job>> {
        validate_id(key)?;
        scope.validate_project(pid)?;
        let request = format!(
            "manifest-scope-v1:{}:{delay_ms}",
            serde_json::to_string(scope).map_err(Error::io)?
        );
        let p = self.handle(pid)?;
        let db = p.db.lock().map_err(lock_error)?;
        let previous: Option<(String, String)> = db
            .query_row(
                "SELECT id,request_hash FROM jobs WHERE idempotency_key=?1",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(db_error)?;
        previous
            .map(|(id, old)| {
                if old != request {
                    return Err(Error::new("IDEMPOTENCY_CONFLICT", "幂等键已被不同请求使用"));
                }
                read_job(&db, pid, &id)
            })
            .transpose()
    }
    pub fn submit_scope_job(
        &self,
        pid: &str,
        key: &str,
        scope: &ScopeRef,
        delay_ms: u64,
        capture: Option<(QuerySpec, Vec<QuerySourceVersion>)>,
    ) -> Result<Job> {
        self.submit_with_run(pid, key, scope, delay_ms, capture, None)
    }
    pub fn retry_registered_job(
        &self,
        pid: &str,
        submission: &ToolSubmission,
    ) -> Result<Option<Job>> {
        self.retry_request(pid, &submission.idempotency_key, &tool_request(submission)?)
    }
    fn retry_request(&self, pid: &str, key: &str, request: &str) -> Result<Option<Job>> {
        validate_id(key)?;
        let p = self.handle(pid)?;
        let db = p.db.lock().map_err(lock_error)?;
        let previous: Option<(String, String)> = db
            .query_row(
                "SELECT id,request_hash FROM jobs WHERE idempotency_key=?1",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(db_error)?;
        previous
            .map(|(id, old)| {
                if old != request {
                    return Err(Error::new("IDEMPOTENCY_CONFLICT", "幂等键已被不同请求使用"));
                }
                read_job(&db, pid, &id)
            })
            .transpose()
    }
    pub fn submit_registered_job(
        &self,
        pid: &str,
        submission: &ToolSubmission,
        frozen: &JobRun,
        capture: Option<(QuerySpec, Vec<QuerySourceVersion>)>,
    ) -> Result<Job> {
        if submission.run != frozen.run {
            return Err(Error::invalid("提交参数与固定参数不一致"));
        }
        self.submit_with_run(
            pid,
            &submission.idempotency_key,
            &submission.scope,
            submission.delay_ms,
            capture,
            Some((tool_request(submission)?, frozen)),
        )
    }
    pub fn job_run(&self, pid: &str, id: &str) -> Result<JobRun> {
        let p = self.handle(pid)?;
        let db = p.db.lock().map_err(lock_error)?;
        read_job(&db, pid, id)?;
        let row: Option<(String, String, String)> = db
            .query_row(
                "SELECT run_json,fields_json,versions_json FROM job_runs WHERE job_id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(db_error)?;
        row.map(|(run, fields, versions)| {
            Ok(JobRun {
                run: serde_json::from_str(&run).map_err(Error::io)?,
                fields: serde_json::from_str(&fields).map_err(Error::io)?,
                source_versions: serde_json::from_str(&versions).map_err(Error::io)?,
            })
        })
        .unwrap_or_else(|| Ok(JobRun::default()))
    }
    #[allow(clippy::too_many_arguments)]
    fn submit_with_run(
        &self,
        pid: &str,
        key: &str,
        scope: &ScopeRef,
        delay_ms: u64,
        capture: Option<(QuerySpec, Vec<QuerySourceVersion>)>,
        registered: Option<(String, &JobRun)>,
    ) -> Result<Job> {
        validate_id(key)?;
        scope.validate_project(pid)?;
        let request = registered
            .as_ref()
            .map(|(hash, _)| hash.clone())
            .unwrap_or(format!(
                "manifest-scope-v1:{}:{delay_ms}",
                serde_json::to_string(scope).map_err(Error::io)?
            ));
        let p = self.handle(pid)?;
        self.mark_background(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.transaction().map_err(db_error)?;
        let previous: Option<(String, String)> = tx
            .query_row(
                "SELECT id,request_hash FROM jobs WHERE idempotency_key=?1",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(db_error)?;
        if let Some((id, old)) = previous {
            if old != request {
                return Err(Error::new("IDEMPOTENCY_CONFLICT", "幂等键已被不同请求使用"));
            }
            return read_job(&tx, pid, &id);
        }
        let id = new_id();
        let (total, status, input_sql, results, provenance, owned_result) =
            if let ScopeTarget::Source {
                source_id,
                revision,
            } = &scope.target
            {
                let (spec, versions) =
                    capture.ok_or_else(|| Error::invalid("来源范围缺少构建版本"))?;
                let spec = spec.normalize()?;
                if spec.source_ids != vec![source_id.clone()]
                    || !spec.conditions.is_empty()
                    || versions.len() != 1
                    || versions[0].source_id != *source_id
                    || versions[0].catalog_revision != *revision
                {
                    return Err(Error::new("SOURCE_CHANGED", "来源范围版本不一致"));
                }
                let result = query::insert_result(&tx, pid, None, &spec, &versions)?;
                (
                    0,
                    "waiting_input",
                    None,
                    vec![result.id.clone()],
                    serde_json::json!({"version":1,"scope":scope,"queries":[result.clone()],"meaning":"fixed_asset_members; metadata_values_are_not_frozen"}),
                    Some(result.id),
                )
            } else {
                if capture.is_some() {
                    return Err(Error::invalid("已固定范围不需要来源构建"));
                }
                let resolved = scopes::resolve(&tx, pid, scope)?;
                if resolved.count == 0 {
                    return Err(Error::invalid("任务输入不能为空"));
                }
                (
                    resolved.count,
                    "queued",
                    Some(resolved.sql),
                    resolved.results,
                    resolved.provenance,
                    None,
                )
            };
        tx.execute("INSERT INTO jobs(id,operator,status,total,created_at,idempotency_key,request_hash,delay_ms) VALUES (?1,'core.manifest',?2,?3,?4,?5,?6,?7)",params![id,status,total as i64,now(),key,request,delay_ms.min(1000) as i64]).map_err(db_error)?;
        if let Some((_, frozen)) = registered {
            for field in &frozen.fields {
                if let ScalarInput::Artifact { artifact_id } = field {
                    crate::artifacts::require_scalar(&tx, pid, artifact_id)?;
                    tx.execute(
                        "INSERT INTO artifact_references VALUES ('job_input',?1,?2)",
                        params![id, artifact_id],
                    )
                    .map_err(db_error)?;
                }
            }
            tx.execute(
                "UPDATE jobs SET operator=?2 WHERE id=?1",
                params![id, frozen.run.operator_id],
            )
            .map_err(db_error)?;
            tx.execute(
                "INSERT INTO job_runs VALUES (?1,?2,?3,?4)",
                params![
                    id,
                    serde_json::to_string(&frozen.run).map_err(Error::io)?,
                    serde_json::to_string(&frozen.fields).map_err(Error::io)?,
                    serde_json::to_string(&frozen.source_versions).map_err(Error::io)?
                ],
            )
            .map_err(db_error)?;
        }
        if let Some(sql) = input_sql {
            tx.execute(
                &format!("INSERT INTO job_inputs SELECT ?1,source_id,asset_id FROM ({sql})"),
                [&id],
            )
            .map_err(db_error)?;
        }
        scopes::references(&tx, "job", &id, &results)?;
        let scope_artifacts: Vec<String> = provenance["artifacts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        scopes::artifact_references(&tx, "job_scope", &id, &scope_artifacts)?;
        tx.execute(
            "INSERT INTO job_scopes VALUES (?1,?2,?3,?4)",
            params![
                id,
                serde_json::to_string(scope).map_err(Error::io)?,
                provenance.to_string(),
                owned_result
            ],
        )
        .map_err(db_error)?;
        event(&tx, "job.created", &id)?;
        let job = read_job(&tx, pid, &id)?;
        tx.commit().map_err(db_error)?;
        Ok(job)
    }
    pub fn job_owned_result(&self, pid: &str, id: &str) -> Result<Option<String>> {
        let p = self.handle(pid)?;
        let db = p.db.lock().map_err(lock_error)?;
        db.query_row(
            "SELECT result_id FROM job_scopes WHERE job_id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_error)
        .map(Option::flatten)
    }
    pub fn resolve_job_scopes(&self, pid: &str) -> Result<()> {
        let p = self.handle(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let pending = {
            let mut stmt=db.prepare("SELECT j.id,s.result_id FROM jobs j JOIN job_scopes s ON s.job_id=j.id WHERE j.status='waiting_input' ORDER BY j.created_at,j.id LIMIT 32").map_err(db_error)?;
            stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .map_err(db_error)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(db_error)?
        };
        for (id, rid) in pending {
            let tx = db.transaction().map_err(db_error)?;
            let result = query::read_result(&tx, pid, &rid)?;
            if result.state == ResultState::Ready && result.count.is_some_and(|n| n > 0) {
                tx.execute("INSERT INTO job_inputs SELECT ?1,source_id,asset_id FROM result_members WHERE result_id=?2",params![id,rid]).map_err(db_error)?;
                tx.execute(
                    "UPDATE jobs SET status='queued',total=?2 WHERE id=?1",
                    params![id, result.count.unwrap_or(0) as i64],
                )
                .map_err(db_error)?;
                event(&tx, "job.changed", &id)?;
            } else if !matches!(result.state, ResultState::Queued | ResultState::Running) {
                tx.execute(
                    "UPDATE jobs SET status='failed',error=?2 WHERE id=?1",
                    params![
                        id,
                        result
                            .error
                            .unwrap_or_else(|| "来源范围为空或未能完整固定成员".into())
                    ],
                )
                .map_err(db_error)?;
                event(&tx, "job.changed", &id)?;
            }
            tx.commit().map_err(db_error)?;
        }
        Ok(())
    }
}
fn tool_request(submission: &ToolSubmission) -> Result<String> {
    Ok(format!(
        "tool-v1:{}:{}:{}",
        serde_json::to_string(&submission.run).map_err(Error::io)?,
        serde_json::to_string(&submission.scope).map_err(Error::io)?,
        submission.delay_ms
    ))
}
