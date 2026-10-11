//! Shared, resumable visual jobs. Rust owns state and resource admission;
//! the supervised Python process owns model execution and Lance table writes.
mod bundle;
mod process;
#[cfg(windows)]
mod process_tree;
mod results;

use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
};
use studio_application::{
    QueryAdapter, ReadCancellation, SourceAdapter,
    visual::{VisualExecutor, VisualRepository},
};
use studio_domain::{
    Error, METADATA_MEMORY_BYTES, ReadClass, ReadPriority, ReadRequest, Result, visual::*,
};
use studio_storage::{SqliteStore, atomic_json};
use studio_visual_store::VisualStore;

struct Session {
    config: VisualRuntimeConfig,
    store: Arc<VisualStore>,
    worker: Mutex<Arc<process::Worker>>,
    description: Value,
}
struct Active {
    id: String,
    action: Arc<AtomicU8>, // 0 run, 1 pause, 2 cancel
    cancelled: ReadCancellation,
}
pub struct Backend {
    root: PathBuf,
    registry: Arc<SqliteStore>,
    sources: Arc<crate::sources::SourceService>,
    resources: Arc<dyn studio_application::ReadResources>,
    session: Mutex<Option<Arc<Session>>>,
    gate: Mutex<()>,
    active: Mutex<Option<Active>>,
    stopped: AtomicBool,
}
fn lock(e: impl std::fmt::Display) -> Error {
    Error::new("VISUAL_UNAVAILABLE", e.to_string())
}
fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|e| Error::new("VISUAL_PROTOCOL", e.to_string()))
}
fn flag() -> ReadCancellation {
    Arc::new(AtomicBool::new(false))
}
fn lease_id(run: &str, source: &str) -> String {
    format!("visual/{run}/{source}")
}

impl Backend {
    pub fn new(
        root: PathBuf,
        registry: Arc<SqliteStore>,
        sources: Arc<crate::sources::SourceService>,
        resources: Arc<dyn studio_application::ReadResources>,
    ) -> Arc<Self> {
        Arc::new(Self {
            root,
            registry,
            sources,
            resources,
            session: Mutex::new(None),
            gate: Mutex::new(()),
            active: Mutex::new(None),
            stopped: AtomicBool::new(false),
        })
    }
    pub fn busy(&self) -> bool {
        self.active.lock().map_or(true, |a| a.is_some())
    }
    pub fn runtime(&self) -> Result<Value> {
        let config = fs::read(self.root.join("visual-runtime.json"))
            .ok()
            .map(|v| serde_json::from_slice::<VisualRuntimeConfig>(&v))
            .transpose()
            .map_err(Error::io)?;
        let session = self.session.lock().map_err(lock)?;
        let (running, description) = session
            .as_ref()
            .map(|s| {
                (
                    s.worker.lock().is_ok_and(|w| w.alive()),
                    s.description.clone(),
                )
            })
            .unwrap_or((false, Value::Null));
        Ok(
            json!({"configured":config.is_some(),"config":config,"worker_running":running,"busy":self.busy(),"description":description}),
        )
    }
    fn open(&self, config: VisualRuntimeConfig) -> Result<Arc<Session>> {
        let store = Arc::new(VisualStore::open(&config.materials_root)?);
        let worker = process::Worker::start(&self.root, &config)?;
        let description = worker.request("describe", json!({"model_root":config.model_root}))?;
        let recipes: Vec<VisualRecipe> = decode(description["recipes"].clone())?;
        store.register_recipes(&recipes)?;
        Ok(Arc::new(Session {
            config,
            store,
            worker: Mutex::new(worker),
            description,
        }))
    }
    fn ensure(&self) -> Result<Arc<Session>> {
        if self.stopped.load(Ordering::Acquire) {
            return Err(lock("视觉服务正在停止"));
        }
        let mut session = self.session.lock().map_err(lock)?;
        if let Some(session) = &*session {
            return Ok(session.clone());
        }
        let config: VisualRuntimeConfig = serde_json::from_slice(
            &fs::read(self.root.join("visual-runtime.json"))
                .map_err(|_| Error::new("VISUAL_UNAVAILABLE", "请先配置视觉运行环境"))?,
        )
        .map_err(Error::io)?;
        let created = self.open(config)?;
        *session = Some(created.clone());
        Ok(created)
    }
    fn worker(&self, session: &Session) -> Result<Arc<process::Worker>> {
        let mut worker = session.worker.lock().map_err(lock)?;
        if !worker.alive() {
            worker.stop()?;
            *worker = process::Worker::start(&self.root, &session.config)?;
        }
        Ok(worker.clone())
    }
    pub fn configure(&self, mut config: VisualRuntimeConfig) -> Result<Value> {
        let _gate = self.gate.lock().map_err(lock)?;
        if self.busy() {
            return Err(Error::new("VISUAL_BUSY", "请先暂停当前视觉任务再修改环境"));
        }
        if !matches!(config.device.as_str(), "cuda" | "cpu") {
            return Err(Error::invalid("设备须为 cuda 或 cpu"));
        }
        config.python = config.python.canonicalize().map_err(Error::io)?;
        config.model_root = config.model_root.canonicalize().map_err(Error::io)?;
        fs::create_dir_all(&config.materials_root).map_err(Error::io)?;
        config.materials_root = config.materials_root.canonicalize().map_err(Error::io)?;
        for path in self
            .registry
            .source_location_roots()?
            .iter()
            .chain(std::iter::once(&config.model_root))
        {
            let path = path.canonicalize().map_err(Error::io)?;
            if config.materials_root.starts_with(&path) || path.starts_with(&config.materials_root)
            {
                return Err(Error::invalid("视觉材料目录须独立于数据湖和模型目录"));
            }
        }
        let previous = self.session.lock().map_err(lock)?.clone();
        let old_root = previous
            .as_ref()
            .map(|s| s.config.materials_root.clone())
            .or_else(|| {
                fs::read(self.root.join("visual-runtime.json"))
                    .ok()
                    .and_then(|v| serde_json::from_slice::<VisualRuntimeConfig>(&v).ok())
                    .map(|c| c.materials_root)
            });
        if let Some(old_root) = old_root {
            let old_root = old_root.canonicalize().map_err(Error::io)?;
            if old_root != config.materials_root {
                let unfinished = if let Some(old) = &previous {
                    old.store.has_unfinished()?
                } else {
                    VisualStore::open(&old_root)?.has_unfinished()?
                };
                if unfinished {
                    return Err(Error::new(
                        "VISUAL_BUSY",
                        "原材料目录还有未完成任务，请恢复或取消后再切换目录",
                    ));
                }
            }
        }
        let session = if let Some(old) = previous
            .as_ref()
            .filter(|s| s.store.root() == config.materials_root)
        {
            old.worker.lock().map_err(lock)?.stop()?;
            let worker = process::Worker::start(&self.root, &config)?;
            let description =
                worker.request("describe", json!({"model_root":config.model_root}))?;
            old.store.register_recipes(&decode::<Vec<VisualRecipe>>(
                description["recipes"].clone(),
            )?)?;
            Arc::new(Session {
                config: config.clone(),
                store: old.store.clone(),
                worker: Mutex::new(worker),
                description,
            })
        } else {
            self.open(config.clone())?
        };
        atomic_json(&self.root.join("visual-runtime.json"), &config)?;
        if let Some(old) = previous {
            old.worker.lock().map_err(lock)?.stop()?;
        }
        *self.session.lock().map_err(lock)? = Some(session);
        self.runtime()
    }
    pub fn models(&self) -> Result<Value> {
        Ok(self.ensure()?.description.clone())
    }
    pub fn recipes(&self) -> Result<Vec<VisualRecipe>> {
        self.ensure()?.store.recipes()
    }
    pub fn sources(&self) -> Result<Vec<studio_domain::Source>> {
        self.registry.visual_sources()
    }
    pub fn runs(&self, after: u64, limit: u32) -> Result<Vec<VisualRun>> {
        self.ensure()?.store.runs(after, limit)
    }
    pub fn run(&self, id: &str) -> Result<VisualRun> {
        self.ensure()?.store.run(id)
    }
    pub fn coverage(&self) -> Result<Vec<VisualCoverage>> {
        self.ensure()?.store.coverage()
    }
    pub fn failures(&self, id: &str, after: i64, limit: u32) -> Result<Vec<VisualOutcome>> {
        let session = self.ensure()?;
        session.store.run(id)?;
        if after < 0 {
            return Err(Error::invalid("失败游标不能为负数"));
        }
        session.store.failures(id, after, limit)
    }

    pub fn create(self: &Arc<Self>, mut request: VisualRunRequest) -> Result<VisualRun> {
        let _gate = self.gate.lock().map_err(lock)?;
        if self.busy() {
            return Err(Error::new("VISUAL_BUSY", "当前已有视觉提取或索引任务"));
        }
        if request.source_ids.is_empty()
            || request.source_ids.len() > 3
            || request.recipe_ids.len() > 3
            || !(1..=512).contains(&request.batch_size)
            || request.max_objects_per_source == Some(0)
            || request.source_ids.iter().collect::<BTreeSet<_>>().len() != request.source_ids.len()
            || request.recipe_ids.iter().collect::<BTreeSet<_>>().len() != request.recipe_ids.len()
            || request.assets.len() > 4096
            || (request.scope_run_id.is_some() && !request.assets.is_empty())
            || ((request.scope_run_id.is_some() || !request.assets.is_empty())
                && request.max_objects_per_source.is_some())
            || request.micro_batch_sizes.iter().any(|(k, v)| {
                !matches!(
                    k.as_str(),
                    "siglip2_naflex" | "embeddinggemma2" | "kaloscope3"
                ) || !(1..=64).contains(v)
            })
            || request
                .assets
                .iter()
                .any(|k| !request.source_ids.contains(&k.source_id))
            || request
                .assets
                .iter()
                .map(|k| (&k.source_id, &k.asset_id))
                .collect::<BTreeSet<_>>()
                .len()
                != request.assets.len()
            || (!request.assets.is_empty()
                && request
                    .source_ids
                    .iter()
                    .any(|s| !request.assets.iter().any(|k| &k.source_id == s)))
        {
            return Err(Error::invalid(
                "视觉任务需要 1-3 个不同来源、最多 3 个配方、1-512 张批次",
            ));
        }
        let session = self.ensure()?;
        let current: Vec<VisualRecipe> = decode(session.description["recipes"].clone())?;
        for id in &request.recipe_ids {
            session.store.recipe(id)?;
            if !current.iter().any(|r| &r.id == id) {
                return Err(Error::invalid(
                    "旧配方的向量可继续检索；补算须使用匹配该配方的模型和运行环境",
                ));
            }
        }
        let available = self.registry.visual_sources()?;
        let mut snapshots = Vec::new();
        if let Some(id) = &request.scope_run_id {
            let root = session.store.scope_root(id)?;
            let original = session.store.run(&root)?;
            if request.source_ids.iter().collect::<BTreeSet<_>>()
                != original.request.source_ids.iter().collect::<BTreeSet<_>>()
            {
                return Err(Error::invalid("复用任务范围时来源集合须保持一致"));
            }
            snapshots = original.sources;
            for source in &mut snapshots {
                source.after_row = 0;
                source.discovered = 0;
                source.complete = false;
            }
            request.scope_run_id = Some(root);
        }
        if request.scope_run_id.is_none() {
            for id in &request.source_ids {
                let source = available
                    .iter()
                    .find(|s| &s.id == id)
                    .ok_or_else(|| Error::new("NOT_FOUND", "来源未登记或不属于三个 Booru 数据湖"))?
                    .clone();
                let read = self.sources.background(
                    ReadClass::NativeQuery,
                    METADATA_MEMORY_BYTES,
                    flag(),
                )?;
                let version = read
                    .query(METADATA_MEMORY_BYTES, false)
                    .read_version(&source, false)?;
                let probe = read.probe(&source)?;
                if probe.revision != version.catalog_revision {
                    return Err(Error::new("SOURCE_CHANGED", "来源正在发布，请重试创建任务"));
                }
                snapshots.push(VisualSourceSnapshot {
                    source,
                    version,
                    total_objects: probe.count.unwrap_or(0),
                    after_row: 0,
                    discovered: 0,
                    complete: false,
                });
            }
        }
        let run = session.store.create_run(&request, &snapshots)?;
        if let Err(error) = self.retain(&run) {
            self.release(&run);
            session
                .store
                .status(&run.id, "failed", Some(&error.message))?;
            return Err(error);
        }
        self.launch(session, run.id.clone(), 0)?;
        Ok(run)
    }
    fn retain(&self, run: &VisualRun) -> Result<()> {
        let available = self.registry.visual_sources()?;
        let mut snapshots = run.sources.clone();
        for snapshot in &mut snapshots {
            let source = available
                .iter()
                .find(|s| s.id == snapshot.source.id && s.kind == snapshot.source.kind)
                .ok_or_else(|| Error::new("SOURCE_DETACHED", "视觉来源已解除关联"))?;
            snapshot.source = source.clone();
            let read =
                self.sources
                    .background(ReadClass::NativeQuery, METADATA_MEMORY_BYTES, flag())?;
            read.query(METADATA_MEMORY_BYTES, false).retain_version(
                &snapshot.source,
                &snapshot.version,
                &lease_id(&run.id, &snapshot.source.id),
                "studio-visual",
                true,
            )?;
        }
        self.ensure()?.store.refresh_sources(&run.id, &snapshots)?;
        Ok(())
    }
    fn release(&self, run: &VisualRun) {
        let available = self.registry.visual_sources().unwrap_or_default();
        for snapshot in &run.sources {
            let result = (|| -> Result<()> {
                let read = self.sources.background(
                    ReadClass::NativeQuery,
                    METADATA_MEMORY_BYTES,
                    flag(),
                )?;
                let source = available
                    .iter()
                    .find(|s| s.id == snapshot.source.id)
                    .unwrap_or(&snapshot.source);
                read.query(METADATA_MEMORY_BYTES, false)
                    .release_version(source, &lease_id(&run.id, &snapshot.source.id))
            })();
            if let Err(error) = result {
                tracing::warn!(run=%run.id,code=error.code,"visual source lease release deferred");
            }
        }
    }
    pub fn action(self: &Arc<Self>, id: &str, action: &str) -> Result<VisualRun> {
        let _gate = self.gate.lock().map_err(lock)?;
        if !matches!(action, "pause" | "resume" | "cancel") {
            return Err(Error::invalid("视觉任务操作须为 pause、resume 或 cancel"));
        }
        let session = self.ensure()?;
        let run = session.store.run(id)?;
        let active = self.active.lock().map_err(lock)?;
        if let Some(active) = active.as_ref() {
            if active.id != id {
                return Err(Error::new("VISUAL_BUSY", "另一项视觉任务仍在运行"));
            }
            if action == "resume" {
                return Err(Error::new("VISUAL_BUSY", "该任务尚未暂停"));
            }
            active
                .action
                .store(if action == "pause" { 1 } else { 2 }, Ordering::Release);
            active.cancelled.store(true, Ordering::Release);
            session.store.status(
                id,
                if action == "pause" {
                    "pausing"
                } else {
                    "cancelling"
                },
                None,
            )?;
        } else {
            drop(active);
            if !matches!(run.status.as_str(), "paused" | "failed") {
                return Err(Error::invalid("只有暂停或失败的视觉任务可恢复或取消"));
            }
            if action == "pause" {
                return Ok(run);
            }
            self.retain(&run)?;
            session.store.status(id, "queued", None)?;
            self.launch(
                session.clone(),
                id.into(),
                if action == "cancel" { 2 } else { 0 },
            )?;
        }
        session.store.run(id)
    }
    fn launch(self: &Arc<Self>, session: Arc<Session>, id: String, initial: u8) -> Result<()> {
        let action = Arc::new(AtomicU8::new(initial));
        let cancelled = flag();
        *self.active.lock().map_err(lock)? = Some(Active {
            id: id.clone(),
            action: action.clone(),
            cancelled: cancelled.clone(),
        });
        let backend = self.clone();
        std::thread::Builder::new()
            .name("studio-visual".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    backend.execute(&session, &id, &action, cancelled)
                }));
                let result = result.unwrap_or_else(|_| Err(lock("视觉调度异常退出，批次可恢复")));
                let control = action.load(Ordering::Acquire);
                let state = if control == 1 {
                    "paused"
                } else if result.is_err() {
                    "failed"
                } else if control == 2 {
                    "cancelled"
                } else if session.store.run(&id).is_ok_and(|r| r.failed_features > 0) {
                    "completed_with_errors"
                } else {
                    "completed"
                };
                let message = result.err().map(|e| e.message);
                let _ = session.store.status(&id, state, message.as_deref());
                if matches!(state, "completed" | "completed_with_errors" | "cancelled")
                    && let Ok(run) = session.store.run(&id)
                {
                    backend.release(&run);
                }
                drop(session);
                if let Ok(mut active) = backend.active.lock() {
                    *active = None;
                }
            })
            .map_err(Error::io)?;
        Ok(())
    }
    fn execute(
        &self,
        session: &Session,
        id: &str,
        action: &AtomicU8,
        cancelled: ReadCancellation,
    ) -> Result<()> {
        // Reconcile the one interrupted batch before any new table write. A pause
        // or cancel applies at its boundary, including after an engine restart.
        if let Some(batch) = session.store.pending_any()? {
            self.batch(session, batch, flag())?;
        }
        if action.load(Ordering::Acquire) != 0 {
            return Ok(());
        }
        self.retain(&session.store.run(id)?)?;
        session.store.status(id, "planning", None)?;
        loop {
            if action.load(Ordering::Acquire) != 0 {
                return Ok(());
            }
            let run = session.store.run(id)?;
            let Some((index, snapshot)) = run.sources.iter().enumerate().find(|(_, s)| !s.complete)
            else {
                break;
            };
            let limit = run
                .request
                .max_objects_per_source
                .map(|n| n.saturating_sub(snapshot.discovered).min(4096) as u32)
                .unwrap_or(4096);
            let page = if let Some(scope) = &run.request.scope_run_id {
                session
                    .store
                    .scope_page(scope, &snapshot.source.id, snapshot.after_row, limit)?
            } else {
                let read = self.sources.background(
                    ReadClass::NativeQuery,
                    METADATA_MEMORY_BYTES,
                    cancelled.clone(),
                )?;
                let page = if run.request.assets.is_empty() {
                    read.visual_media(
                        &snapshot.source,
                        &snapshot.version,
                        snapshot.after_row,
                        limit,
                    )?
                } else {
                    let assets = run
                        .request
                        .assets
                        .iter()
                        .filter(|k| k.source_id == snapshot.source.id)
                        .map(|k| k.asset_id.clone())
                        .collect::<Vec<_>>();
                    read.visual_selected(&snapshot.source, &snapshot.version, &assets)?
                };
                drop(read);
                page
            };
            session.store.discover(&run, index, &page)?;
        }
        session.store.status(id, "running", None)?;
        loop {
            if action.load(Ordering::Acquire) != 0 {
                return Ok(());
            }
            let run = session.store.run(id)?;
            let Some(batch) = session.store.next_batch_with_cancel(&run, &cancelled)? else {
                return Ok(());
            };
            self.batch(session, batch, cancelled.clone())?;
        }
    }
    fn batch(
        &self,
        session: &Session,
        mut batch: VisualBatchRequest,
        cancelled: ReadCancellation,
    ) -> Result<()> {
        let locations = self.registry.visual_sources()?;
        for item in &mut batch.items {
            let source = locations
                .iter()
                .find(|s| s.id == item.source_id)
                .ok_or_else(|| Error::new("SOURCE_DETACHED", "视觉来源已解除关联"))?;
            item.media_root = source
                .media_root
                .clone()
                .ok_or_else(|| Error::invalid("缺少媒体目录"))?;
        }
        let worker = self.worker(session)?;
        let decode_lease = self.resources.acquire(
            ReadRequest {
                class: ReadClass::Decode,
                priority: ReadPriority::Background,
                bytes: DECODE_BATCH_BYTES,
            },
            &cancelled,
        )?;
        let media_lease = self.resources.acquire(
            ReadRequest {
                class: ReadClass::Media,
                priority: ReadPriority::Background,
                bytes: MEDIA_BATCH_BYTES,
            },
            &cancelled,
        )?;
        let prepared = worker.request("prepare", json!(batch));
        drop(media_lease); // inference never monopolizes the archive read slot
        let prepared = prepared?;
        let receipt: VisualBatchReceipt = if !prepared["complete"].is_null() {
            decode(prepared["complete"].clone())?
        } else {
            decode(worker.request("finish", json!({"batch_id":batch.batch_id}))?)?
        };
        drop(decode_lease);
        session.store.publish(&receipt)
    }
    pub fn search(&self, request: VisualSearchRequest) -> Result<VisualSearchResult> {
        if !(1..=100).contains(&request.limit)
            || request.source_ids.len() > 3
            || request.asset.is_some() == request.vector.is_some()
            || request.nprobes.is_some_and(|n| n == 0 || n > 65536)
        {
            return Err(Error::invalid(
                "检索需要一张图片引用或一个向量，返回 1-100 项",
            ));
        }
        let session = self.ensure()?;
        let recipe = session.store.recipe(&request.recipe_id)?;
        if request.vector.as_ref().is_some_and(|v| {
            v.len() != recipe.dimension as usize
                || v.iter().any(|n| !n.is_finite())
                || v.iter().all(|n| *n == 0.0)
        }) {
            return Err(Error::invalid("查询向量维度或数值无效"));
        }
        let version = session
            .store
            .selected_version(&recipe.id, request.snapshot_id.as_deref())?;
        if request.snapshot_id.is_none()
            && !request.source_ids.is_empty()
            && !session.store.projection_ready(&recipe.id)?
        {
            return Err(Error::new(
                "RESULT_NOT_READY",
                "来源关联投影尚未发布；继续视觉任务后可按来源检索",
            ));
        }
        let object_id = request
            .asset
            .as_ref()
            .map(|k| session.store.object(k))
            .transpose()?;
        let source_ids = session.store.source_numbers(&request.source_ids)?;
        let _lease = self.resources.acquire(
            ReadRequest {
                class: ReadClass::NativeQuery,
                priority: ReadPriority::Interactive,
                bytes: METADATA_MEMORY_BYTES,
            },
            &flag(),
        )?;
        let value=self.worker(&session)?.request("search",json!({"recipe":recipe,"version":version,"object_id":object_id,"vector":request.vector,"source_ids":source_ids,"limit":request.limit,"exact":request.exact,"nprobes":request.nprobes.unwrap_or(32)}))?;
        let items = value["items"]
            .as_array()
            .ok_or_else(|| lock("检索响应无效"))?
            .iter()
            .map(|v| {
                let id = v["object_id"]
                    .as_i64()
                    .ok_or_else(|| lock("候选身份无效"))?;
                let distance = v["distance"]
                    .as_f64()
                    .filter(|x| x.is_finite())
                    .ok_or_else(|| lock("候选距离无效"))?;
                Ok((id, Some(distance)))
            })
            .collect::<Result<Vec<_>>>()?;
        let items = session.store.matches(&items, &request.source_ids)?;
        if items.iter().any(|i| i.assets.is_empty()) {
            return Err(Error::new("VISUAL_PROTOCOL", "来源检索投影与目录不一致"));
        }
        Ok(VisualSearchResult {
            recipe_id: recipe.id,
            version,
            method: value["method"].as_str().unwrap_or("unknown").into(),
            score_type: "cosine_distance".into(),
            items,
        })
    }
    pub fn duplicates(
        &self,
        query: Option<&studio_domain::AssetKey>,
        after: i64,
        limit: u32,
        radius: u32,
    ) -> Result<(Vec<VisualMatch>, Option<i64>)> {
        self.ensure()?.store.duplicates(query, after, limit, radius)
    }
    pub fn index(self: &Arc<Self>, recipe_id: &str, partitions: Option<u32>) -> Result<Value> {
        let _gate = self.gate.lock().map_err(lock)?;
        if self.busy() {
            return Err(Error::new(
                "VISUAL_BUSY",
                "索引维护与提取不能同时写同一材料库",
            ));
        }
        let session = self.ensure()?;
        let recipe = session.store.recipe(recipe_id)?;
        session.store.version(recipe_id)?;
        // The HTTP handler calls this on the blocking pool. Reads remain available
        // at the previous published version throughout index construction.
        let id = format!("index-{recipe_id}");
        *self.active.lock().map_err(lock)? = Some(Active {
            id,
            action: Arc::new(AtomicU8::new(0)),
            cancelled: flag(),
        });
        let result = (|| -> Result<Value> {
            if let Some(batch) = session.store.pending_any()? {
                self.batch(&session, batch, flag())?;
            }
            let value = self
                .worker(&session)?
                .request("index", json!({"recipe":recipe,"partitions":partitions}))?;
            let version = value["version"]
                .as_u64()
                .ok_or_else(|| lock("索引版本无效"))?;
            session.store.publish_index(recipe_id, version)?;
            Ok(value)
        })();
        *self.active.lock().map_err(lock)? = None;
        result
    }
    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Ok(active) = self.active.lock()
            && let Some(active) = active.as_ref()
        {
            active.action.store(1, Ordering::Release);
            active.cancelled.store(true, Ordering::Release);
        }
        if let Ok(session) = self.session.lock()
            && let Some(session) = session.as_ref()
            && let Ok(worker) = session.worker.lock()
        {
            let _ = worker.stop();
        }
    }
}
