use super::*;

fn progress(session: &Session, id: &str) -> Option<VisualIndexProgress> {
    let path = session
        .config
        .materials_root
        .join("index-jobs")
        .join(id)
        .join("progress.json");
    if fs::metadata(&path).ok()?.len() > 16384 {
        return None;
    }
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

impl Backend {
    pub fn index_status(&self) -> Result<Value> {
        let session = self.ensure()?;
        let _history = session.history.read().map_err(lock)?;
        let _lease = self.resources.acquire(
            ReadRequest {
                class: ReadClass::NativeQuery,
                priority: ReadPriority::Interactive,
                bytes: METADATA_MEMORY_BYTES,
            },
            &flag(),
        )?;
        let mut items = Vec::new();
        for recipe in session.store.recipes()? {
            let version = match session.store.version(&recipe.id) {
                Ok(v) => v,
                Err(e) if e.code == "RESULT_NOT_READY" => continue,
                Err(e) => return Err(e),
            };
            let mut row = self
                .worker(&session)?
                .request("index_status", json!({"recipe":recipe,"version":version}))?;
            let last = session.store.last_index_result(&recipe.id)?;
            let trained = last.as_ref().and_then(|r| r["rows"].as_u64()).unwrap_or(0);
            let rows = row["rows"].as_u64().ok_or_else(|| lock("索引行数无效"))?;
            let tail = row["unindexed_rows"]
                .as_u64()
                .ok_or_else(|| lock("未索引行数无效"))?;
            row["recipe_id"] = json!(recipe.id);
            row["version"] = json!(version);
            row["trained_rows"] = json!(trained);
            let old_builder = last.as_ref().and_then(|r| r["build_revision"].as_u64()) != Some(2);
            row["needs_rebuild"] = json!(
                rows >= 256
                    && (row["index_type"].is_null()
                        || trained == 0
                        || old_builder
                        || (rows >= trained.saturating_mul(2)
                            && rows.saturating_sub(trained) >= 10000)
                        || tail > 10000.max(rows / 10))
            );
            items.push(row);
        }
        Ok(json!({"items":items}))
    }
    pub fn discovery_cursors(&self) -> Result<Vec<VisualDiscoveryCursor>> {
        self.ensure()?.store.discovery_cursors()
    }
    pub fn index_job(&self, id: &str) -> Result<VisualIndexJob> {
        let session = self.ensure()?;
        let mut job = session.store.index_job(id)?;
        if !matches!(job.status.as_str(), "completed" | "cancelled")
            && let Some(p) = progress(&session, id)
        {
            job.progress = p;
        }
        if let Some(active) = self
            .active
            .lock()
            .map_err(lock)?
            .as_ref()
            .filter(|a| a.id == id)
        {
            job.worker_running = active
                .worker
                .lock()
                .map_err(lock)?
                .as_ref()
                .is_some_and(|w| w.alive());
        }
        Ok(job)
    }
    pub fn index_jobs(&self, after: u64, limit: u32) -> Result<Vec<VisualIndexJob>> {
        let session = self.ensure()?;
        session
            .store
            .index_jobs(after, limit)?
            .iter()
            .map(|j| self.index_job(&j.id))
            .collect()
    }
    pub fn create_index(self: &Arc<Self>, request: VisualIndexRequest) -> Result<VisualIndexJob> {
        let _gate = self.gate.lock().map_err(lock)?;
        validate_request_id(request.request_id.as_deref())?;
        if request.partitions.is_some_and(|n| n == 0 || n > 8192)
            || !(1..=100).contains(&request.max_iterations)
            || !(32..=256).contains(&request.sample_rate)
        {
            return Err(Error::invalid(
                "索引分区 1-8192，训练迭代 1-100，采样率 32-256；训练样本最多 262144 条",
            ));
        }
        let session = self.ensure()?;
        if let Some(id) = request.request_id.as_deref() {
            match session.store.index_job(id) {
                Ok(saved) => {
                    if !same_request(&saved.request, &request)? {
                        return Err(Error::invalid("同一请求标识已用于不同的索引参数"));
                    }
                    return self.index_job(id);
                }
                Err(e) if e.code == "NOT_FOUND" => {}
                Err(e) => return Err(e),
            }
        }
        if self.busy() {
            return Err(Error::new("VISUAL_BUSY", "当前已有视觉提取或索引任务"));
        }
        if session.store.unfinished_index()?.is_some() {
            return Err(Error::new("VISUAL_BUSY", "请先恢复或取消未完成的索引任务"));
        }
        session.store.recipe(&request.recipe_id)?;
        if session.store.pending_any()?.is_some() {
            return Err(Error::new("VISUAL_BUSY", "请先恢复提取任务并确认遗留批次"));
        }
        let input = session.store.version(&request.recipe_id)?;
        let job = session.store.create_index_job(&request, input)?;
        self.launch_index(session, job.id.clone())?;
        self.index_job(&job.id)
    }
    pub fn index_action(self: &Arc<Self>, id: &str, operation: &str) -> Result<VisualIndexJob> {
        let _gate = self.gate.lock().map_err(lock)?;
        if !matches!(operation, "pause" | "resume" | "cancel") {
            return Err(Error::invalid("索引操作须为 pause、resume 或 cancel"));
        }
        let session = self.ensure()?;
        let job = session.store.index_job(id)?;
        {
            let active = self.active.lock().map_err(lock)?;
            if let Some(active) = active.as_ref() {
                if active.id != id || operation == "resume" {
                    return Err(Error::new(
                        "VISUAL_BUSY",
                        "另一项任务正在运行，或本任务尚未暂停",
                    ));
                }
                active
                    .action
                    .store(if operation == "pause" { 1 } else { 2 }, Ordering::Release);
                active.cancelled.store(true, Ordering::Release);
                session.store.index_status(
                    id,
                    if operation == "pause" {
                        "pausing"
                    } else {
                        "cancelling"
                    },
                    None,
                    None,
                )?;
                if let Some(worker) = active.worker.lock().map_err(lock)?.as_ref() {
                    worker.stop()?;
                }
            } else {
                drop(active);
                if !matches!(job.status.as_str(), "paused" | "failed") {
                    return Err(Error::invalid("只有暂停或失败的索引任务可恢复或取消"));
                }
                match operation {
                    "resume" => {
                        session.store.index_status(id, "queued", None, None)?;
                        self.launch_index(session.clone(), id.into())?;
                    }
                    "cancel" => {
                        self.release_index_pin(&session, &job)?;
                        session.store.index_status(
                            id,
                            "cancelled",
                            None,
                            progress(&session, id).as_ref(),
                        )?;
                    }
                    _ => {}
                }
            }
        }
        self.index_job(id)
    }
    fn release_index_pin(&self, session: &Session, job: &VisualIndexJob) -> Result<()> {
        self.worker(session)?.request(
            "release_index_input",
            json!({"recipe":session.store.recipe(&job.request.recipe_id)?,"job_id":job.id}),
        )?;
        Ok(())
    }
    fn launch_index(self: &Arc<Self>, session: Arc<Session>, id: String) -> Result<()> {
        let action = Arc::new(AtomicU8::new(0));
        let cancelled = flag();
        let worker_slot = Arc::new(Mutex::new(None));
        *self.active.lock().map_err(lock)? = Some(Active {
            id: id.clone(),
            action: action.clone(),
            cancelled: cancelled.clone(),
            worker: worker_slot.clone(),
        });
        let backend = self.clone();
        let failure_id = id.clone();
        let failure_session = session.clone();
        let spawned=std::thread::Builder::new().name("studio-visual-index".into()).spawn(move|| {
            let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||->Result<Option<Value>> {
                let job=session.store.index_job(&id)?;
                if action.load(Ordering::Acquire)!=0 {return Ok(None);}
                session.store.index_status(&id,"running",None,None)?;
                let _lease=backend.resources.acquire(ReadRequest{class:ReadClass::NativeQuery,priority:ReadPriority::Background,bytes:4<<30},&cancelled)?;
                let worker=process::Worker::start_index(&backend.root,&session.config)?;
                *worker_slot.lock().map_err(lock)?=Some(worker.clone());
                if action.load(Ordering::Acquire)!=0 {worker.stop()?;return Ok(None);}
                let recipe=session.store.recipe(&job.request.recipe_id)?;
                let result=worker.request("index_job",json!({"recipe":recipe,"job_id":id,"input_version":job.input_version,"partitions":job.request.partitions,"max_iterations":job.request.max_iterations,"sample_rate":job.request.sample_rate}));
                worker.stop()?;
                result.map(Some)
            })).unwrap_or_else(|_|Err(lock("索引调度异常退出，可从检查点恢复")));
            if let Ok(slot)=worker_slot.lock() && let Some(worker)=slot.as_ref() {let _=worker.stop();}
            let _finalize=backend.gate.lock().unwrap_or_else(|e|e.into_inner());
            let control=action.load(Ordering::Acquire);
            let saved=if control==0 {
                match result {
                    Ok(Some(value))=>session.store.publish_index_job(&id,&value),
                    Ok(None)=>Err(lock("索引任务未返回结果")),
                    Err(error)=>Err(error),
                }
            } else {Ok(())};
            if control!=0 {
                let state=if control==1 {"paused"} else {"cancelled"};
                let _=session.store.index_status(&id,state,None,progress(&session,&id).as_ref());
            } else if let Err(error)=saved {
                let _=session.store.index_status(&id,"failed",Some(&error.message),progress(&session,&id).as_ref());
            }
            if let Ok(job)=session.store.index_job(&id) && matches!(job.status.as_str(),"completed"|"cancelled") && let Err(error)=backend.release_index_pin(&session,&job) {
                tracing::warn!(index_job=%id,error=%error.message,"index input pin release deferred");
            }
            if let Ok(mut active)=backend.active.lock() {*active=None;}
        });
        if let Err(error) = spawned {
            *self.active.lock().map_err(lock)? = None;
            failure_session.store.index_status(
                &failure_id,
                "failed",
                Some(&error.to_string()),
                None,
            )?;
            return Err(Error::io(error));
        }
        Ok(())
    }
    pub fn retention_policy(&self) -> Result<VisualRetentionPolicy> {
        self.ensure()?.store.retention_policy()
    }
    pub fn set_retention_policy(
        &self,
        policy: VisualRetentionPolicy,
    ) -> Result<VisualRetentionPolicy> {
        if !(1..=1000).contains(&policy.keep_last_versions)
            || !(1..=87600).contains(&policy.min_age_hours)
        {
            return Err(Error::invalid(
                "保留版本数为 1-1000，最小版本年龄为 1-87600 小时",
            ));
        }
        self.ensure()?.store.set_retention_policy(&policy)?;
        Ok(policy)
    }
    pub fn cleanup(&self, apply: bool) -> Result<Value> {
        let _gate = self.gate.lock().map_err(lock)?;
        let session = self.ensure()?;
        if self.busy()
            || session.store.pending_any()?.is_some()
            || session.store.unfinished_index()?.is_some()
        {
            return Err(Error::new(
                "VISUAL_BUSY",
                "请先让视觉写入任务到达已确认的停止状态",
            ));
        }
        self.cleanup_session(&session, apply)
    }
    fn cleanup_session(&self, session: &Session, apply: bool) -> Result<Value> {
        let _history = session.history.write().map_err(lock)?;
        let policy = session.store.retention_policy()?;
        let mut items = Vec::new();
        for recipe in session.store.recipes()? {
            let version = match session.store.version(&recipe.id) {
                Ok(v) => v,
                Err(e) if e.code == "RESULT_NOT_READY" => continue,
                Err(e) => return Err(e),
            };
            let protected = session.store.protected_versions(&recipe.id)?;
            let value=self.worker(session)?.request("retention",json!({"recipe":recipe,"published_version":version,"protected_versions":protected,"keep_last":policy.keep_last_versions,"min_age_hours":policy.min_age_hours,"apply":apply}))?;
            items.push(value);
        }
        let value = json!({"items":items});
        if apply {
            atomic_json(
                &session.config.materials_root.join("retention-last.json"),
                &value,
            )?;
        }
        Ok(value)
    }
    pub(super) fn automatic_cleanup(&self, session: &Session) {
        let Ok(mut last) = session.last_cleanup.lock() else {
            return;
        };
        if last.elapsed() < std::time::Duration::from_secs(3600) {
            return;
        }
        *last = std::time::Instant::now();
        if session.store.retention_policy().is_ok_and(|p| p.automatic)
            && let Err(error) = self.cleanup_session(session, true)
        {
            tracing::warn!(error=%error.message,"visual history cleanup deferred; published features retained");
        }
    }
}
