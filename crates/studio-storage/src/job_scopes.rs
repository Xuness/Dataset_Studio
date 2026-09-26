use crate::*;
use studio_application::QueryRepository;

impl SqliteStore {
    pub fn retry_job(&self, pid: &str, id: &str) -> Result<Job> {
        let p = self.handle(pid)?;
        self.mark_background(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.project_transaction().map_err(db_error)?;
        if management::removed(&tx, "job", id)? {
            return Err(Error::new(
                "JOB_NOT_RETRYABLE",
                "任务记录已清理，不能重试已释放的输入",
            ));
        }
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
        tx.execute("DELETE FROM job_progress WHERE job_id=?1", [id])
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
        if let ScopeTarget::QueryResult { result_id } = &scope.target {
            let result = self.query_result(pid, result_id)?;
            if result.cache.mode == "view" {
                return Ok(result.spec.source_ids);
            }
        }
        let p = self.handle(pid)?;
        let db = p.read()?;
        let resolved = scopes::resolve(&db, pid, scope)?;
        if resolved.count == 0 {
            return Ok(Vec::new());
        }
        // Project sources are small; probe each source's first matching member.
        // DISTINCT over members would revisit every image on every browse poll.
        let predicate = if matches!(scope.target, ScopeTarget::Selection { .. }) {
            "EXISTS(SELECT 1 FROM selection WHERE source_id=s.id) OR EXISTS(SELECT 1 FROM result_members m WHERE m.result_id=(SELECT result_id FROM selection_base WHERE singleton=1) AND m.source_id=s.id AND NOT EXISTS(SELECT 1 FROM selection_exclusions e WHERE e.source_id=m.source_id AND e.asset_id=m.asset_id))".to_owned()
        } else {
            format!(
                "EXISTS(SELECT 1 FROM ({}) m WHERE m.source_id=s.id)",
                resolved.sql
            )
        };
        let mut stmt = db
            .prepare(&format!(
                "SELECT s.id FROM sources s WHERE {predicate} ORDER BY s.id"
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
        let db = p.read()?;
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
                if management::removed(&db, "job", &id)? {
                    return Err(Error::new(
                        "OBJECT_REMOVED",
                        "此请求生成的任务已清理，请使用新的提交请求",
                    ));
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
        let db = p.read()?;
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
                if management::removed(&db, "job", &id)? {
                    return Err(Error::new(
                        "OBJECT_REMOVED",
                        "此请求生成的任务已清理，请使用新的提交请求",
                    ));
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
        let db = p.read()?;
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
        let operation = self.begin_member_write(pid, key)?;
        let cancelled = operation.cancelled();
        let mut db = p.write_cancelled(&cancelled)?;
        let flag = cancelled.clone();
        db.progress_handler(1000, Some(move || flag.load(Ordering::Acquire)))
            .map_err(db_error)?;
        let outcome = (|| {
            let tx = db.project_transaction().map_err(db_error)?;
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
                if management::removed(&tx, "job", &id)? {
                    return Err(Error::new(
                        "OBJECT_REMOVED",
                        "此请求生成的任务已清理，请使用新的提交请求",
                    ));
                }
                return read_job(&tx, pid, &id);
            }
            let id = new_id();
            let (total, status, input_sql, results, provenance, owned_result) = if let Some((
                spec,
                versions,
            )) = capture
            {
                let spec = spec.normalize()?;
                match &scope.target {
                    ScopeTarget::Source {
                        source_id,
                        revision,
                    } => {
                        if spec.source_ids != vec![source_id.clone()]
                            || !spec.conditions.is_empty()
                            || versions.len() != 1
                            || versions[0].source_id != *source_id
                            || versions[0].catalog_revision != *revision
                        {
                            return Err(Error::new("SOURCE_CHANGED", "来源范围版本不一致"));
                        }
                    }
                    ScopeTarget::QueryResult { result_id } => {
                        let view = query::ready_result(&tx, pid, result_id)?;
                        if view.cache.mode != "view"
                            || view.spec != spec
                            || view.source_versions != versions
                        {
                            return Err(Error::invalid("浏览视图与捕获版本不一致"));
                        }
                    }
                    _ => return Err(Error::invalid("已固定范围不需要来源构建")),
                }
                let result = query::insert_result(&tx, pid, None, &spec, &versions)?;
                tx.execute(
                    "UPDATE query_results SET internal=1 WHERE id=?1",
                    [&result.id],
                )
                .map_err(db_error)?;
                if versions
                    .iter()
                    .any(|v| v.consistency == "retained_online_snapshot")
                {
                    tx.execute(
                        "UPDATE query_results SET storage_kind='sealed' WHERE id=?1",
                        [&result.id],
                    )
                    .map_err(db_error)?;
                    tx.execute(
                        "UPDATE query_families SET fixed=1 WHERE id=?1",
                        [&result.id],
                    )
                    .map_err(db_error)?;
                }
                (
                    0,
                    "waiting_input",
                    None,
                    vec![result.id.clone()],
                    serde_json::json!({"version":1,"scope":scope,"queries":[result.clone()],"meaning":"fixed_asset_members; metadata_values_are_not_frozen"}),
                    Some(result.id),
                )
            } else {
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
            if let Some(sql) = &input_sql {
                let detached: bool = tx.query_row(&format!("SELECT EXISTS(SELECT 1 FROM object_metadata s WHERE s.kind='source' AND s.deleted=1 AND EXISTS(SELECT 1 FROM ({sql}) i WHERE i.source_id=s.id))"),[],|r|r.get(0)).map_err(db_error)?;
                if detached {
                    return Err(Error::new(
                        "SOURCE_DETACHED",
                        "任务输入中的数据湖已取消关联，请先重新关联",
                    ));
                }
            }
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
                let base = match &scope.target {
                    ScopeTarget::QueryResult { result_id } => Some(result_id.clone()),
                    ScopeTarget::Workset { collection_id } => tx
                        .query_row(
                            "SELECT result_id FROM collection_bases WHERE collection_id=?1",
                            [collection_id],
                            |r| r.get::<_, String>(0),
                        )
                        .optional()
                        .map_err(db_error)?,
                    ScopeTarget::Selection { .. } => tx
                        .query_row(
                            "SELECT result_id FROM selection_base WHERE singleton=1",
                            [],
                            |r| r.get::<_, String>(0),
                        )
                        .optional()
                        .map_err(db_error)?,
                    _ => None,
                };
                if let Some(base) = &base {
                    tx.execute(
                        "INSERT INTO job_input_bases VALUES(?1,?2)",
                        params![id, base],
                    )
                    .map_err(db_error)?;
                    match &scope.target {
                        ScopeTarget::Workset { collection_id } => {
                            tx.execute("INSERT INTO job_input_legacy SELECT ?1,source_id,asset_id FROM collection_inclusions WHERE collection_id=?2",params![id,collection_id]).map_err(db_error)?;
                            tx.execute("INSERT INTO job_input_exclusions SELECT ?1,source_id,asset_id FROM collection_exclusions WHERE collection_id=?2",params![id,collection_id]).map_err(db_error)?;
                        }
                        ScopeTarget::Selection { .. } => {
                            tx.execute("INSERT INTO job_input_legacy SELECT ?1,source_id,asset_id FROM selection",[&id]).map_err(db_error)?;
                            tx.execute("INSERT INTO job_input_exclusions SELECT ?1,source_id,asset_id FROM selection_exclusions",[&id]).map_err(db_error)?;
                        }
                        _ => {}
                    }
                }
                operation.update(0, Some(total));
                let mut copied = if base.is_some() { total } else { 0 };
                let mut after = (String::new(), String::new());
                loop {
                    if base.is_some() {
                        break;
                    }
                    studio_application::read_cancelled(&cancelled)?;
                    let last = {
                        let mut stmt = tx.prepare(&format!("SELECT source_id,asset_id FROM ({sql}) WHERE (source_id,asset_id)>(?1,?2) ORDER BY source_id,asset_id LIMIT 32768")).map_err(db_error)?;
                        stmt.query_map(params![after.0, after.1], |r| {
                            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                        })
                        .map_err(db_error)?
                        .last()
                        .transpose()
                        .map_err(db_error)?
                    };
                    let Some(last) = last else { break };
                    copied += tx.execute(&format!("INSERT INTO job_input_legacy SELECT ?1,source_id,asset_id FROM ({sql}) WHERE (source_id,asset_id)>(?2,?3) AND (source_id,asset_id)<=(?4,?5)"),params![id,after.0,after.1,last.0,last.1]).map_err(db_error)? as u64;
                    after = last;
                    operation.update(copied, Some(total));
                }
                if copied != total {
                    return Err(Error::new("SOURCE_CHANGED", "任务输入数量已变化"));
                }
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
            studio_application::read_cancelled(&cancelled)?;
            tx.commit().map_err(db_error)?;
            Ok(job)
        })();
        let cleared = db
            .progress_handler(0, None::<fn() -> bool>)
            .map_err(db_error);
        operation.finish(outcome.and_then(|job| {
            cleared?;
            Ok(job)
        }))
    }
    pub fn job_owned_result(&self, pid: &str, id: &str) -> Result<Option<String>> {
        let p = self.handle(pid)?;
        let db = p.read()?;
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
        let pending = {
            let db = p.read()?;
            let mut stmt=db.prepare("SELECT j.id,s.result_id FROM jobs j JOIN job_scopes s ON s.job_id=j.id WHERE j.status='waiting_input' ORDER BY j.created_at,j.id LIMIT 32").map_err(db_error)?;
            stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .map_err(db_error)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(db_error)?
        };
        for (id, rid) in pending {
            let result = self.query_result(pid, &rid)?;
            if result.state == ResultState::Ready && result.count.is_some_and(|n| n > 0) {
                let operation = match self.begin_member_write(pid, &id) {
                    Ok(operation) => operation,
                    Err(error) if error.code == "CANCELLED" => continue,
                    Err(error) => return Err(error),
                };
                let cancelled = operation.cancelled();
                let mut db = p.write_cancelled(&cancelled)?;
                let flag = cancelled.clone();
                db.progress_handler(1000, Some(move || flag.load(Ordering::Acquire)))
                    .map_err(db_error)?;
                let outcome = (|| {
                    let tx = db.project_transaction().map_err(db_error)?;
                    let state: String = tx
                        .query_row("SELECT status FROM jobs WHERE id=?1", [&id], |r| r.get(0))
                        .map_err(db_error)?;
                    if state != "waiting_input" {
                        return Ok(());
                    }
                    let result = query::ready_result(&tx, pid, &rid)?;
                    let total = result.count.unwrap_or(0);
                    let mut copied = 0;
                    let mut after = (String::new(), String::new());
                    operation.update(0, Some(total));
                    let sealed: bool = tx
                        .query_row(
                            "SELECT storage_kind='sealed' FROM query_results WHERE id=?1",
                            [&rid],
                            |r| r.get(0),
                        )
                        .map_err(db_error)?;
                    if sealed {
                        tx.execute(
                            "INSERT OR REPLACE INTO job_input_bases VALUES(?1,?2)",
                            params![id, rid],
                        )
                        .map_err(db_error)?;
                        copied = total;
                    }
                    loop {
                        if sealed {
                            break;
                        }
                        studio_application::read_cancelled(&cancelled)?;
                        let last = {
                            let mut stmt=tx.prepare("SELECT source_id,asset_id FROM result_members WHERE result_id=?1 AND (source_id,asset_id)>(?2,?3) ORDER BY source_id,asset_id LIMIT 32768").map_err(db_error)?;
                            stmt.query_map(params![rid, after.0, after.1], |r| {
                                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                            })
                            .map_err(db_error)?
                            .last()
                            .transpose()
                            .map_err(db_error)?
                        };
                        let Some(last) = last else { break };
                        copied+=tx.execute("INSERT INTO job_input_legacy SELECT ?1,source_id,asset_id FROM result_members WHERE result_id=?2 AND (source_id,asset_id)>(?3,?4) AND (source_id,asset_id)<=(?5,?6)",params![id,rid,after.0,after.1,last.0,last.1]).map_err(db_error)? as u64;
                        after = last;
                        operation.update(copied, Some(total));
                    }
                    if copied != total {
                        return Err(Error::new("SOURCE_CHANGED", "任务输入数量已变化"));
                    }
                    tx.execute(
                        "UPDATE jobs SET status='queued',total=?2 WHERE id=?1",
                        params![id, total as i64],
                    )
                    .map_err(db_error)?;
                    event(&tx, "job.changed", &id)?;
                    studio_application::read_cancelled(&cancelled)?;
                    tx.commit().map_err(db_error)
                })();
                let cleared = db
                    .progress_handler(0, None::<fn() -> bool>)
                    .map_err(db_error);
                if let Err(error) = operation.finish(outcome.and(cleared)) {
                    let tx = db.project_transaction().map_err(db_error)?;
                    tx.execute(
                        "UPDATE jobs SET status=?2,error=?3 WHERE id=?1 AND status='waiting_input'",
                        params![
                            id,
                            if error.code == "CANCELLED" {
                                "cancelled"
                            } else {
                                "failed"
                            },
                            error.to_string()
                        ],
                    )
                    .map_err(db_error)?;
                    job_telemetry::finish(&tx, &id)?;
                    event(&tx, "job.changed", &id)?;
                    tx.commit().map_err(db_error)?;
                }
            } else {
                let mut db = match p.db.try_lock() {
                    Ok(db) => db,
                    Err(std::sync::TryLockError::WouldBlock) => continue,
                    Err(error) => return Err(Error::new("INTERNAL_ERROR", error.to_string())),
                };
                let tx = db.project_transaction().map_err(db_error)?;
                let result = query::read_result(&tx, pid, &rid)?;
                job_telemetry::waiting(&tx, &id, result.processed)?;
                if !matches!(
                    result.state,
                    ResultState::Queued | ResultState::Running | ResultState::Ready
                ) || result.count == Some(0) && result.state == ResultState::Ready
                {
                    tx.execute("UPDATE jobs SET status='failed',error=?2 WHERE id=?1 AND status='waiting_input'",params![id,result.error.unwrap_or_else(||"来源范围为空或未能完整固定成员".into())]).map_err(db_error)?;
                    job_telemetry::finish(&tx, &id)?;
                    event(&tx, "job.changed", &id)?;
                }
                tx.commit().map_err(db_error)?;
            }
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
