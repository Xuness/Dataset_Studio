use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::{Duration, Instant},
};
use studio_application::{Media, MediaInput, MediaSource, ReadResources, read_cancelled};
use studio_domain::*;
use studio_resources::{CachePin, PreviewCache, epoch_ms, preview_key};
use studio_sources::SourceRouter;
use studio_storage::SqliteStore;
#[cfg(test)]
mod tests;

pub struct Preview {
    pub media: Media,
    pub cache: &'static str,
    pub freshness: &'static str,
    pub verified_ms: u64,
    _pin: Option<CachePin>,
}
pub struct PreviewOptions {
    pub edge: u32,
    pub priority: ReadPriority,
    pub max_source_bytes: u64,
}
#[derive(Default, Clone, serde::Serialize)]
pub struct PreviewMetrics {
    pub shared: u64,
    pub queued: usize,
    pub active_subscriptions: usize,
    pub generated: u64,
    pub cancelled_last: u64,
    pub cancelled_before_read: u64,
    pub source_bytes: u64,
    pub pack_opens: u64,
    pub seeks: u64,
    pub decode_ms: u64,
    pub read_ms: u64,
    pub batches: u64,
    pub max_batch: usize,
    pub queue_wait_ms: u64,
    pub max_queue_wait_ms: u64,
    pub cancelled_finished: u64,
    pub max_cancel_latency_ms: u64,
    pub trace: Vec<PhysicalReadTrace>,
}
struct Subscription {
    cancelled: Arc<AtomicBool>,
    work: Option<Weak<Work>>,
}
struct Work {
    created: Instant,
    cancelled_at: Mutex<Option<Instant>>,
    key: String,
    source: Source,
    asset_id: String,
    edge: u32,
    bytes: u64,
    priority: AtomicU8,
    cancelled: Arc<AtomicBool>,
    consumers: Mutex<Vec<Arc<AtomicBool>>>,
    output: tokio::sync::watch::Sender<Option<Result<Arc<Preview>>>>,
}
impl Work {
    fn priority(&self) -> ReadPriority {
        match self.priority.load(Ordering::Acquire) {
            0 => ReadPriority::Interactive,
            1 => ReadPriority::Background,
            _ => ReadPriority::Prefetch,
        }
    }
    fn check_consumers(&self) -> bool {
        if self.output.borrow().is_some() {
            return false;
        }
        let empty = self
            .consumers
            .lock()
            .map(|c| c.iter().all(|v| v.load(Ordering::Acquire)))
            .unwrap_or(true);
        if empty && !self.cancelled.swap(true, Ordering::AcqRel) {
            if let Ok(mut time) = self.cancelled_at.lock() {
                *time = Some(Instant::now());
            }
            true
        } else {
            false
        }
    }
}
struct State {
    tickets: HashMap<String, Subscription>,
    cancelled_early: VecDeque<(String, Instant)>,
    work: HashMap<String, Arc<Work>>,
    queue: VecDeque<Arc<Work>>,
    interactive_streak: usize,
    metrics: PreviewMetrics,
}
pub struct PreviewService {
    pub resources: Arc<dyn ReadResources>,
    pub cache: PreviewCache,
    state: Mutex<State>,
    notify: tokio::sync::Notify,
    stopping: AtomicBool,
}
pub struct ReadTicket {
    service: Arc<PreviewService>,
    key: String,
    pub cancelled: Arc<AtomicBool>,
}
impl Drop for ReadTicket {
    fn drop(&mut self) {
        self.service.detach(&self.key);
    }
}
fn state_error() -> Error {
    Error::new("INTERNAL_ERROR", "共享读取状态不可用")
}
impl PreviewService {
    pub fn new(resources: Arc<dyn ReadResources>, cache: PreviewCache) -> Arc<Self> {
        Arc::new(Self {
            resources,
            cache,
            state: Mutex::new(State {
                tickets: HashMap::new(),
                cancelled_early: VecDeque::new(),
                work: HashMap::new(),
                queue: VecDeque::new(),
                interactive_streak: 0,
                metrics: PreviewMetrics::default(),
            }),
            notify: tokio::sync::Notify::new(),
            stopping: AtomicBool::new(false),
        })
    }
    pub fn ticket(self: &Arc<Self>, pid: &str, id: &str) -> Result<ReadTicket> {
        validate_id(id)?;
        let key = format!("{pid}:{id}");
        let mut state = self.state.lock().map_err(|_| state_error())?;
        state
            .cancelled_early
            .retain(|(_, t)| t.elapsed() < Duration::from_secs(60));
        if state.cancelled_early.iter().any(|(k, _)| k == &key)
            || self.stopping.load(Ordering::Acquire)
        {
            return Err(Error::new("CANCELLED", "读取订阅已取消"));
        }
        if state.tickets.contains_key(&key) {
            return Err(Error::invalid("读取订阅身份重复"));
        }
        if state.tickets.len() >= 256 {
            return Err(Error::new("READ_BUDGET_EXCEEDED", "读取订阅队列已满"));
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        state.tickets.insert(
            key.clone(),
            Subscription {
                cancelled: cancelled.clone(),
                work: None,
            },
        );
        Ok(ReadTicket {
            service: self.clone(),
            key,
            cancelled,
        })
    }
    pub fn cancel(&self, pid: &str, id: &str) -> Result<()> {
        validate_id(id)?;
        let key = format!("{pid}:{id}");
        let mut state = self.state.lock().map_err(|_| state_error())?;
        if let Some(ticket) = state.tickets.get(&key) {
            ticket.cancelled.store(true, Ordering::Release);
            let last = ticket
                .work
                .as_ref()
                .and_then(Weak::upgrade)
                .is_some_and(|w| w.check_consumers());
            state.metrics.cancelled_last += u64::from(last);
        }
        state
            .cancelled_early
            .retain(|(k, t)| k != &key && t.elapsed() < Duration::from_secs(60));
        while state.cancelled_early.len() >= 1024 {
            state.cancelled_early.pop_front();
        }
        state.cancelled_early.push_back((key, Instant::now()));
        self.notify.notify_one();
        Ok(())
    }
    fn detach(&self, key: &str) {
        if let Ok(mut state) = self.state.lock()
            && let Some(ticket) = state.tickets.remove(key)
        {
            ticket.cancelled.store(true, Ordering::Release);
            let last = ticket
                .work
                .as_ref()
                .and_then(Weak::upgrade)
                .is_some_and(|w| w.check_consumers());
            state.metrics.cancelled_last += u64::from(last);
            self.notify.notify_one();
        }
    }
    pub fn metrics(&self) -> PreviewMetrics {
        self.state
            .lock()
            .map(|s| {
                let mut m = s.metrics.clone();
                m.queued = s.queue.len();
                m.active_subscriptions = s.tickets.len();
                m
            })
            .unwrap_or_default()
    }
    pub fn shutdown(&self) {
        self.stopping.store(true, Ordering::Release);
        if let Ok(state) = self.state.lock() {
            for ticket in state.tickets.values() {
                ticket.cancelled.store(true, Ordering::Release);
            }
            for work in state.work.values() {
                work.cancelled.store(true, Ordering::Release);
            }
        }
        self.notify.notify_one();
    }
    pub async fn get(
        self: &Arc<Self>,
        store: Arc<SqliteStore>,
        ticket: &ReadTicket,
        pid: String,
        asset: AssetKey,
        options: PreviewOptions,
    ) -> Result<Arc<Preview>> {
        let service = self.clone();
        let cancel = ticket.cancelled.clone();
        let ticket_key = ticket.key.clone();
        let edge = options.edge.clamp(96, 1600);
        let priority = options.priority;
        let byte_limit = options.max_source_bytes.min(64 << 20);
        let AssetKey {
            source_id: sid,
            asset_id: aid,
        } = asset;
        let prepared = tokio::task::spawn_blocking(move || -> Result<_> {
            let _permit = service.resources.acquire(
                ReadRequest {
                    class: ReadClass::Index,
                    priority,
                    bytes: 32 << 20,
                },
                &cancel,
            )?;
            read_cancelled(&cancel)?;
            // This project membership check is mandatory even on shared/offline hits.
            let source = store.source(&pid, &sid)?;
            let version = SourceRouter.content_version(&source, &aid)?;
            let key = preview_key(&source, &aid, &version, edge);
            let identity = SourceRouter.verify_media_identity(&source, &aid);
            let online = match &identity {
                Ok(_) => true,
                Err(error) if matches!(error.code, "IO_ERROR" | "SOURCE_UNAVAILABLE") => false,
                Err(error) => return Err(error.clone()),
            };
            read_cancelled(&cancel)?;
            // Serialize joining with completion. A request which checked the cache
            // while another generator was publishing cannot start a duplicate read.
            let mut state = service.state.lock().map_err(|_| state_error())?;
            let existing = state
                .work
                .get(&key)
                .filter(|w| online && !w.cancelled.load(Ordering::Acquire))
                .cloned();
            let work = if let Some(work) = existing {
                if work.bytes > byte_limit {
                    return Err(Error::new(
                        "READ_BUDGET_EXCEEDED",
                        "共享读取所需字节超过该请求预算",
                    ));
                }
                state.metrics.shared += 1;
                work.priority.fetch_min(priority as u8, Ordering::AcqRel);
                work
            } else {
                if let Some(cached) = service.cache.get(&key, online)? {
                    return Ok((
                        None,
                        Some(Arc::new(Preview {
                            media: Media {
                                bytes: cached.bytes,
                                content_type: "image/jpeg".into(),
                            },
                            cache: "hit",
                            freshness: if online { "verified" } else { "offline_cached" },
                            verified_ms: cached.verified_ms,
                            _pin: Some(cached.pin),
                        })),
                    ));
                }
                let identity = identity.map_err(|e| {
                    Error::new(
                        "SOURCE_UNAVAILABLE",
                        format!("来源离线且没有可用缓存：{}", e.message),
                    )
                })?;
                if identity.bytes > byte_limit {
                    return Err(Error::new(
                        "READ_BUDGET_EXCEEDED",
                        "源图片读取量超过该请求预算",
                    ));
                }
                if identity.bytes > 64 << 20 {
                    return Err(Error::new(
                        "MEDIA_TOO_LARGE",
                        "预览支持最大 64 MiB 的单张图片",
                    ));
                }
                if state.work.len() >= 128 || state.queue.len() >= 128 {
                    return Err(Error::new("READ_BUDGET_EXCEEDED", "缩略图生成队列已满"));
                }
                let (output, _) = tokio::sync::watch::channel(None);
                let work = Arc::new(Work {
                    created: Instant::now(),
                    cancelled_at: Mutex::new(None),
                    key: key.clone(),
                    source,
                    asset_id: aid,
                    edge,
                    bytes: identity.bytes,
                    priority: AtomicU8::new(priority as u8),
                    cancelled: Arc::new(AtomicBool::new(false)),
                    consumers: Mutex::new(Vec::new()),
                    output,
                });
                state.work.insert(key, work.clone());
                state.queue.push_back(work.clone());
                work
            };
            {
                let mut consumers = work.consumers.lock().map_err(|_| state_error())?;
                consumers.retain(|c| !c.load(Ordering::Acquire));
                consumers.push(cancel.clone());
            }
            state
                .tickets
                .get_mut(&ticket_key)
                .ok_or_else(|| Error::new("CANCELLED", "订阅已离开"))?
                .work = Some(Arc::downgrade(&work));
            Ok((Some(work), None))
        })
        .await
        .map_err(Error::io)??;
        read_cancelled(&ticket.cancelled)?;
        if let Some(hit) = prepared.1 {
            return Ok(hit);
        }
        let work = prepared.0.expect("cache hit or queued work");
        self.notify.notify_one();
        let mut output = work.output.subscribe();
        loop {
            read_cancelled(&ticket.cancelled)?;
            if let Some(result) = output.borrow_and_update().clone() {
                return result;
            }
            tokio::select! {
                changed = output.changed() => { changed.map_err(Error::io)?; },
                _ = tokio::time::sleep(Duration::from_millis(20)) => {},
            }
        }
    }
    fn take_batch(&self) -> Vec<Arc<Work>> {
        let Ok(mut state) = self.state.lock() else {
            return Vec::new();
        };
        let mut batch = Vec::<Arc<Work>>::new();
        let mut bytes = 0;
        while batch.len() < 16 {
            let chosen = if state.interactive_streak >= 4 {
                (!state.queue.is_empty()).then_some(0)
            } else {
                None
            }
            .or_else(|| {
                state
                    .queue
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, w)| w.priority())
                    .map(|(i, _)| i)
            });
            let Some(index) = chosen else {
                break;
            };
            let work = &state.queue[index];
            if let Some(first) = batch.first()
                && (first.source.id != work.source.id || bytes + work.bytes > 64 << 20)
            {
                break;
            }
            let work = state.queue.remove(index).expect("selected queue member");
            if work.cancelled.load(Ordering::Acquire) {
                Self::cancel_metrics(&mut state, &work);
                state.metrics.cancelled_before_read += 1;
                let _ = work
                    .output
                    .send(Some(Err(Error::new("CANCELLED", "读取已取消"))));
                if state
                    .work
                    .get(&work.key)
                    .is_some_and(|w| Arc::ptr_eq(w, &work))
                {
                    state.work.remove(&work.key);
                }
                continue;
            }
            state.interactive_streak = if state.interactive_streak >= 4 {
                0
            } else {
                state.interactive_streak + 1
            };
            bytes += work.bytes;
            let waited = work.created.elapsed().as_millis() as u64;
            state.metrics.queue_wait_ms += waited;
            state.metrics.max_queue_wait_ms = state.metrics.max_queue_wait_ms.max(waited);
            batch.push(work);
        }
        batch
    }
    fn finish(&self, work: &Arc<Work>, result: Result<Arc<Preview>>) {
        work.output.send_replace(Some(result));
        if let Ok(mut state) = self.state.lock() {
            Self::cancel_metrics(&mut state, work);
            if state
                .work
                .get(&work.key)
                .is_some_and(|w| Arc::ptr_eq(w, work))
            {
                state.work.remove(&work.key);
            }
        }
    }
    fn cancel_metrics(state: &mut State, work: &Work) {
        if let Ok(time) = work.cancelled_at.lock()
            && let Some(time) = *time
        {
            state.metrics.cancelled_finished += 1;
            state.metrics.max_cancel_latency_ms = state
                .metrics
                .max_cancel_latency_ms
                .max(time.elapsed().as_millis() as u64);
        }
    }
    async fn batch(self: &Arc<Self>, batch: Vec<Arc<Work>>) {
        let service = self.clone();
        let jobs = batch.clone();
        let read = tokio::task::spawn_blocking(move || -> Result<_> {
            let priority = jobs
                .iter()
                .map(|w| w.priority())
                .min()
                .unwrap_or(ReadPriority::Prefetch);
            // A batch has a separate cancellation flag which reflects all consumers.
            // The monitor below updates it while this thread waits for admission.
            let all_cancelled = Arc::new(AtomicBool::new(false));
            let stop = Arc::new(AtomicBool::new(false));
            let watch_cancel = all_cancelled.clone();
            let watch_stop = stop.clone();
            let watch_jobs = jobs.clone();
            let monitor = std::thread::spawn(move || {
                while !watch_stop.load(Ordering::Acquire) {
                    if watch_jobs
                        .iter()
                        .all(|w| w.cancelled.load(Ordering::Acquire))
                    {
                        watch_cancel.store(true, Ordering::Release);
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            });
            let permit = service.resources.acquire(
                ReadRequest {
                    class: ReadClass::Media,
                    priority,
                    bytes: jobs.iter().map(|w| w.bytes).sum(),
                },
                &all_cancelled,
            );
            stop.store(true, Ordering::Release);
            let _ = monitor.join();
            let _permit = permit.inspect_err(|error| {
                if error.code == "CANCELLED"
                    && let Ok(mut state) = service.state.lock()
                {
                    state.metrics.cancelled_before_read += jobs.len() as u64;
                }
            })?;
            let start = Instant::now();
            let inputs = jobs
                .iter()
                .map(|w| MediaInput {
                    asset_id: w.asset_id.clone(),
                    cancelled: w.cancelled.clone(),
                    byte_limit: w.bytes,
                })
                .collect::<Vec<_>>();
            let result = SourceRouter.read_many(&jobs[0].source, &inputs)?;
            if let Ok(mut state) = service.state.lock() {
                let m = &mut state.metrics;
                m.batches += 1;
                m.max_batch = m.max_batch.max(jobs.len());
                m.source_bytes += result.stats.bytes;
                m.pack_opens += result.stats.opens;
                m.seeks += result.stats.seeks;
                m.cancelled_before_read += result.stats.cancelled_before_read;
                m.read_ms += start.elapsed().as_millis() as u64;
                m.trace.extend(result.stats.trace);
                if m.trace.len() > 128 {
                    m.trace.drain(..m.trace.len() - 128);
                }
            }
            Ok(result.items)
        })
        .await
        .map_err(Error::io)
        .and_then(|r| r);
        match read {
            Err(error) => {
                for work in batch {
                    self.finish(&work, Err(error.clone()));
                }
            }
            Ok(items) => {
                // At most two buffers are decoding concurrently; the entire staged
                // source batch is capped at 64 MiB and only one batch is staged.
                let mut pending = tokio::task::JoinSet::new();
                for (work, item) in batch.into_iter().zip(items) {
                    if pending.len() >= 2 {
                        let _ = pending.join_next().await;
                    }
                    let service = self.clone();
                    pending.spawn_blocking(move || {
                        let result = (|| {
                            let media = item?;
                            let _permit = service.resources.acquire(
                                ReadRequest {
                                    class: ReadClass::Decode,
                                    priority: work.priority(),
                                    bytes: 256 << 20,
                                },
                                &work.cancelled,
                            )?;
                            read_cancelled(&work.cancelled)?;
                            let start = Instant::now();
                            let media = studio_sources::thumbnail(media, work.edge)?;
                            if let Ok(mut state) = service.state.lock() {
                                state.metrics.decode_ms += start.elapsed().as_millis() as u64;
                                state.metrics.generated += 1;
                            }
                            read_cancelled(&work.cancelled)?;
                            let pin = service.cache.put(&work.key, &media.bytes)?;
                            Ok(Arc::new(Preview {
                                media,
                                cache: "generated",
                                freshness: "verified",
                                verified_ms: epoch_ms(),
                                _pin: pin,
                            }))
                        })();
                        service.finish(&work, result);
                    });
                }
                while pending.join_next().await.is_some() {}
            }
        }
    }
    pub async fn run(self: Arc<Self>) {
        let mut maintenance = tokio::time::interval(Duration::from_secs(2));
        while !self.stopping.load(Ordering::Acquire) {
            tokio::select! { _ = self.notify.notified() => {}, _ = maintenance.tick() => {
                let cache = self.cache.clone();
                let _ = tokio::task::spawn_blocking(move || cache.maintain(128)).await;
            } }
            // A small, bounded gather window groups visible-range requests by pack.
            tokio::time::sleep(Duration::from_millis(8)).await;
            loop {
                let batch = self.take_batch();
                if batch.is_empty() {
                    break;
                }
                self.batch(batch).await;
            }
        }
        let batch = self.take_batch();
        for work in batch {
            self.finish(&work, Err(Error::new("CANCELLED", "引擎已停止")));
        }
    }
}
