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
pub mod analysis;
mod creation;
mod health;
mod media;
pub use creation::{create, preflight};

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
    gate: health::DispatchGate,
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
            gate: Default::default(),
        }
    }
}
async fn work<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f).await.map_err(Error::io)?
}
impl Runner {
    pub fn metrics(&self, pid: &str, directory: &std::path::Path) -> Result<AestheticMetrics> {
        let volume = health::storage_volume(directory)?;
        let (dispatch_health, storage_error_code, retained_outcomes) =
            self.gate.metrics(pid, &volume);
        Ok(AestheticMetrics {
            dispatch_health,
            storage_error_code,
            retained_outcomes,
            active_requests: self.active_requests.load(Ordering::Relaxed),
            reserved_request_bytes: self.reserved.load(Ordering::Relaxed),
            peak_request_bytes: self.peak.load(Ordering::Relaxed),
            upload_budget_bytes_per_second: UPLOAD_BYTES_PER_SECOND,
            uploaded_body_bytes: self.uploaded.load(Ordering::Relaxed),
            ..Default::default()
        })
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
        self.gate.shutdown();
        if let Ok(active) = self.active.lock() {
            for control in active.values() {
                control.reads.store(true, Ordering::Release);
                control.cancel.cancel();
            }
        }
    }
    pub fn launch(self: &Arc<Self>, state: AppState, pid: String, id: String) -> Result<()> {
        if self.gate.is_shutdown() {
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
            if let Err(error) = runner.finish(&state, &pid, &id, result).await {
                tracing::error!(%error,"evaluation finish needs recovery");
            }
            if let Ok(mut active) = runner.active.lock() {
                active.remove(&(pid, id));
            }
        });
        Ok(())
    }
    async fn finish(
        &self,
        state: &AppState,
        pid: &str,
        id: &str,
        result: Result<()>,
    ) -> Result<()> {
        let volume = health::storage_volume(&state.store.directory(pid)?)?;
        let error = result.err();
        let mut recovering = error
            .as_ref()
            .is_some_and(|e| e.code != "IO_ERROR" && health::storage_failure(e));
        if recovering && let Some(error) = &error {
            self.gate.fault(pid, &volume, None, error);
        }
        let message = error.map(|e| e.to_string());
        let mut delay = Duration::from_millis(250);
        loop {
            let copy = state.clone();
            let p = pid.to_owned();
            let sid = id.to_owned();
            let error = message.clone();
            let saved = work(move || {
                let db = copy.store.evaluation(&p)?;
                let stage = db.settle(&sid, error)?;
                copy.store.sync_evaluation(&p, &stage)?;
                if recovering {
                    db.health_check()?;
                }
                Ok(())
            })
            .await;
            match saved {
                Ok(()) => {
                    if recovering {
                        self.gate.recovered(pid, None);
                    }
                    return Ok(());
                }
                Err(error) if health::storage_failure(&error) => {
                    self.gate.fault(pid, &volume, None, &error);
                    if self.gate.is_shutdown() {
                        return Err(error);
                    }
                    if !recovering {
                        tracing::warn!(stage_id=%id,%error,"evaluation finalization retained for retry");
                    }
                    recovering = true;
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_secs(2));
                }
                Err(error) => return Err(error),
            }
        }
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
        let volume = health::storage_volume(&state.store.project(&pid)?.directory)?;
        loop {
            let ledger = db.clone();
            let sid = id.clone();
            let stage = work(move || ledger.stage(&sid)).await?;
            if stage.state != "preparing" {
                break;
            }
            if self.gate.is_shutdown() {
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
        let mut failure: Option<Error> = None;
        loop {
            if failure.is_some() {
                if let Some(error) = &failure
                    && error.code != "IO_ERROR"
                    && health::storage_failure(error)
                {
                    self.gate.fault(&pid, &volume, None, error);
                }
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
            if stage.state == "running"
                && let Err(error) = self.gate.check(&pid, &volume)
            {
                failure = Some(error);
                continue;
            }
            let can_start = stage.state == "running" && failure.is_none();
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
        let volume = health::storage_volume(&state.store.project(&pid)?.directory)?;
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
        if work(move || ledger.stage(&id)).await?.state != "running" || self.gate.is_shutdown() {
            return Ok(());
        }
        let prepared = {
            let copy = state.clone();
            let media_pid = pid.clone();
            let config = stage.clone();
            let cancel = control.reads.clone();
            work(move || {
                let messages = media::prepare_images(&copy, &media_pid, &config, &mut batch, cancel)?;
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
                use sha2::{Digest, Sha256};
                let body = serde_json::to_vec(&copy.llm.preview(&plan)?).map_err(Error::io)?;
                let size = body.len() as u64;
                let semantic_hash = hex::encode(Sha256::digest(serde_json::to_vec(&serde_json::json!({
                    "version": 1, "config_hash": config.config_hash, "batch": batch.sequence,
                    "members": batch.members, "native_body_sha256": hex::encode(Sha256::digest(&body)),
                })).map_err(Error::io)?));
                if size > config.config.max_request_bytes {
                    return Err(Error::invalid("原生请求超过 12 MiB 预算，尚未发送"));
                }
                Ok((plan, batch.members, size, semantic_hash))
            })
            .await
        };
        let (plan, members, size, semantic_hash) = match prepared {
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
        let runner = self.clone();
        let dispatch_pid = pid.clone();
        let dispatch_volume = volume.clone();
        if !work(move || {
            studio_storage::faults::check("dispatch_after_upload", &id)?;
            runner.gate.commit(&dispatch_pid, &dispatch_volume, || {
                ledger.begin_attempt(&id, sequence, members, aid, semantic_hash)
            })
        })
        .await?
        {
            return Ok(());
        }
        self.uploaded.fetch_add(size, Ordering::Relaxed);
        match state.llm.generate(plan, control.cancel.clone()).await {
            Ok(response) => {
                self.persist_outcome(
                    &db,
                    &pid,
                    &volume,
                    &sid,
                    &attempt,
                    Outcome::Receipt(AestheticReceipt::from(response)),
                )
                .await?;
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
                self.persist_outcome(
                    &db,
                    &pid,
                    &volume,
                    &sid,
                    &attempt,
                    Outcome::Failure(failure),
                )
                .await?;
                Err(error)
            }
        }
    }
    pub fn check_start(&self, state: &AppState, pid: &str) -> Result<()> {
        let db = state.store.evaluation(pid)?;
        let volume = health::storage_volume(&state.store.project(pid)?.directory)?;
        match db.health_check() {
            Ok(()) => self.gate.recovered(pid, None),
            Err(error) => {
                self.gate.fault(pid, &volume, None, &error);
                return Err(error);
            }
        }
        self.gate.check(pid, &volume)
    }
    async fn persist_outcome(
        &self,
        db: &Arc<EvaluationDb>,
        pid: &str,
        volume: &str,
        sid: &str,
        attempt: &str,
        outcome: Outcome,
    ) -> Result<()> {
        let mut recovering = false;
        let mut delay = Duration::from_millis(250);
        loop {
            let ledger = db.clone();
            let id = sid.to_owned();
            let aid = attempt.to_owned();
            let value = outcome.clone();
            let result = work(move || {
                match value {
                    Outcome::Receipt(v) => ledger.receive(&id, &aid, v)?,
                    Outcome::Failure(v) => ledger.fail_attempt(&id, &aid, v)?,
                }
                if recovering {
                    ledger.health_check()?;
                }
                Ok(())
            })
            .await;
            match result {
                Ok(()) => {
                    if recovering {
                        self.gate.recovered(pid, Some(attempt));
                    }
                    return Ok(());
                }
                Err(error) if health::storage_failure(&error) => {
                    self.gate.fault(pid, volume, Some(attempt), &error);
                    if !recovering {
                        tracing::error!(stage_id=%sid,attempt_id=%attempt,%error,"paid outcome retained; storage admission closed");
                    }
                    recovering = true;
                    if self.gate.is_shutdown() {
                        tracing::error!(stage_id=%sid,attempt_id=%attempt,%error,"shutdown with uncommitted outcome; restart must preserve outcome_unknown");
                        return Err(error);
                    }
                    // Keep the existing request's memory reservation; never retry the network.
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_secs(2));
                }
                Err(error) => return Err(error),
            }
        }
    }
}
#[derive(Clone)]
enum Outcome {
    Receipt(AestheticReceipt),
    Failure(LlmFailure),
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
