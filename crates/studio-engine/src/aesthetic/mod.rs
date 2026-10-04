use crate::api::AppState;
use futures::{FutureExt, StreamExt, stream::FuturesUnordered};
use std::{
    collections::HashMap,
    path::PathBuf,
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
mod execution;
mod health;
mod media;
mod receipts;
mod sampling;
pub use creation::{create, preflight};
pub use execution::{check_execution, configure_execution};
pub use receipts::reparse_batch;

fn request_reservation_kib(stage: &AestheticStage, batch: &AestheticBatch) -> Result<u32> {
    let images = batch
        .members
        .iter()
        .filter(|m| m.candidate.bytes <= stage.config.max_image_bytes)
        .map(|m| m.candidate.bytes.div_ceil(3) * 4)
        .sum::<u64>();
    let configuration = serde_json::to_vec(&stage.config).map_err(Error::io)?.len() as u64;
    // Admit preparation too: an oversized parent is encoded before it can be split.
    let bytes = images
        .saturating_add(configuration)
        .saturating_add(65536)
        .saturating_mul(6)
        .saturating_add(64 << 20);
    if bytes > 512 << 20 {
        return Err(Error::new(
            "EVALUATION_CAPACITY_EXCEEDED",
            "此批准备内存超过共享预算，尚未发送",
        ));
    }
    Ok(bytes.div_ceil(1024) as u32)
}
const UPLOAD_BYTES_PER_SECOND: u64 = 3_500_000;
#[derive(Clone)]
struct Control {
    cancel: LlmCancellation,
    reads: Arc<AtomicBool>,
    stop_dispatch: Arc<AtomicBool>,
}
pub struct Runner {
    active: Mutex<HashMap<(String, String), Control>>,
    requests: Arc<Semaphore>,
    bytes: Arc<Semaphore>,
    planners: Arc<Semaphore>,
    upload: tokio::sync::Mutex<Instant>,
    active_requests: AtomicU64,
    reserved: AtomicU64,
    peak: AtomicU64,
    uploaded: AtomicU64,
    gate: health::DispatchGate,
    transfers: Mutex<HashMap<(String, String, u64), AestheticTransfer>>,
}
impl Default for Runner {
    fn default() -> Self {
        Self {
            active: Default::default(),
            requests: Arc::new(Semaphore::new(32)),
            bytes: Arc::new(Semaphore::new(512 * 1024)),
            planners: Arc::new(Semaphore::new(1)),
            upload: tokio::sync::Mutex::new(Instant::now()),
            active_requests: AtomicU64::new(0),
            reserved: AtomicU64::new(0),
            peak: AtomicU64::new(0),
            uploaded: AtomicU64::new(0),
            gate: Default::default(),
            transfers: Default::default(),
        }
    }
}
async fn work<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f).await.map_err(Error::io)?
}
impl Runner {
    pub fn transfer(&self, pid: &str, stage: &str, sequence: u64) -> Option<AestheticTransfer> {
        self.transfers
            .lock()
            .ok()?
            .get(&(pid.into(), stage.into(), sequence))
            .cloned()
    }
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
            control.stop_dispatch.store(true, Ordering::Release);
            control.cancel.cancel();
        }
    }
    pub fn pause(&self, pid: &str, id: &str) {
        if let Ok(active) = self.active.lock()
            && let Some(control) = active.get(&(pid.into(), id.into()))
        {
            control.stop_dispatch.store(true, Ordering::Release);
            control.reads.store(true, Ordering::Release);
        }
    }
    pub fn shutdown(&self) {
        self.gate.shutdown();
        if let Ok(active) = self.active.lock() {
            for control in active.values() {
                control.reads.store(true, Ordering::Release);
                control.stop_dispatch.store(true, Ordering::Release);
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
            stop_dispatch: Arc::new(AtomicBool::new(false)),
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
                control.stop_dispatch.store(true, Ordering::Release);
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
            let concurrency = stage
                .execution_settings
                .as_ref()
                .map_or(stage.config.request.concurrency, |s| s.policy.concurrency);
            if can_start && running.len() < concurrency as usize {
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
            if can_start && running.is_empty() && stage.attempts < stage.call_limit() {
                let ledger = db.clone();
                let sid = id.clone();
                if let Some(due) = work(move || ledger.next_retry_at(&sid)).await? {
                    let now = studio_storage::now().parse::<u64>().unwrap_or(0);
                    let delay = Duration::from_millis(due.saturating_sub(now).clamp(50, 2000));
                    tokio::select! {_=tokio::time::sleep(delay)=>{},_=control.cancel.cancelled()=>break}
                    continue;
                }
            }
            if can_start && running.is_empty() && stage.sampling.is_some() {
                match self
                    .plan_round(db.clone(), id.clone(), control.clone())
                    .await
                {
                    Ok(true) => continue,
                    Ok(false) => break,
                    Err(error) if error.code == "CANCELLED" => break,
                    Err(error) => {
                        failure = Some(error);
                        continue;
                    }
                }
            }
            let Some(result) = running.next().await else {
                break;
            };
            if let Err(error) = result
                && error.code != "EVALUATION_BATCH"
            {
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
        let request_kib = request_reservation_kib(&stage, &batch)?;
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
                .acquire_many_owned(request_kib)
                .await
                .map_err(Error::io)?;
            Ok::<_, Error>((request, bytes))
        };
        let permits = tokio::select! {r=admitted=>r?,_=control.cancel.cancelled()=>return Err(Error::new("CANCELLED","评审已取消"))};
        let reserved = self
            .reserved
            .fetch_add(u64::from(request_kib) * 1024, Ordering::Relaxed)
            + u64::from(request_kib) * 1024;
        self.peak.fetch_max(reserved, Ordering::Relaxed);
        self.active_requests.fetch_add(1, Ordering::Relaxed);
        let _account = Admission {
            runner: self.clone(),
            _permits: permits,
            request_kib,
        };
        let sid = stage.id.clone();
        let sequence = batch.sequence;
        let _transfer = TransferGuard {
            runner: self.clone(),
            key: (pid.clone(), sid.clone(), sequence),
        };
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
                let messages = match media::prepare_images(&copy, &media_pid, &config, &mut batch, cancel)? {
                    media::Prepared::Messages(messages)=>messages,
                    media::Prepared::Rejected(rejected)=>{
                        let valid=batch.members.iter().filter(|m|!rejected.iter().any(|r|r.0==m.candidate.ordinal)).cloned().collect();
                        copy.store.evaluation(&media_pid)?.regroup(&config.id,batch.sequence,vec![valid],rejected,"image_preflight_failed")?;
                        return Ok(None);
                    }
                };
                if serde_json::to_vec(&messages).map_err(Error::io)?.len() as u64 > config.config.max_request_bytes {
                    if batch.members.len()<=2 {return Err(Error::invalid("最小比较批次仍超过请求预算"));}
                    let middle=batch.members.len()/2;
                    copy.store.evaluation(&media_pid)?.regroup(&config.id,batch.sequence,vec![batch.members[..middle].to_vec(),batch.members[middle..].to_vec()],vec![],"input_byte_limit")?;
                    return Ok(None);
                }
                let mut plan = execution::prepare(&copy,&config,messages,studio_domain::new_id(),false)?;
                // One ledger attempt is one network attempt; explicit 429 retries are visible here.
                plan.provider.config.network.rate_limit_retries = 0;
                use sha2::{Digest, Sha256};
                let options=config.execution_settings.as_ref().map(|s|studio_application::aesthetic::recorded_options(&s.policy));
                let preview=match &options {Some(options)=>copy.llm.preview_recorded(&plan,options)?,None=>copy.llm.preview(&plan)?};
                let body = serde_json::to_vec(&preview).map_err(Error::io)?;
                let size = body.len() as u64;
                let semantic_hash = hex::encode(Sha256::digest(serde_json::to_vec(&serde_json::json!({
                    "version": 2, "config_hash": config.config_hash, "execution": config.execution_settings, "batch": batch.sequence,
                    "members": batch.members, "native_body_sha256": hex::encode(Sha256::digest(&body)),
                })).map_err(Error::io)?));
                if size > config.config.max_request_bytes {
                    if batch.members.len()<=2 {return Err(Error::invalid("最小比较批次仍超过请求预算，请检查冻结提示词与图片规格"));}
                    let middle=batch.members.len()/2;
                    copy.store.evaluation(&media_pid)?.regroup(&config.id,batch.sequence,vec![batch.members[..middle].to_vec(),batch.members[middle..].to_vec()],vec![],"native_request_byte_limit")?;
                    return Ok(None);
                }
                Ok(Some((plan, batch.members, size, semantic_hash)))
            })
            .await
        };
        let (plan, members, size, semantic_hash) = match prepared {
            Ok(Some(value)) => value,
            Ok(None) => return Ok(()),
            // Pausing may interrupt image reads before the send commitment. Leave the
            // batch preparing so settlement returns it to the queue without locking images.
            Err(error) if error.code == "CANCELLED" => return Ok(()),
            Err(error) => {
                let ledger = db.clone();
                let id = sid.clone();
                let message = error.to_string();
                work(move || ledger.preparation_failed(&id, sequence, message)).await?;
                return Err(
                    if matches!(error.code, "SOURCE_CHANGED" | "INVALID_INPUT" | "IO_ERROR") {
                        Error::new("EVALUATION_BATCH", message_for_batch(&error))
                    } else {
                        error
                    },
                );
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
        let sink = Arc::new(ReceiptSink {
            runner: self.clone(),
            db: db.clone(),
            pid: pid.clone(),
            volume: volume.clone(),
            stage: sid.clone(),
            attempt: attempt.clone(),
            sequence,
            members,
            semantic_hash,
            size,
            directory: state.store.directory(&pid)?,
            stop_dispatch: control.stop_dispatch.clone(),
        });
        let response = if let Some(settings) = &stage.execution_settings {
            state
                .llm
                .generate_recorded_with_options(
                    plan,
                    control.cancel.clone(),
                    sink,
                    studio_application::aesthetic::recorded_options(&settings.policy),
                )
                .await
        } else {
            state
                .llm
                .generate_recorded(plan, control.cancel.clone(), sink)
                .await
        };
        match response {
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
                let id = sid.clone();
                let result = work(move || {
                    ledger.filtered_batches(
                        &id,
                        sequence.saturating_sub(1),
                        1,
                        None,
                        Some(sequence),
                    )
                })
                .await?;
                if result.first().is_some_and(|b| b.state == "invalid") {
                    return Err(Error::new(
                        "EVALUATION_BATCH",
                        "此批返回未通过格式校验，请查看或本地重解析",
                    ));
                }
                Ok(())
            }
            Err(failure) => {
                let ledger = db.clone();
                let id = sid.clone();
                let aid = attempt.clone();
                let sent = work(move || ledger.attempt_exists(&id, &aid)).await?;
                if failure.code == "LLM_NOT_DISPATCHED" {
                    if sent {
                        self.persist_outcome(
                            &db,
                            &pid,
                            &volume,
                            &sid,
                            &attempt,
                            Outcome::Failure(failure),
                        )
                        .await?;
                    }
                    return Ok(());
                }
                if !sent {
                    // Unsent admission failures retain their queued batch. Requiring a paid
                    // retry here would turn a recoverable storage pause into a candidate lock.
                    let local_code = match failure.code.as_str() {
                        "EVALUATION_STORAGE_UNHEALTHY" => Some("EVALUATION_STORAGE_UNHEALTHY"),
                        "EVALUATION_STORAGE_FULL" => Some("EVALUATION_STORAGE_FULL"),
                        "EVALUATION_STORAGE_IO" => Some("EVALUATION_STORAGE_IO"),
                        "EVALUATION_CORRUPT" => Some("EVALUATION_CORRUPT"),
                        "EVALUATION_WRITER_EXITED" => Some("EVALUATION_WRITER_EXITED"),
                        "EVALUATION_BUSY" => Some("EVALUATION_BUSY"),
                        "DATABASE_ERROR" => Some("DATABASE_ERROR"),
                        "IO_ERROR" => Some("IO_ERROR"),
                        "LLM_QUEUE_FULL" => Some("LLM_QUEUE_FULL"),
                        "CANCELLED" => Some("CANCELLED"),
                        _ => None,
                    };
                    if let Some(code) = local_code {
                        return Err(Error::new(code, failure.message));
                    }
                    let ledger = db.clone();
                    let id = sid.clone();
                    let message = format!("{}: {}", failure.code, failure.message);
                    work(move || ledger.preparation_failed(&id, sequence, message)).await?;
                    return Err(Error::new(
                        "EVALUATION_REMOTE",
                        format!("{}: {}", failure.code, failure.message),
                    ));
                }
                self.persist_outcome(
                    &db,
                    &pid,
                    &volume,
                    &sid,
                    &attempt,
                    Outcome::Failure(failure.clone()),
                )
                .await?;
                let ledger = db.clone();
                let id = sid;
                let message = format!("{}: {}", failure.code, failure.message);
                let recovery =
                    work(move || ledger.schedule_recovery(&id, sequence, failure)).await?;
                if recovery == "halt" {
                    Err(Error::new("EVALUATION_REMOTE", message))
                } else {
                    Ok(())
                }
            }
        }
    }
    pub fn check_start(&self, state: &AppState, pid: &str) -> Result<()> {
        creation::check_storage(&state.store.directory(pid)?)?;
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
                    Outcome::Raw(v) => ledger.save_raw(&id, &aid, v)?,
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
    Raw(LlmRawReceipt),
    Receipt(AestheticReceipt),
    Failure(LlmFailure),
}
struct ReceiptSink {
    runner: Arc<Runner>,
    db: Arc<EvaluationDb>,
    pid: String,
    volume: String,
    stage: String,
    attempt: String,
    sequence: u64,
    members: Vec<AestheticMember>,
    semantic_hash: String,
    size: u64,
    directory: PathBuf,
    stop_dispatch: Arc<AtomicBool>,
}
impl studio_application::llm::LlmReceiptSink for ReceiptSink {
    fn dispatch_cancelled(&self) -> bool {
        self.stop_dispatch.load(Ordering::Acquire)
    }
    fn before_send(&self) -> futures::future::BoxFuture<'_, Result<LlmDispatchDecision>> {
        async move {
            if self.dispatch_cancelled() {
                return Ok(LlmDispatchDecision {
                    defer: true,
                    recovery_deadline_ms: None,
                });
            }
            let db = self.db.clone();
            let id = self.stage.clone();
            let sequence = self.sequence;
            let members = self.members.clone();
            let attempt = self.attempt.clone();
            let hash = self.semantic_hash.clone();
            let runner = self.runner.clone();
            let pid = self.pid.clone();
            let volume = self.volume.clone();
            let directory = self.directory.clone();
            let decision = work(move || {
                studio_storage::faults::check("dispatch_after_upload", &id)?;
                let sent = runner.gate.commit(&pid, &volume, || {
                    creation::check_storage(&directory)?;
                    db.begin_attempt(&id, sequence, members, attempt, hash)
                })?;
                Ok(LlmDispatchDecision {
                    defer: !sent,
                    recovery_deadline_ms: if sent {
                        db.batch_deadline(&id, sequence)?
                    } else {
                        None
                    },
                })
            })
            .await?;
            if !decision.defer {
                self.runner.uploaded.fetch_add(self.size, Ordering::Relaxed);
            }
            Ok(decision)
        }
        .boxed()
    }
    fn progress(&self, p: LlmTransferProgress) {
        if let Ok(mut values) = self.runner.transfers.lock() {
            let value = values
                .entry((self.pid.clone(), self.stage.clone(), self.sequence))
                .or_insert(AestheticTransfer {
                    phase: p.phase.into(),
                    started_at: None,
                    last_data_at: None,
                    received_bytes: 0,
                });
            value.phase = p.phase.into();
            value.started_at = p.started_at_ms.map(|v| v.to_string());
            if let Some(last) = p.last_data_at_ms {
                value.last_data_at = Some(last.to_string());
            }
            value.received_bytes = p.received_bytes;
        }
    }

    fn persist(&self, receipt: LlmRawReceipt) -> futures::future::BoxFuture<'_, Result<()>> {
        async move {
            self.runner
                .persist_outcome(
                    &self.db,
                    &self.pid,
                    &self.volume,
                    &self.stage,
                    &self.attempt,
                    Outcome::Raw(receipt),
                )
                .await
        }
        .boxed()
    }
}
struct Admission {
    request_kib: u32,
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
            .fetch_sub(u64::from(self.request_kib) * 1024, Ordering::Relaxed);
        self.runner.active_requests.fetch_sub(1, Ordering::Relaxed);
    }
}

fn message_for_batch(error: &Error) -> String {
    format!("{}: {}", error.code, error.message)
}
struct TransferGuard {
    runner: Arc<Runner>,
    key: (String, String, u64),
}
impl Drop for TransferGuard {
    fn drop(&mut self) {
        if let Ok(mut transfers) = self.runner.transfers.lock() {
            transfers.remove(&self.key);
        }
    }
}
