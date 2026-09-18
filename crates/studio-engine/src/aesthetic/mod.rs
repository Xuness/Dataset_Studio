use crate::api::AppState;
use futures::{FutureExt, StreamExt, stream::FuturesUnordered};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use studio_application::{
    ProjectRepository, QueryAdapter, aesthetic::AestheticRepository, llm::LlmCancellation,
};
use studio_domain::{Error, Result, aesthetic::*, llm::*};
use studio_storage::aesthetic::EvaluationDb;
use tokio::sync::Semaphore;
mod media;

const REQUEST_KIB: u32 = 104 * 1024; // Native JSON copies, encoded inputs, and a 32 MiB receipt reserve.
const UPLOAD_BYTES_PER_SECOND: u64 = 3_500_000;
#[derive(Clone)]
struct Control {
    cancel: LlmCancellation,
    reads: Arc<AtomicBool>,
}
pub struct Runner {
    active: Mutex<HashMap<(String, String), Control>>,
    requests: Arc<Semaphore>,
    bytes: Arc<Semaphore>,
    upload: tokio::sync::Mutex<Instant>,
    active_requests: AtomicU64,
    reserved: AtomicU64,
    peak: AtomicU64,
    uploaded: AtomicU64,
    stopped: AtomicBool,
}
impl Default for Runner {
    fn default() -> Self {
        Self {
            active: Default::default(),
            requests: Arc::new(Semaphore::new(32)),
            bytes: Arc::new(Semaphore::new(512 * 1024)),
            upload: tokio::sync::Mutex::new(Instant::now()),
            active_requests: AtomicU64::new(0),
            reserved: AtomicU64::new(0),
            peak: AtomicU64::new(0),
            uploaded: AtomicU64::new(0),
            stopped: AtomicBool::new(false),
        }
    }
}
async fn work<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f).await.map_err(Error::io)?
}
impl Runner {
    pub fn metrics(&self) -> AestheticMetrics {
        AestheticMetrics {
            active_requests: self.active_requests.load(Ordering::Relaxed),
            reserved_request_bytes: self.reserved.load(Ordering::Relaxed),
            peak_request_bytes: self.peak.load(Ordering::Relaxed),
            upload_budget_bytes_per_second: UPLOAD_BYTES_PER_SECOND,
            uploaded_body_bytes: self.uploaded.load(Ordering::Relaxed),
            ..Default::default()
        }
    }
    pub fn busy(&self) -> bool {
        self.active.lock().is_ok_and(|m| !m.is_empty())
    }
    pub fn cancel(&self, pid: &str, id: &str) {
        if let Ok(active) = self.active.lock()
            && let Some(control) = active.get(&(pid.into(), id.into()))
        {
            control.reads.store(true, Ordering::Release);
            control.cancel.cancel();
        }
    }
    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Ok(active) = self.active.lock() {
            for control in active.values() {
                control.reads.store(true, Ordering::Release);
                control.cancel.cancel();
            }
        }
    }
    pub fn launch(self: &Arc<Self>, state: AppState, pid: String, id: String) -> Result<()> {
        if self.stopped.load(Ordering::Acquire) {
            return Err(Error::new("EVALUATION_BUSY", "评审执行器正在停止"));
        }
        let lease = state.store.operation_lease(&pid)?;
        let control = Control {
            cancel: Default::default(),
            reads: Arc::new(AtomicBool::new(false)),
        };
        {
            let mut active = self
                .active
                .lock()
                .map_err(|_| Error::io("评审执行器锁不可用"))?;
            if active.contains_key(&(pid.clone(), id.clone())) {
                return Err(Error::new("REVISION_CONFLICT", "阶段已在执行"));
            }
            if active.len() >= 8 {
                return Err(Error::new("EVALUATION_BUSY", "最多同时执行 8 个评审阶段"));
            }
            active.insert((pid.clone(), id.clone()), control.clone());
        }
        let runner = self.clone();
        tokio::spawn(async move {
            let _lease = lease;
            let result = std::panic::AssertUnwindSafe(runner.run(
                state.clone(),
                pid.clone(),
                id.clone(),
                control,
            ))
            .catch_unwind()
            .await
            .unwrap_or_else(|_| {
                Err(Error::new(
                    "INTERNAL_ERROR",
                    "评审执行发生异常，请核对未完成请求",
                ))
            });
            let finish_state = state.clone();
            let finish_pid = pid.clone();
            let finish_id = id.clone();
            if let Err(error) = work(move || {
                let db = finish_state.store.evaluation(&finish_pid)?;
                let stage = db.settle(&finish_id, result.err().map(|e| e.to_string()))?;
                finish_state.store.sync_evaluation(&finish_pid, &stage)
            })
            .await
            {
                tracing::error!(%error,"evaluation finish needs recovery");
            }
            if let Ok(mut active) = runner.active.lock() {
                active.remove(&(pid, id));
            }
        });
        Ok(())
    }
    async fn run(
        self: &Arc<Self>,
        state: AppState,
        pid: String,
        id: String,
        control: Control,
    ) -> Result<()> {
        let store = state.store.clone();
        let p = pid.clone();
        let db = work(move || store.evaluation(&p)).await?;
        loop {
            let ledger = db.clone();
            let sid = id.clone();
            let stage = work(move || ledger.stage(&sid)).await?;
            if stage.state != "preparing" {
                break;
            }
            if self.stopped.load(Ordering::Acquire) {
                return Err(Error::new("CANCELLED", "引擎正在关闭"));
            }
            let copy = state.clone();
            let p = pid.clone();
            let ledger = db.clone();
            let cancel = control.reads.clone();
            let more = work(move || media::freeze_page(&copy, &p, &stage, &ledger, cancel)).await?;
            if !more {
                return Ok(());
            } // Creating/freezing a stage never purchases an API call.
        }
        let ledger = db.clone();
        let sid = id.clone();
        work(move || ledger.parse_received(&sid)).await?;
        let mut running = FuturesUnordered::new();
        let mut failure = None;
        loop {
            if failure.is_some() {
                if let Some(result) = running.next().await {
                    if let Err(e) = result {
                        tracing::warn!(%e,"evaluation drain failed");
                    }
                    continue;
                }
                break;
            }
            let ledger = db.clone();
            let sid = id.clone();
            let stage = match work(move || ledger.stage(&sid)).await {
                Ok(stage) => stage,
                Err(error) => {
                    failure = Some(error);
                    continue;
                }
            };
            let can_start = stage.state == "running"
                && !self.stopped.load(Ordering::Acquire)
                && failure.is_none();
            if can_start && running.len() < stage.config.request.concurrency as usize {
                let ledger = db.clone();
                let sid = id.clone();
                let claim = match work(move || ledger.claim(&sid)).await {
                    Ok(batch) => batch,
                    Err(error) => {
                        failure = Some(error);
                        continue;
                    }
                };
                if let Some(batch) = claim {
                    running.push(self.clone().evaluate(
                        state.clone(),
                        pid.clone(),
                        stage,
                        batch,
                        db.clone(),
                        control.clone(),
                    ));
                    continue;
                }
            }
            let Some(result) = running.next().await else {
                break;
            };
            if let Err(error) = result {
                failure = Some(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }
    async fn evaluate(
        self: Arc<Self>,
        state: AppState,
        pid: String,
        stage: AestheticStage,
        mut batch: AestheticBatch,
        db: Arc<EvaluationDb>,
        control: Control,
    ) -> Result<()> {
        let admitted = async {
            let request = self
                .requests
                .clone()
                .acquire_owned()
                .await
                .map_err(Error::io)?;
            let bytes = self
                .bytes
                .clone()
                .acquire_many_owned(REQUEST_KIB)
                .await
                .map_err(Error::io)?;
            Ok::<_, Error>((request, bytes))
        };
        let permits = tokio::select! {r=admitted=>r?,_=control.cancel.cancelled()=>return Err(Error::new("CANCELLED","评审已取消"))};
        let reserved = self
            .reserved
            .fetch_add(u64::from(REQUEST_KIB) * 1024, Ordering::Relaxed)
            + u64::from(REQUEST_KIB) * 1024;
        self.peak.fetch_max(reserved, Ordering::Relaxed);
        self.active_requests.fetch_add(1, Ordering::Relaxed);
        let _account = Admission {
            runner: self.clone(),
            _permits: permits,
        };
        let sid = stage.id.clone();
        let sequence = batch.sequence;
        let ledger = db.clone();
        let id = sid.clone();
        if work(move || ledger.stage(&id)).await?.state != "running"
            || self.stopped.load(Ordering::Acquire)
        {
            return Ok(());
        }
        let prepared = {
            let copy = state.clone();
            let config = stage.clone();
            let cancel = control.reads.clone();
            work(move || {
                let messages = media::prepare_images(&copy, &pid, &config, &mut batch, cancel)?;
                let mut plan = copy.llm.prepare(LlmInvocationRequest {
                    invocation_id: studio_domain::new_id(),
                    model_id: config.config.model.model_id.clone(),
                    expected_model_revision: Some(config.config.model.model_revision),
                    expected_provider_revision: Some(config.config.model.provider_revision),
                    preset_id: None,
                    expected_preset_revision: None,
                    system_prompt_id: None,
                    expected_system_prompt_revision: None,
                    overrides: config.config.model.parameters.clone(),
                    messages,
                    tools: vec![],
                })?;
                // One ledger attempt is one network attempt; explicit 429 retries are visible here.
                plan.provider.config.network.rate_limit_retries = 0;
                let size = serde_json::to_vec(&copy.llm.preview(&plan)?)
                    .map_err(Error::io)?
                    .len() as u64;
                if size > config.config.max_request_bytes {
                    return Err(Error::invalid("原生请求超过 12 MiB 预算，尚未发送"));
                }
                Ok((plan, batch.members, size))
            })
            .await
        };
        let (plan, members, size) = match prepared {
            Ok(value) => value,
            Err(error) => {
                let ledger = db.clone();
                let id = sid.clone();
                let message = error.to_string();
                work(move || ledger.preparation_failed(&id, sequence, message)).await?;
                return Err(error);
            }
        };
        // Pace request admission by serialized bytes. This is not a TCP traffic shaper.
        let wait = {
            let mut next = self.upload.lock().await;
            let now = Instant::now();
            let start = (*next).max(now);
            *next = start + Duration::from_secs_f64(size as f64 / UPLOAD_BYTES_PER_SECOND as f64);
            start.saturating_duration_since(now)
        };
        tokio::select! {_=tokio::time::sleep(wait)=>{},_=control.cancel.cancelled()=>return Err(Error::new("CANCELLED","上传等待已取消"))}
        let attempt = plan.snapshot.invocation_id.clone();
        let ledger = db.clone();
        let id = sid.clone();
        let aid = attempt.clone();
        if !work(move || ledger.begin_attempt(&id, sequence, members, aid)).await? {
            return Ok(());
        }
        self.uploaded.fetch_add(size, Ordering::Relaxed);
        match state.llm.generate(plan, control.cancel.clone()).await {
            Ok(response) => {
                let receipt = AestheticReceipt::from(response);
                // Retain the paid result and its admission reservation until durable storage works.
                loop {
                    let ledger = db.clone();
                    let id = sid.clone();
                    let aid = attempt.clone();
                    let value = receipt.clone();
                    match work(move || ledger.receive(&id, &aid, value)).await {
                        Ok(()) => break,
                        Err(error)
                            if matches!(
                                error.code,
                                "DATABASE_ERROR" | "EVALUATION_BUSY" | "IO_ERROR"
                            ) =>
                        {
                            self.stopped.store(true, Ordering::Release);
                            tracing::error!(stage_id=%sid,attempt_id=%attempt,%error,"paid response retained; dispatch stopped");
                            tokio::time::sleep(Duration::from_secs(2)).await;
                        }
                        Err(error) => return Err(error),
                    }
                }
                let ledger = db.clone();
                let id = sid.clone();
                work(move || ledger.parse_received(&id)).await?;
                let ledger = db.clone();
                let id = sid;
                let stage = work(move || ledger.stage(&id)).await?;
                if stage.invalid > 0 {
                    return Err(Error::invalid(
                        "存在无效评审；请检查批次原始返回后决定是否重试",
                    ));
                }
                Ok(())
            }
            Err(failure) => {
                let error = Error::new("EVALUATION_REMOTE", failure.message.clone());
                let ledger = db.clone();
                work(move || ledger.fail_attempt(&sid, &attempt, failure)).await?;
                Err(error)
            }
        }
    }
}
struct Admission {
    runner: Arc<Runner>,
    _permits: (
        tokio::sync::OwnedSemaphorePermit,
        tokio::sync::OwnedSemaphorePermit,
    ),
}
impl Drop for Admission {
    fn drop(&mut self) {
        self.runner
            .reserved
            .fetch_sub(u64::from(REQUEST_KIB) * 1024, Ordering::Relaxed);
        self.runner.active_requests.fetch_sub(1, Ordering::Relaxed);
    }
}

pub fn create(state: &AppState, pid: &str, request: AestheticCreate) -> Result<AestheticStage> {
    studio_application::aesthetic::validate_create(&request)?;
    let db = state.store.evaluation(pid)?;
    match db.stage(&request.idempotency_key) {
        Ok(stage) => {
            if serde_json::to_value(&stage.config.request).map_err(Error::io)?
                != serde_json::to_value(&request).map_err(Error::io)?
            {
                return Err(Error::new(
                    "IDEMPOTENCY_CONFLICT",
                    "评审创建键已被不同请求使用",
                ));
            }
            return Ok(stage);
        }
        Err(e) if e.code == "NOT_FOUND" => {}
        Err(e) => return Err(e),
    }
    let plan = state.llm.prepare(LlmInvocationRequest {
        invocation_id: request.idempotency_key.clone(),
        model_id: request.model_id.clone(),
        expected_model_revision: None,
        expected_provider_revision: None,
        preset_id: None,
        expected_preset_revision: None,
        system_prompt_id: Some(request.system_prompt_id.clone()),
        expected_system_prompt_revision: None,
        overrides: request.overrides.clone(),
        messages: vec![LlmMessage {
            role: LlmRole::User,
            content: vec![LlmContent::Text {
                text: studio_application::aesthetic::OUTPUT_INSTRUCTIONS.into(),
            }],
        }],
        tools: vec![],
    })?;
    let scope = studio_domain::ScopeRef {
        project_id: pid.into(),
        target: studio_domain::ScopeTarget::Workset {
            collection_id: request.collection_id.clone(),
        },
    };
    let mut sources = Vec::new();
    for id in state.store.scope_source_ids(pid, &scope)? {
        let source = state.store.source(pid, &id)?;
        let spec = studio_domain::QuerySpec {
            version: 1,
            source_ids: vec![id],
            conditions: if source.kind == "danbooru" {
                vec![studio_domain::QueryCondition {
                    field: "rating".into(),
                    operator: studio_domain::QueryOperator::IsPresent,
                    value: None,
                }]
            } else {
                vec![]
            },
            observation_rule: studio_domain::ObservationRule::AnyObservation,
            order: studio_domain::QueryOrder::AssetKeyAsc,
            input_scope: None,
        };
        sources.push(studio_sources::QueryReader::default().query_version(&source, &spec)?);
    }
    let total = state.store.register_evaluation(pid, &request)?;
    db.create(
        AestheticConfig {
            version: 1,
            request,
            model: plan.snapshot,
            sources,
            image_policy: "stored_original_v1".into(),
            grouping_policy: "origin_rating_agreement_min_post_created_year_v1".into(),
            observation_policy: "meaningful_indifference_v1".into(),
            max_image_bytes: 2 << 20,
            max_request_bytes: 12 << 20,
        },
        total,
    )
}
