# Dataset Studio 二次审查：源码核对证据
日期：2026-09-26。对象：用户上传的 `Dataset_Studio-source-20260926-fd42c7e.zip`。
## 范围与证据等级
本附件是定向源码核对，不是全仓库逐行审计。仅有 Studio 源码；没有 Danbooru-Store 源码、正式数据湖、项目运行数据库或原报告链接的 JSON 证据。原报告中的本机容量、耗时、上游 sync 实现不能在这里独立复核。没有 Rust/Cargo 运行环境，未执行工程回归或 HTTP/UI 压测。
`probe_incremental_publish.py` 只读取源码并建立内存合成数据库。运行库为 Python SQLite 3.46.1，不等于 Studio 实际捆绑的 SQLite。新版本优化器可能选择不同计划，因此实验是 SQL 访问路径风险复现，不是生产延迟或实际运行计划的替代。
源码保持原状；摘录中的行号是上传源码内的实际行号。建议项不等于已实现功能。
## E01 — Native DuckDB ownership and runtime gate
文件：`crates/studio-sources/src/duckdb.rs`；SHA-256：`cc320dd305280daa0ed315074ef93f5decbac0e2076424104dc15cadaced3881`。
### 原文件 L376–L421
```text
  376 |     fn with_api(
  377 |         api: Arc<Api>,
  378 |         path: &Path,
  379 |         readonly: bool,
  380 |         budget: Duration,
  381 |         cancelled: Option<Arc<AtomicBool>>,
  382 |         scratch: Option<tempfile::TempDir>,
  383 |         memory_bytes: u64,
  384 |     ) -> Result<Self> {
  385 |         let started = Instant::now();
  386 |         let version = unsafe { CStr::from_ptr((api.library_version)()) }.to_string_lossy();
  387 |         if version != "v1.5.4" {
  388 |             return Err(Error::new(
  389 |                 "METADATA_RUNTIME_UNSUPPORTED",
  390 |                 format!("需要 DuckDB v1.5.4，当前为 {version}"),
  391 |             ));
  392 |         }
  393 |         let path = CString::new(path.to_string_lossy().as_bytes()).map_err(Error::io)?;
  394 |         let bulk = scratch.is_some();
  395 |         let memory = format!("{memory_bytes}B");
  396 |         let temporary = scratch
  397 |             .as_ref()
  398 |             .map(|d| d.path().to_string_lossy().into_owned())
  399 |             .unwrap_or_default();
  400 |         let temporary_limit = format!("{QUERY_TEMP_BYTES}B");
  401 |         let mut config = std::ptr::null_mut();
  402 |         unsafe {
  403 |             if (api.create_config)(&mut config) != 0 {
  404 |                 return Err(Error::new("SOURCE_FORMAT_ERROR", "无法创建 DuckDB 配置"));
  405 |             }
  406 |             for (key, value) in [
  407 |                 (
  408 |                     "access_mode",
  409 |                     if readonly { "READ_ONLY" } else { "READ_WRITE" },
  410 |                 ),
  411 |                 ("threads", if bulk { "2" } else { "1" }),
  412 |                 ("memory_limit", memory.as_str()),
  413 |                 ("temp_directory", temporary.as_str()),
  414 |                 ("max_temp_directory_size", temporary_limit.as_str()),
  415 |                 (
  416 |                     "preserve_insertion_order",
  417 |                     if bulk { "false" } else { "true" },
  418 |                 ),
  419 |                 ("enable_external_access", "false"),
  420 |                 ("autoinstall_known_extensions", "false"),
  421 |                 ("autoload_known_extensions", "false"),
```
### 原文件 L433–L462
```text
  433 |             let mut db = std::ptr::null_mut();
  434 |             let mut message = std::ptr::null_mut();
  435 |             let status = (api.open_ext)(path.as_ptr(), &mut db, config, &mut message);
  436 |             (api.destroy_config)(&mut config);
  437 |             if status != 0 {
  438 |                 let message = if message.is_null() {
  439 |                     "无法打开分析索引".into()
  440 |                 } else {
  441 |                     let s = CStr::from_ptr(message).to_string_lossy().into_owned();
  442 |                     (api.free)(message.cast());
  443 |                     s
  444 |                 };
  445 |                 if !db.is_null() {
  446 |                     (api.close)(&mut db);
  447 |                 }
  448 |                 let lower = message.to_lowercase();
  449 |                 let code = if lower.contains("could not set lock")
  450 |                     || lower.contains("conflicting lock")
  451 |                     || lower.contains("database is locked")
  452 |                     || lower.contains("lock on file")
  453 |                     || lower.contains("file is already open")
  454 |                     || lower.contains("different configuration")
  455 |                 {
  456 |                     "SOURCE_BUSY"
  457 |                 } else if lower.contains("cannot open file") {
  458 |                     "SOURCE_UNAVAILABLE"
  459 |                 } else {
  460 |                     "SOURCE_FORMAT_ERROR"
  461 |                 };
  462 |                 return Err(Error::new(code, message));
```
## E02 — Catalog snapshot, latest-version fence, keyset page
文件：`crates/studio-sources/src/backends/canonical/catalog.rs`；SHA-256：`da9b6f35f3371efddc9524bfe9106428d6c50762238218fcaf2456a376964ffa`。
### 原文件 L117–L141
```text
  117 |         let db = Connection::open_with_flags(
  118 |             dbpath,
  119 |             OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
  120 |         )
  121 |         .map_err(err)?;
  122 |         db.busy_timeout(std::time::Duration::from_millis(400))
  123 |             .map_err(err)?;
  124 |         db.execute_batch("PRAGMA query_only=ON; PRAGMA mmap_size=2147483648; BEGIN;")
  125 |             .map_err(err)?;
  126 |         let seq: u64 = db
  127 |             .query_row("SELECT value FROM state WHERE key='seq'", [], |r| {
  128 |                 unsigned(r, 0)
  129 |             })
  130 |             .map_err(err)?;
  131 |         let revision = format!("catalog-v1:{}:{seq}", current.generation);
  132 |         Ok(Self {
  133 |             db,
  134 |             revision,
  135 |             library_id: library.library_id,
  136 |             root,
  137 |             index,
  138 |             generation_path: generation,
  139 |             generation: current.generation,
  140 |             sequence: seq,
  141 |         })
```
### 原文件 L146–L167
```text
  146 |     pub fn verify_unchanged(&self, source: &Source) -> Result<()> {
  147 |         let pointer: Current =
  148 |             serde_json::from_slice(&fs::read(self.index.join("CURRENT.json")).map_err(Error::io)?)
  149 |                 .map_err(Error::io)?;
  150 |         if pointer.library_id != self.library_id
  151 |             || pointer.generation != self.generation
  152 |             || pointer.index_version != 1
  153 |         {
  154 |             return Err(Error::new(
  155 |                 "SOURCE_CHANGED",
  156 |                 "读取过程中数据源切换了索引版本，请刷新元数据",
  157 |             ));
  158 |         }
  159 |         // A fresh connection supplies an end fence; the original transaction remains open.
  160 |         let current = Self::open(source)?;
  161 |         if current.revision != self.revision || current.library_id != self.library_id {
  162 |             return Err(Error::new(
  163 |                 "SOURCE_CHANGED",
  164 |                 "读取过程中数据源已更新，请刷新元数据",
  165 |             ));
  166 |         }
  167 |         Ok(())
```
### 原文件 L187–L209
```text
  187 |     pub fn page_ordered(
  188 |         &self,
  189 |         source: &Source,
  190 |         after: Option<&str>,
  191 |         limit: usize,
  192 |         revision: Option<&str>,
  193 |         descending: bool,
  194 |     ) -> Result<AssetPage> {
  195 |         if revision.is_some_and(|r| r != self.revision) {
  196 |             return Err(Error::new(
  197 |                 "SOURCE_CHANGED",
  198 |                 "索引版本已变化，请刷新结果范围",
  199 |             ));
  200 |         }
  201 |         let op = if descending { "<" } else { ">" };
  202 |         let direction = if descending { "DESC" } else { "ASC" };
  203 |         let mut stmt=self.db.prepare(&format!("SELECT sha256,length,stored_ext FROM objects WHERE sha256{op}?1 ORDER BY sha256 {direction} LIMIT ?2")).map_err(err)?;
  204 |         let mut items = stmt
  205 |             .query_map(
  206 |                 params![
  207 |                     after.unwrap_or(if descending { "z" } else { "" }),
  208 |                     (limit + 1) as i64
  209 |                 ],
```
## E03 — Matched watermarks are explicitly not historical snapshots
文件：`crates/studio-sources/src/query/mod.rs`；SHA-256：`0baa057b0562a06bf543886d2dfcb2ca4fe408d7fbc7285405fd6b32442fe300`。
### 原文件 L145–L182
```text
  145 |         catalog.verify_unchanged(source)?;
  146 |         Ok(
  147 |             serde_json::json!({"spec":spec,"catalog_revision":catalog.revision,"storage_sql":storage_sql,"storage_plan":storage_plan,"metadata":metadata,"limits":{"native_memory_bytes":QUERY_MEMORY_BYTES,"native_threads":2,"temporary_disk_bytes":QUERY_TEMP_BYTES,"source_budget_seconds":600,"batch_rows":512},"consistency":"read transactions plus matching watermarks and end fences; not a historical snapshot"}),
  148 |         )
  149 |     }
  150 | }
  151 | pub(crate) fn analysis_sequence(db: &Session, catalog: &Catalog) -> Result<String> {
  152 |     db.query("BEGIN TRANSACTION")?;
  153 |     let rows = db.query("SELECT CAST(MAX(seq) AS VARCHAR) FROM applied")?;
  154 |     let sequence = rows
  155 |         .first()
  156 |         .and_then(|r| r.first())
  157 |         .and_then(|v| v.clone())
  158 |         .ok_or_else(|| Error::new("SOURCE_FORMAT_ERROR", "分析索引缺少水位"))?;
  159 |     if sequence != catalog.sequence.to_string() {
  160 |         return Err(Error::new(
  161 |             "SOURCE_CHANGED",
  162 |             "存储与分析索引水位不同，请等待来源更新完成",
  163 |         ));
  164 |     }
  165 |     Ok(sequence)
  166 | }
  167 | fn version(source: &Source, catalog: &Catalog, sequence: Option<String>) -> QuerySourceVersion {
  168 |     QuerySourceVersion {
  169 |         semantics_version: sequence.as_ref().and_then(|_| {
  170 |             crate::profiles::site(&source.kind)?
  171 |                 .normalizer
  172 |                 .map(str::to_owned)
  173 |         }),
  174 |         source_id: source.id.clone(),
  175 |         catalog_revision: catalog.revision.clone(),
  176 |         consistency: if sequence.is_some() {
  177 |             "request_transactions_matched_watermarks".into()
  178 |         } else {
  179 |             "catalog_read_transaction".into()
  180 |         },
  181 |         analysis_sequence: sequence,
  182 |     }
```
## E04 — Metadata still depends on native analysis reads
文件：`crates/studio-sources/src/metadata.rs`；SHA-256：`11525d0565ca9f93bf38f8358a1151e753a0e3cf6901d07b98bcbeb3eb0d366b`。
### 原文件 L191–L265
```text
  191 | struct ReadSession {
  192 |     catalog: Catalog,
  193 |     db: Session,
  194 |     version: ReadVersion,
  195 | }
  196 | impl ReadSession {
  197 |     fn open(
  198 |         reader: &MetadataReader,
  199 |         source: &Source,
  200 |         asset: &str,
  201 |         expected: Option<&str>,
  202 |     ) -> Result<Self> {
  203 |         Self::open_cancelled(
  204 |             reader,
  205 |             source,
  206 |             asset,
  207 |             expected,
  208 |             Arc::new(AtomicBool::new(false)),
  209 |         )
  210 |     }
  211 |     fn open_cancelled(
  212 |         reader: &MetadataReader,
  213 |         source: &Source,
  214 |         asset: &str,
  215 |         expected: Option<&str>,
  216 |         cancelled: ReadCancellation,
  217 |     ) -> Result<Self> {
  218 |         read_cancelled(&cancelled)?;
  219 |         if !crate::profiles::is_canonical(source) {
  220 |             return Err(Error::new(
  221 |                 "METADATA_UNSUPPORTED",
  222 |                 "该来源尚未提供元数据检查",
  223 |             ));
  224 |         }
  225 |         sha(asset)?;
  226 |         let catalog = Catalog::open(source).map_err(source_error)?;
  227 |         catalog.asset(source, asset)?;
  228 |         let db = reader
  229 |             .runtime
  230 |             .open_metadata(&catalog.analysis_path().map_err(source_error)?, cancelled)?;
  231 |         db.query("BEGIN TRANSACTION")?;
  232 |         let applied = db.query("SELECT CAST(MAX(seq) AS VARCHAR) FROM applied")?;
  233 |         let seq = applied
  234 |             .first()
  235 |             .and_then(|r| r[0].as_deref())
  236 |             .ok_or_else(|| Error::new("SOURCE_FORMAT_ERROR", "分析索引缺少水位"))?;
  237 |         if seq != catalog.sequence.to_string() {
  238 |             return Err(Error::new(
  239 |                 "SOURCE_CHANGED",
  240 |                 "存储索引和分析索引水位不同，等待来源更新完成后重试",
  241 |             ));
  242 |         }
  243 |         let version = ReadVersion {
  244 |             token: format!("metadata-v1:{}:{}:{seq}", source.id, catalog.generation),
  245 |             library_id: source.id.clone(),
  246 |             generation: catalog.generation.clone(),
  247 |             catalog_sequence: catalog.sequence.to_string(),
  248 |             analysis_sequence: seq.into(),
  249 |             consistency: "request_transactions_matched_watermarks".into(),
  250 |         };
  251 |         if expected.is_some_and(|v| v != version.token) {
  252 |             return Err(Error::new(
  253 |                 "SOURCE_CHANGED",
  254 |                 "元数据版本已变化，请重新加载当前对象",
  255 |             ));
  256 |         }
  257 |         Ok(Self {
  258 |             catalog,
  259 |             db,
  260 |             version,
  261 |         })
  262 |     }
  263 |     fn finish(&self, source: &Source) -> Result<()> {
  264 |         self.catalog.verify_unchanged(source).map_err(source_error)
  265 |     }
```
## E05 — Identity latest-only readers, gated rebuild, DELETE journal
文件：`crates/studio-sources/src/identity_index.rs`；SHA-256：`fc8621ad960271817ba6c2999592f3f0d56af3680c9cc7d3cfd7501c0dab9be7`。
### 原文件 L140–L178
```text
  140 |     pub fn is_current(&self, source: &Source) -> Result<bool> {
  141 |         let catalog = Catalog::open(source)?;
  142 |         let gate = self.gate(&source.id)?;
  143 |         let Ok(_guard) = gate.try_lock() else {
  144 |             return Ok(false);
  145 |         };
  146 |         Ok(self
  147 |             .open(&source.id)
  148 |             .ok()
  149 |             .and_then(|db| stamp(&db).ok())
  150 |             .is_some_and(|s| s.generation == catalog.generation && s.sequence == catalog.sequence))
  151 |     }
  152 |     pub fn reader(&self, source: &Source) -> Result<IdentityReader> {
  153 |         let catalog = Catalog::open(source)?;
  154 |         let gate = self.gate(&source.id)?;
  155 |         let _guard = gate.try_lock().map_err(|_| preparing())?;
  156 |         let db = self.open(&source.id).map_err(|_| preparing())?;
  157 |         db.execute_batch("BEGIN").map_err(err)?;
  158 |         let stamp = stamp(&db)?;
  159 |         if stamp.generation != catalog.generation || stamp.sequence != catalog.sequence {
  160 |             return Err(preparing());
  161 |         }
  162 |         Ok(IdentityReader {
  163 |             db,
  164 |             generation: stamp.generation,
  165 |             sequence: stamp.sequence,
  166 |         })
  167 |     }
  168 |     pub fn ensure(
  169 |         &self,
  170 |         source: &Source,
  171 |         memory: u64,
  172 |         temporary_root: &Path,
  173 |         cancelled: Arc<AtomicBool>,
  174 |     ) -> Result<()> {
  175 |         let gate = self.gate(&source.id)?;
  176 |         let _guard = gate.lock().map_err(|_| preparing())?;
  177 |         read_cancelled(&cancelled)?;
  178 |         let catalog = Catalog::open(source)?;
```
### 原文件 L240–L258
```text
  240 |         // as source-index storage; the source database remains read-only.
  241 |         let page_size: i64 = db
  242 |             .pragma_query_value(None, "page_size", |r| r.get(0))
  243 |             .map_err(err)?;
  244 |         db.pragma_update(None, "max_page_count", (8_i64 << 30) / page_size)
  245 |             .map_err(err)?;
  246 |         db.execute_batch(&format!("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA cache_size=-{}; PRAGMA temp_store=FILE; CREATE TABLE IF NOT EXISTS identities(sha BLOB NOT NULL,record_id BLOB NOT NULL,post_id INTEGER,PRIMARY KEY(sha,record_id)) WITHOUT ROWID; CREATE TABLE IF NOT EXISTS summaries(sha BLOB PRIMARY KEY,post_count INTEGER NOT NULL,posts_json TEXT NOT NULL) WITHOUT ROWID; CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY,value TEXT); CREATE TEMP TABLE changed(sha BLOB PRIMARY KEY) WITHOUT ROWID;",sqlite_memory/1024)).map_err(err)?;
  247 |         let flag = cancelled.clone();
  248 |         db.progress_handler(1000, Some(move || flag.load(Ordering::Acquire)))
  249 |             .map_err(err)?;
  250 |         let outcome = (|| {
  251 |             let tx = db.transaction().map_err(err)?;
  252 |             let incremental = delta.is_some();
  253 |             if let Some(sql) = &delta {
  254 |                 native.query(&format!("CREATE TEMP TABLE studio_changed AS {sql}"))?;
  255 |                 let mut mark = tx
  256 |                     .prepare("INSERT OR IGNORE INTO changed VALUES (?1)")
  257 |                     .map_err(err)?;
  258 |                 native.stream_ids(
```
## E06 — Single foundational worker and per-revision preparing jobs
文件：`crates/studio-engine/src/source_indexes.rs`；SHA-256：`50559d5c7937ff86e23c7de25b0987bb246dc87edebc9b82ae20598b9930c296`。
### 原文件 L55–L104
```text
   55 |         cache: Arc<CacheControl>,
   56 |         query_directory: PathBuf,
   57 |     ) -> Arc<Self> {
   58 |         Arc::new(Self {
   59 |             indexes: Mutex::new(HashMap::new()),
   60 |             rating_builds: Mutex::new(HashMap::new()),
   61 |             sources,
   62 |             budget,
   63 |             cache,
   64 |             query_directory,
   65 |             browse_index: handles.browse,
   66 |             identity_index: handles.identities,
   67 |             rating_cache: handles.ratings,
   68 |             workers: Arc::new(tokio::sync::Semaphore::new(1)),
   69 |         })
   70 |     }
   71 |     fn schedule(self: &Arc<Self>, work: impl FnOnce() + Send + 'static) {
   72 |         let workers = self.workers.clone();
   73 |         tokio::spawn(async move {
   74 |             if let Ok(permit) = workers.acquire_owned().await {
   75 |                 let _ = tokio::task::spawn_blocking(move || {
   76 |                     let _permit = permit;
   77 |                     work();
   78 |                 })
   79 |                 .await;
   80 |             }
   81 |         });
   82 |     }
   83 |     pub fn ensure_browse_index(
   84 |         &self,
   85 |         source: &Source,
   86 |         cancelled: Arc<AtomicBool>,
   87 |     ) -> Result<Option<BrowseIndexStamp>> {
   88 |         if !self.sources.has(source, |c| c.post_order) || self.browse_index.is_current(source)? {
   89 |             return Ok(None);
   90 |         }
   91 |         let budget = self.budget.wait(&cancelled)?;
   92 |         let _permit = self.sources.background(
   93 |             ReadClass::NativeQuery,
   94 |             budget.memory_bytes,
   95 |             cancelled.clone(),
   96 |         )?;
   97 |         self.browse_index
   98 |             .ensure(
   99 |                 source,
  100 |                 budget.memory_bytes,
  101 |                 &self.query_directory,
  102 |                 cancelled,
  103 |             )
  104 |             .map(Some)
```
### 原文件 L106–L148
```text
  106 |     pub fn prepare_browse_index(
  107 |         self: &Arc<Self>,
  108 |         source: &Source,
  109 |         read: &SourceRead,
  110 |     ) -> Result<bool> {
  111 |         if !self.sources.has(source, |c| c.post_order) || self.browse_index.is_current(source)? {
  112 |             return Ok(true);
  113 |         }
  114 |         let revision = read.probe(source)?.revision;
  115 |         let key = format!("browse:{}:{revision}:v1", source.id);
  116 |         let mut jobs = self
  117 |             .indexes
  118 |             .lock()
  119 |             .map_err(|_| Error::new("INTERNAL_ERROR", "排序准备状态不可用"))?;
  120 |         if let Some((_, error)) = jobs.get(&key) {
  121 |             if error.is_some() {
  122 |                 return Err(jobs
  123 |                     .remove(&key)
  124 |                     .and_then(|(_, e)| e)
  125 |                     .expect("present error"));
  126 |             }
  127 |             return Ok(false);
  128 |         }
  129 |         if jobs.len() >= 32 {
  130 |             return Err(Error::new("READ_BUDGET_EXCEEDED", "等待排序准备的来源过多"));
  131 |         }
  132 |         let cancelled = Arc::new(AtomicBool::new(false));
  133 |         jobs.insert(key.clone(), (cancelled.clone(), None));
  134 |         let runner = self.clone();
  135 |         let source = source.clone();
  136 |         self.schedule(move || {
  137 |             let result = runner.ensure_browse_index(&source, cancelled);
  138 |             if let Ok(mut jobs) = runner.indexes.lock() {
  139 |                 if let Err(error) = result {
  140 |                     if let Some((_, state)) = jobs.get_mut(&key) {
  141 |                         *state = Some(error);
  142 |                     }
  143 |                 } else {
  144 |                     jobs.remove(&key);
  145 |                 }
  146 |             }
  147 |         });
  148 |         Ok(false)
```
## E07 — Heavy-query global admission has one active lease
文件：`crates/studio-engine/src/query_budget.rs`；SHA-256：`444b90fc0b7217f3f7053c03974215791ffecd310487ad8dde766624752a00ac`。
### 原文件 L20–L37
```text
   20 |             return Err(Error::invalid("范围查询内存须为 1 至 64 GiB 的整数"));
   21 |         }
   22 |         Ok(self)
   23 |     }
   24 |     fn bytes(self) -> u64 {
   25 |         u64::from(self.memory_gib) << 30
   26 |     }
   27 | }
   28 | struct State {
   29 |     configured: Config,
   30 |     active: Option<Config>,
   31 | }
   32 | pub struct QueryBudget {
   33 |     path: PathBuf,
   34 |     resources: Arc<ReadCoordinator>,
   35 |     state: Mutex<State>,
   36 |     available: Condvar,
   37 | }
```
### 原文件 L85–L126
```text
   85 |         Ok(QueryBudgetLease {
   86 |             owner: self,
   87 |             memory_bytes,
   88 |         })
   89 |     }
   90 |     pub fn wait(&self, cancelled: &AtomicBool) -> Result<QueryBudgetLease<'_>> {
   91 |         loop {
   92 |             studio_application::read_cancelled(cancelled)?;
   93 |             let mut state = self.state.lock().map_err(|_| lock_error())?;
   94 |             if state.active.is_none() {
   95 |                 let memory_bytes = state.configured.bytes();
   96 |                 self.resources.set_query_memory(memory_bytes)?;
   97 |                 state.active = Some(state.configured);
   98 |                 return Ok(QueryBudgetLease {
   99 |                     owner: self,
  100 |                     memory_bytes,
  101 |                 });
  102 |             }
  103 |             state = self
  104 |                 .available
  105 |                 .wait_timeout(state, Duration::from_millis(50))
  106 |                 .map_err(|_| lock_error())?
  107 |                 .0;
  108 |             drop(state);
  109 |         }
  110 |     }
  111 | }
  112 | fn lock_error() -> Error {
  113 |     Error::new("INTERNAL_ERROR", "查询预算状态不可用")
  114 | }
  115 | pub struct QueryBudgetLease<'a> {
  116 |     owner: &'a QueryBudget,
  117 |     pub memory_bytes: u64,
  118 | }
  119 | impl Drop for QueryBudgetLease<'_> {
  120 |     fn drop(&mut self) {
  121 |         if let Ok(mut state) = self.owner.state.lock() {
  122 |             state.active = None;
  123 |             self.owner.available.notify_all();
  124 |             if let Err(error) = self
  125 |                 .owner
  126 |                 .resources
```
## E08 — Literal tags and same-observation predicate semantics
文件：`crates/studio-sources/src/query/compiler.rs`；SHA-256：`0cb32f4f80729c3eeb02868f30173ea34a5d5f7a3cdb28c8b8c1c95b24e369ee`。
### 原文件 L20–L67
```text
   20 | fn predicate(column: &str, condition: &QueryCondition) -> Result<String> {
   21 |     use QueryOperator::*;
   22 |     if condition.operator == IsMissing {
   23 |         return Ok(format!("{column} IS NULL"));
   24 |     }
   25 |     if condition.operator == IsPresent {
   26 |         return Ok(format!("{column} IS NOT NULL"));
   27 |     }
   28 |     if let Some(QueryValue::TextList(values)) = &condition.value {
   29 |         let list = values
   30 |             .iter()
   31 |             .map(|v| quote(v))
   32 |             .collect::<Vec<_>>()
   33 |             .join(",");
   34 |         return match condition.operator {
   35 |             In => Ok(format!("{column} IN ({list})")),
   36 |             HasAllTags => Ok(format!("list_has_all(string_split({column},' '),[{list}])")),
   37 |             HasAnyTags => Ok(format!("list_has_any(string_split({column},' '),[{list}])")),
   38 |             HasNoTags => Ok(format!(
   39 |                 "NOT list_has_any(string_split({column},' '),[{list}])"
   40 |             )),
   41 |             _ => Err(Error::invalid("该操作不接受集合值")),
   42 |         };
   43 |     }
   44 |     let value = match &condition.value {
   45 |         Some(QueryValue::Text(value)) => quote(value),
   46 |         Some(QueryValue::Integer(value)) => value
   47 |             .parse::<i64>()
   48 |             .map_err(|_| Error::invalid("整数条件无效"))?
   49 |             .to_string(),
   50 |         Some(QueryValue::Boolean(value)) => value.to_string(),
   51 |         Some(QueryValue::TextList(_)) => unreachable!("list handled above"),
   52 |         None => return Err(Error::invalid("条件缺少值")),
   53 |     };
   54 |     if condition.operator == HasTag {
   55 |         return Ok(format!("list_contains(string_split({column},' '),{value})"));
   56 |     }
   57 |     let operator = match condition.operator {
   58 |         Eq => "=",
   59 |         Ne => "!=",
   60 |         Gte => ">=",
   61 |         Lte => "<=",
   62 |         _ => return Err(Error::invalid("条件操作无效")),
   63 |     };
   64 |     Ok(format!("{column} {operator} {value}"))
   65 | }
   66 | 
   67 | pub(crate) fn ranking_predicate(spec: &QuerySpec) -> Result<String> {
```
### 原文件 L189–L228
```text
  189 |     for condition in &spec.conditions {
  190 |         let column = match condition.field.as_str() {
  191 |             "asset.id" => identity,
  192 |             "stored.bytes" | "stored.extension" => continue,
  193 |             "post.id" => "o.post_id",
  194 |             "source.width" => "o.image_width",
  195 |             "source.height" => "o.image_height",
  196 |             "source.extension" => "o.file_ext",
  197 |             "score" => "o.score",
  198 |             "fav_count" => "o.fav_count",
  199 |             "rating" => "o.rating",
  200 |             "tags" => "o.tag_string",
  201 |             "is_deleted" => "o.is_deleted",
  202 |             _ => return Err(Error::new("QUERY_UNSUPPORTED", "字段没有查询实现")),
  203 |         };
  204 |         predicates.push(predicate(column, condition)?);
  205 |     }
  206 |     let predicate = predicates.join(" AND ");
  207 |     if rating_candidates {
  208 |         // The base already establishes the current-post to asset association.
  209 |         // Rejoining the full source identity tables would discard that benefit.
  210 |         return Ok(format!(
  211 |             "SELECT {identity} FROM studio_rating_candidates a JOIN studio_rating_observations o ON o.row_id=a.row_id WHERE {predicate}"
  212 |         ));
  213 |     }
  214 |     let observations = if rating_candidates {
  215 |         "studio_rating_observations"
  216 |     } else {
  217 |         "observations"
  218 |     };
  219 |     // No DISTINCT or global sort here: the project-owned SQLite sink handles both,
  220 |     // so the C API can stream identities instead of materializing the whole result.
  221 |     Ok(match spec.observation_rule {
  222 |         ObservationRule::CurrentPost => format!(
  223 |             "SELECT a.sha256 FROM current_posts cp JOIN assets a ON a.asset_id=cp.asset_id JOIN {observations} o ON o.row_id=cp.row_id WHERE {predicate}"
  224 |         ),
  225 |         ObservationRule::AnyObservation => format!(
  226 |             "SELECT a.sha256 FROM assets a JOIN observations o ON o.post_id=a.post_id WHERE {predicate} UNION ALL SELECT a.sha256 FROM assets a JOIN observations o ON o.observation_id=a.observation_id WHERE (a.post_id IS NULL OR o.post_id IS DISTINCT FROM a.post_id) AND {predicate}"
  227 |         ),
  228 |     })
```
## E09 — Rating-cache string scanning is scoped, not all queries are whole-lake scans
文件：`crates/studio-sources/src/rating_cache.rs`；SHA-256：`7b2f249bc91f39331ad8f3668e2f56320bd5d26f122ded4782cce945c499b04b`。
### 原文件 L48–L88
```text
   48 |         })
   49 |         .filter(|v| !v.is_empty())
   50 | }
   51 | 
   52 | // SQLite instr is case-sensitive. Literal-space token boundaries retain the
   53 | // source query semantics, including NULLs and values containing a space.
   54 | fn tag_predicates(spec: &QuerySpec) -> Option<(String, Vec<String>)> {
   55 |     let mut clauses = Vec::new();
   56 |     let mut values = Vec::new();
   57 |     for condition in &spec.conditions {
   58 |         if condition.field == "rating" {
   59 |             continue;
   60 |         }
   61 |         if condition.field != "tags" {
   62 |             return None;
   63 |         }
   64 |         if condition.operator == QueryOperator::IsMissing {
   65 |             clauses.push("tags IS NULL".to_owned());
   66 |             continue;
   67 |         }
   68 |         if condition.operator == QueryOperator::IsPresent {
   69 |             clauses.push("tags IS NOT NULL".to_owned());
   70 |             continue;
   71 |         }
   72 |         let tags = match (&condition.operator, &condition.value) {
   73 |             (QueryOperator::HasTag, Some(QueryValue::Text(value))) => vec![value.clone()],
   74 |             (
   75 |                 QueryOperator::HasAllTags | QueryOperator::HasAnyTags | QueryOperator::HasNoTags,
   76 |                 Some(QueryValue::TextList(values)),
   77 |             ) => values.clone(),
   78 |             _ => return None,
   79 |         };
   80 |         let mut tests = Vec::new();
   81 |         for tag in tags {
   82 |             if tag.contains(' ') {
   83 |                 tests.push("0".to_owned());
   84 |             } else {
   85 |                 values.push(format!(" {tag} "));
   86 |                 tests.push(format!("instr(' '||tags||' ',?{})>0", values.len()));
   87 |             }
   88 |         }
```
### 原文件 L572–L611
```text
  572 |     pub(crate) fn import(
  573 |         &self,
  574 |         source: &Source,
  575 |         ratings: &[String],
  576 |         native: &Session,
  577 |         generation: &str,
  578 |         sequence: u64,
  579 |         spec: &QuerySpec,
  580 |     ) -> Result<u64> {
  581 |         let (predicate, parameters) =
  582 |             tag_predicates(spec).unwrap_or_else(|| ("1=1".into(), Vec::new()));
  583 |         native.import_candidates(|append| {
  584 |             for rating in ratings {
  585 |                 let gate = self.gate(&source.id, rating)?;
  586 |                 let _guard = gate.lock().map_err(|_| gate_error())?;
  587 |                 let db = self.read(&source.id, rating)?;
  588 |                 db.execute_batch("BEGIN").map_err(err)?;
  589 |                 db.progress_handler(10000, Some(native.cancellation_probe()))
  590 |                     .map_err(err)?;
  591 |                 let entry = stamp(&db, &source.id, rating)?;
  592 |                 if entry.generation != generation || entry.sequence != sequence {
  593 |                     return Err(Error::new(
  594 |                         "SOURCE_CHANGED",
  595 |                         "基础分级缓存版本已变化，请重新计算",
  596 |                     ));
  597 |                 }
  598 |                 // Scan Tag payloads sequentially; let the budgeted native
  599 |                 // relation order matches, avoiding random reads of the Tag table.
  600 |                 let sql = if predicate == "1=1" {
  601 |                     "SELECT row_id,sha FROM members ORDER BY sha,row_id".to_owned()
  602 |                 } else {
  603 |                     format!("SELECT row_id,sha FROM members NOT INDEXED WHERE {predicate}")
  604 |                 };
  605 |                 let mut statement = db.prepare(&sql).map_err(err)?;
  606 |                 let mut rows = statement
  607 |                     .query(rusqlite::params_from_iter(&parameters))
  608 |                     .map_err(err)?;
  609 |                 loop {
  610 |                     let next = rows.next();
  611 |                     native.check_cancelled()?;
```
## E10 — Versioned members already reuse unchanged rows
文件：`crates/studio-storage/src/schema_v6.sql`；SHA-256：`e6dcb66c6b811d42bb3ddcc1d3a176746379b358f4956446ae7d266e03f53961`。
### 原文件 L26–L43
```text
   26 | CREATE TABLE query_member_data (
   27 |     family_id TEXT NOT NULL REFERENCES query_families(id),
   28 |     source_id TEXT NOT NULL REFERENCES sources(id),
   29 |     asset_id TEXT NOT NULL,
   30 |     valid_from INTEGER NOT NULL,
   31 |     valid_until INTEGER,
   32 |     post_id INTEGER,
   33 |     PRIMARY KEY(family_id,source_id,asset_id,valid_from)
   34 | ) WITHOUT ROWID;
   35 | INSERT INTO query_member_data(family_id,source_id,asset_id,valid_from)
   36 | SELECT result_id,source_id,asset_id,1 FROM result_members;
   37 | DROP TABLE result_members;
   38 | CREATE INDEX query_members_post ON query_member_data(family_id,post_id,source_id,asset_id,valid_from);
   39 | CREATE INDEX query_members_expired ON query_member_data(family_id,valid_until) WHERE valid_until IS NOT NULL;
   40 | CREATE VIEW result_members AS
   41 | SELECT r.id AS result_id,m.source_id,m.asset_id
   42 | FROM query_results r JOIN query_member_data m ON m.family_id=r.family_id
   43 | WHERE m.valid_from<=r.member_revision AND (m.valid_until IS NULL OR m.valid_until>r.member_revision);
```
## E11 — Publication: broad delta UPDATE, full family COUNT, writer held across one transaction
文件：`crates/studio-storage/src/query_cache.rs`；SHA-256：`a2bd694b54438d01a6aa611aae37510b69e75d79cdeed80bd30273596f894f15`。
### 原文件 L864–L959
```text
  864 |     pub fn publish_stage_with_budget(
  865 |         &self,
  866 |         pid: &str,
  867 |         rid: &str,
  868 |         stage: &QueryStage,
  869 |         mode: &str,
  870 |         cancelled: &AtomicBool,
  871 |         cache_bytes: u64,
  872 |     ) -> Result<()> {
  873 |         let p = self.handle(pid)?;
  874 |         let mut db = p.db.lock().map_err(lock_error)?;
  875 |         let result = query::read_result(&db, pid, rid)?;
  876 |         if result.state != ResultState::Running {
  877 |             return Err(Error::new("CANCELLED", "结果构建已停止"));
  878 |         }
  879 |         let (family, revision): (String, i64) = db
  880 |             .query_row(
  881 |                 "SELECT family_id,member_revision FROM query_results WHERE id=?1",
  882 |                 [rid],
  883 |                 |r| Ok((r.get(0)?, r.get(1)?)),
  884 |             )
  885 |             .map_err(db_error)?;
  886 |         // Only the project is written in these transactions; staging is sealed.
  887 |         db.execute(
  888 |             "ATTACH DATABASE ?1 AS query_stage",
  889 |             [stage.file.path().to_string_lossy().as_ref()],
  890 |         )
  891 |         .map_err(db_error)?;
  892 |         let old_cache: i64 = db
  893 |             .query_row("PRAGMA cache_size", [], |r| r.get(0))
  894 |             .map_err(db_error)?;
  895 |         db.execute_batch(&format!(
  896 |             "PRAGMA cache_size=-{}",
  897 |             cache_bytes.clamp(64 << 20, 4 << 30) / 1024
  898 |         ))
  899 |         .map_err(db_error)?;
  900 |         let outcome = (|| {
  901 |             let mut changed = 0u64;
  902 |             let mut inserted = 0u64;
  903 |             let tx = db.transaction().map_err(db_error)?;
  904 |             for source in &result.spec.source_ids {
  905 |                 studio_application::read_cancelled(cancelled)?;
  906 |                 let affected = if stage.full_sources.contains(source) {
  907 |                     "1=1"
  908 |                 } else {
  909 |                     "EXISTS(SELECT 1 FROM query_stage.affected a WHERE a.source_id=m.source_id AND a.asset_id=m.asset_id)"
  910 |                 };
  911 |                 changed+=tx.execute(&format!("UPDATE query_member_data AS m SET valid_until=?3 WHERE family_id=?1 AND source_id=?2 AND valid_until IS NULL AND ({affected}) AND NOT EXISTS(SELECT 1 FROM query_stage.matches s WHERE s.source_id=m.source_id AND s.asset_id=m.asset_id AND s.post_id IS m.post_id)"),params![family,source,revision]).map_err(db_error)? as u64;
  912 |             }
  913 |             let mut after = (String::new(), String::new());
  914 |             loop {
  915 |                 studio_application::read_cancelled(cancelled)?;
  916 |                 let last = {
  917 |                     let mut stmt=tx.prepare("SELECT source_id,asset_id FROM query_stage.matches WHERE (source_id,asset_id)>(?1,?2) ORDER BY source_id,asset_id LIMIT 32768").map_err(db_error)?;
  918 |                     stmt.query_map(params![after.0, after.1], |r| {
  919 |                         Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
  920 |                     })
  921 |                     .map_err(db_error)?
  922 |                     .last()
  923 |                     .transpose()
  924 |                     .map_err(db_error)?
  925 |                 };
  926 |                 let Some(last) = last else { break };
  927 |                 let added=tx.execute("INSERT INTO query_member_data(family_id,source_id,asset_id,valid_from,post_id) SELECT ?1,s.source_id,s.asset_id,?2,s.post_id FROM query_stage.matches s WHERE (s.source_id,s.asset_id)>(?3,?4) AND (s.source_id,s.asset_id)<=(?5,?6) AND NOT EXISTS(SELECT 1 FROM query_member_data m WHERE m.family_id=?1 AND m.source_id=s.source_id AND m.asset_id=s.asset_id AND m.valid_until IS NULL) ORDER BY s.source_id,s.asset_id",params![family,revision,after.0,after.1,last.0,last.1]).map_err(db_error)? as u64;
  928 |                 changed += added;
  929 |                 inserted += added;
  930 |                 after = last;
  931 |             }
  932 |             studio_application::read_cancelled(cancelled)?;
  933 |             let count:i64=tx.query_row("SELECT count(*) FROM query_member_data WHERE family_id=?1 AND valid_from<=?2 AND (valid_until IS NULL OR valid_until>?2)",params![family,revision],|r|r.get(0)).map_err(db_error)?;
  934 |             tx.execute(
  935 |                 "UPDATE query_families SET stored_members=stored_members+?2 WHERE id=?1",
  936 |                 params![family, inserted as i64],
  937 |             )
  938 |             .map_err(db_error)?;
  939 |             tx.execute("UPDATE query_results SET count=?2,processed=?3,cache_mode=?4,evaluated_count=?5,changed_members=?6,post_ready=?7 WHERE id=?1",params![rid,count,stage.processed as i64,mode,stage.evaluated as i64,changed as i64,stage.post_ready]).map_err(db_error)?;
  940 |             // An evaluated refresh can also prove that membership AND stored
  941 |             // post associations did not change. Keep the old revision/index.
  942 |             if changed == 0 && revision > 1 {
  943 |                 tx.execute("UPDATE query_results SET member_revision=(SELECT latest_revision FROM query_families WHERE id=?2) WHERE id=?1", params![rid, family]).map_err(db_error)?;
  944 |             }
  945 |             if changed > 0 {
  946 |                 touch_sizes(&tx)?;
  947 |             }
  948 |             tx.commit().map_err(db_error)?;
  949 |             Ok(())
  950 |         })();
  951 |         let detached = db
  952 |             .execute_batch("DETACH DATABASE query_stage;")
  953 |             .map_err(db_error);
  954 |         let restored = db
  955 |             .execute_batch(&format!(
  956 |                 "PRAGMA cache_size={old_cache}; PRAGMA shrink_memory;"
  957 |             ))
  958 |             .map_err(db_error);
  959 |         outcome.and(detached).and(restored)
```
## E12 — Ready publication and rollback exist: no claim of dirty visible results
文件：`crates/studio-storage/src/query.rs`；SHA-256：`1882b024251d7c6abb9f4ac4251b75ad27f073c8212f9e2a578d2e7f40d046f3`。
### 原文件 L531–L568
```text
  531 |     pub fn finish_result(&self, pid: &str, id: &str, error: Option<&Error>) -> Result<QueryResult> {
  532 |         let p = self.handle(pid)?;
  533 |         let mut db = p.db.lock().map_err(lock_error)?;
  534 |         let tx = db.transaction().map_err(db_error)?;
  535 |         // The final mutable-selection fence and publication share one transaction.
  536 |         let result = read_result(&tx, pid, id)?;
  537 |         if result.state != ResultState::Running {
  538 |             clear_input_references(&tx, id)?;
  539 |             crate::query_cache::rollback_revision(&tx, id)?;
  540 |             tx.commit().map_err(db_error)?;
  541 |             return Ok(result);
  542 |         }
  543 |         let scope_error = if error.is_none() && result.state == ResultState::Running {
  544 |             validate_input(&tx, pid, &result.spec).err()
  545 |         } else {
  546 |             None
  547 |         };
  548 |         let (status, message) = match error.or(scope_error.as_ref()) {
  549 |             None => ("ready", None),
  550 |             Some(e) => (
  551 |                 match e.code {
  552 |                     "INTERRUPTED" => "interrupted",
  553 |                     "CANCELLED" => "cancelled",
  554 |                     _ => "failed",
  555 |                 },
  556 |                 Some(e.to_string()),
  557 |             ),
  558 |         };
  559 |         if tx.execute("UPDATE query_results SET status=?2,error=?3,count=CASE WHEN ?2='ready' THEN count ELSE NULL END WHERE id=?1 AND status='running'",params![id,status,message]).map_err(db_error)?>0 { event(&tx,"result.changed",id)?; }
  560 |         clear_input_references(&tx, id)?;
  561 |         if status == "ready" {
  562 |             tx.execute("UPDATE query_families SET latest_revision=(SELECT member_revision FROM query_results WHERE id=?1),prune_pending=1,latest_result_id=?1,latest_count=(SELECT COALESCE(count,0) FROM query_results WHERE id=?1),post_ready=(SELECT post_ready FROM query_results WHERE id=?1),touched_at=CAST(?2 AS INTEGER) WHERE id=(SELECT family_id FROM query_results WHERE id=?1)",params![id,now()]).map_err(db_error)?;
  563 |         } else {
  564 |             crate::query_cache::rollback_revision(&tx, id)?;
  565 |         }
  566 |         let result = read_result(&tx, pid, id)?;
  567 |         tx.commit().map_err(db_error)?;
  568 |         Ok(result)
```
## E13 — Independent committed read pool
文件：`crates/studio-storage/src/read_pool.rs`；SHA-256：`720d2b048a0da4960ddabc84f3f3c9d2b14e590457f0149e3e608e5e30babec7`。
### 原文件 L1–L57
```text
    1 | use crate::*;
    2 | use std::ops::Deref;
    3 | 
    4 | /// Short WAL snapshots are independent of the single project writer. At most
    5 | /// four idle connections (4 MiB page cache each) are retained; API read budgets
    6 | /// continue to govern concurrent expensive work.
    7 | #[derive(Default)]
    8 | pub(super) struct ReadPool {
    9 |     idle: Mutex<Vec<Connection>>,
   10 | }
   11 | pub(super) struct ReadGuard<'a> {
   12 |     db: Option<Connection>,
   13 |     pool: &'a ReadPool,
   14 | }
   15 | impl ReadPool {
   16 |     pub fn read(&self, path: &Path) -> Result<ReadGuard<'_>> {
   17 |         let cached = self.idle.lock().map_err(lock_error)?.pop();
   18 |         let db = if let Some(db) = cached {
   19 |             db
   20 |         } else {
   21 |             let db = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
   22 |                 .map_err(db_error)?;
   23 |             db.busy_timeout(std::time::Duration::from_secs(3))
   24 |                 .map_err(db_error)?;
   25 |             db.execute_batch("PRAGMA query_only=ON; PRAGMA cache_size=-4096;")
   26 |                 .map_err(db_error)?;
   27 |             db
   28 |         };
   29 |         db.execute_batch("BEGIN").map_err(db_error)?;
   30 |         Ok(ReadGuard {
   31 |             db: Some(db),
   32 |             pool: self,
   33 |         })
   34 |     }
   35 | }
   36 | impl Deref for ReadGuard<'_> {
   37 |     type Target = Connection;
   38 |     fn deref(&self) -> &Connection {
   39 |         self.db.as_ref().expect("live read snapshot")
   40 |     }
   41 | }
   42 | impl Drop for ReadGuard<'_> {
   43 |     fn drop(&mut self) {
   44 |         if let Some(db) = self.db.take() {
   45 |             if db.execute_batch("ROLLBACK").is_err() {
   46 |                 return;
   47 |             }
   48 |             if let Ok(mut idle) = self.pool.idle.lock()
   49 |                 && idle.len() < 4
   50 |             {
   51 |                 idle.push(db);
   52 |             }
   53 |         }
   54 |     }
   55 | }
   56 | 
   57 | #[cfg(test)]
```
## E14 — Draft reads still share the writer mutex
文件：`crates/studio-storage/src/drafts.rs`；SHA-256：`ffc04283cd8c5a79c487b117fe571ae9e3b3f7a06698199d12bc66745e7f668f`。
### 原文件 L62–L84
```text
   62 | impl DraftRepository for SqliteStore {
   63 |     fn draft(&self, pid: &str, module: &str, instance: &str) -> Result<Option<Draft>> {
   64 |         key(module)?;
   65 |         key(instance)?;
   66 |         let p = self.handle(pid)?;
   67 |         read_draft(&*p.db.lock().map_err(lock_error)?, pid, module, instance)
   68 |     }
   69 |     fn save_draft(
   70 |         &self,
   71 |         pid: &str,
   72 |         module: &str,
   73 |         instance: &str,
   74 |         request: SaveDraft,
   75 |     ) -> Result<Draft> {
   76 |         key(module)?;
   77 |         key(instance)?;
   78 |         let json = payload(&request)?;
   79 |         let p = self.handle(pid)?;
   80 |         let mut db = p.db.lock().map_err(lock_error)?;
   81 |         let tx = db.transaction().map_err(db_error)?;
   82 |         let old = read_draft(&tx, pid, module, instance)?;
   83 |         if old.as_ref().map_or(0, |d| d.revision) != request.expected_revision {
   84 |             return Err(Error::new("REVISION_CONFLICT", "草稿已被另一次编辑修改"));
```
## E15 — Preview content identity and shared mutex; Drop may evict
文件：`crates/studio-resources/src/cache.rs`；SHA-256：`508dd7d6e053a88b87feecc9eba3624d89f040161e4c0fd203335750f77b498e`。
### 原文件 L23–L39
```text
   23 | pub fn preview_key(source: &Source, asset_id: &str, content_version: &str, edge: u32) -> String {
   24 |     // Library/content identities survive relinking and append-only index generations.
   25 |     // Change renderer/encoder tags when output semantics or decoder policy changes.
   26 |     hex::encode(Sha256::digest(
   27 |         serde_json::to_vec(&(
   28 |             "preview-key-v1",
   29 |             &source.kind,
   30 |             &source.id,
   31 |             asset_id,
   32 |             content_version,
   33 |             edge.clamp(96, 1600),
   34 |             "fit-no-crop-v1",
   35 |             "image-0.25-v1",
   36 |             "jpeg-q86-v1",
   37 |         ))
   38 |         .expect("cache key is serializable"),
   39 |     ))
```
### 原文件 L59–L94
```text
   59 | struct State {
   60 |     db: Connection,
   61 |     objects: PathBuf,
   62 |     _lock: File,
   63 |     pins: HashMap<String, usize>,
   64 |     metrics: CacheMetrics,
   65 |     scan: Option<ReadDir>,
   66 |     settings: PathBuf,
   67 | }
   68 | #[derive(Clone)]
   69 | pub struct PreviewCache {
   70 |     inner: Arc<Mutex<State>>,
   71 | }
   72 | pub struct CachedPreview {
   73 |     pub bytes: Vec<u8>,
   74 |     pub verified_ms: u64,
   75 |     pub pin: CachePin,
   76 | }
   77 | pub struct CachePin {
   78 |     inner: Arc<Mutex<State>>,
   79 |     key: String,
   80 | }
   81 | impl Drop for CachePin {
   82 |     fn drop(&mut self) {
   83 |         if let Ok(mut state) = self.inner.lock() {
   84 |             if let Some(count) = state.pins.get_mut(&self.key) {
   85 |                 *count -= 1;
   86 |                 if *count == 0 {
   87 |                     state.pins.remove(&self.key);
   88 |                 }
   89 |             }
   90 |             // A quota change may have been waiting for this last reader.
   91 |             let clear = state.metrics.clear_pending;
   92 |             let _ = state.trim(32, clear);
   93 |         }
   94 |     }
```
### 原文件 L109–L115
```text
  109 | fn open_index(path: &Path) -> rusqlite::Result<(Connection, u64, u64, u64)> {
  110 |     let db = Connection::open(path)?;
  111 |     db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
  112 |         CREATE TABLE IF NOT EXISTS entries(key TEXT PRIMARY KEY,bytes INTEGER NOT NULL,sha256 TEXT NOT NULL,verified_ms INTEGER NOT NULL,used_ms INTEGER NOT NULL) WITHOUT ROWID;
  113 |         CREATE INDEX IF NOT EXISTS cache_lru ON entries(used_ms,key);
  114 |         CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,value INTEGER NOT NULL);
  115 |         INSERT OR IGNORE INTO settings VALUES('quota_bytes',2147483648);")?;
```
## E16 — Preview read/write/hash/fsync/trim share one mutex
文件：`crates/studio-resources/src/cache.rs`；SHA-256：`508dd7d6e053a88b87feecc9eba3624d89f040161e4c0fd203335750f77b498e`。
### 原文件 L291–L390
```text
  291 |     pub fn get(&self, key: &str, verified_online: bool) -> Result<Option<CachedPreview>> {
  292 |         if !valid_key(key) {
  293 |             return Err(Error::invalid("缓存身份无效"));
  294 |         }
  295 |         let mut state = self.inner.lock().map_err(cache_error)?;
  296 |         let entry: Option<(u64, String, u64)> = state
  297 |             .db
  298 |             .query_row(
  299 |                 "SELECT bytes,sha256,verified_ms FROM entries WHERE key=?1",
  300 |                 [key],
  301 |                 |r| Ok((unsigned(r, 0)?, r.get(1)?, unsigned(r, 2)?)),
  302 |             )
  303 |             .optional()
  304 |             .map_err(cache_error)?;
  305 |         let Some((size, hash, mut verified_ms)) = entry else {
  306 |             state.metrics.misses += 1;
  307 |             return Ok(None);
  308 |         };
  309 |         let path = state.file(key);
  310 |         let bytes = if size <= MAX_ENTRY && path.canonicalize().ok().as_ref() == Some(&path) {
  311 |             fs::metadata(&path)
  312 |                 .ok()
  313 |                 .filter(|m| m.is_file() && m.len() == size)
  314 |                 .and_then(|_| fs::read(&path).ok())
  315 |         } else {
  316 |             None
  317 |         };
  318 |         let Some(bytes) = bytes.filter(|b| hex::encode(Sha256::digest(b)) == hash) else {
  319 |             state.metrics.corrupt += 1;
  320 |             state.metrics.misses += 1;
  321 |             state.remove(key)?;
  322 |             return Ok(None);
  323 |         };
  324 |         if verified_online {
  325 |             verified_ms = epoch_ms();
  326 |         }
  327 |         state
  328 |             .db
  329 |             .execute(
  330 |                 "UPDATE entries SET used_ms=?2,verified_ms=?3 WHERE key=?1",
  331 |                 params![key, epoch_ms() as i64, verified_ms as i64],
  332 |             )
  333 |             .map_err(cache_error)?;
  334 |         state.metrics.hits += 1;
  335 |         state.metrics.read_bytes += bytes.len() as u64;
  336 |         *state.pins.entry(key.into()).or_default() += 1;
  337 |         Ok(Some(CachedPreview {
  338 |             bytes,
  339 |             verified_ms,
  340 |             pin: CachePin {
  341 |                 inner: self.inner.clone(),
  342 |                 key: key.into(),
  343 |             },
  344 |         }))
  345 |     }
  346 |     pub fn put(&self, key: &str, bytes: &[u8]) -> Result<Option<CachePin>> {
  347 |         if !valid_key(key) || bytes.len() as u64 > MAX_ENTRY {
  348 |             return Err(Error::invalid("缩略图缓存材料超出限制"));
  349 |         }
  350 |         let mut state = self.inner.lock().map_err(cache_error)?;
  351 |         state.ensure_objects()?;
  352 |         if bytes.len() as u64 > state.metrics.quota_bytes || state.metrics.clear_pending {
  353 |             return Ok(None);
  354 |         }
  355 |         if state.pins.contains_key(key) {
  356 |             return Ok(None);
  357 |         }
  358 |         // Write data first. A crash before the index commit leaves an orphan which
  359 |         // bounded maintenance removes; the index never claims a partial file.
  360 |         let mut file = tempfile::Builder::new()
  361 |             .prefix("partial-")
  362 |             .tempfile_in(&state.objects)
  363 |             .map_err(cache_error)?;
  364 |         file.write_all(bytes).map_err(cache_error)?;
  365 |         file.as_file().sync_all().map_err(cache_error)?;
  366 |         let path = state.file(key);
  367 |         if path
  368 |             .symlink_metadata()
  369 |             .is_ok_and(|m| m.file_type().is_symlink())
  370 |         {
  371 |             return Err(Error::new("CACHE_PATH_INVALID", "缓存材料不能是链接"));
  372 |         }
  373 |         file.persist(&path).map_err(cache_error)?;
  374 |         let prior: Option<u64> = state
  375 |             .db
  376 |             .query_row("SELECT bytes FROM entries WHERE key=?1", [key], |r| {
  377 |                 unsigned(r, 0)
  378 |             })
  379 |             .optional()
  380 |             .map_err(cache_error)?;
  381 |         state.db.execute("INSERT INTO entries VALUES(?1,?2,?3,?4,?4) ON CONFLICT(key) DO UPDATE SET bytes=excluded.bytes,sha256=excluded.sha256,verified_ms=excluded.verified_ms,used_ms=excluded.used_ms", params![key,bytes.len() as i64,hex::encode(Sha256::digest(bytes)),epoch_ms() as i64]).map_err(cache_error)?;
  382 |         state.metrics.bytes = state.metrics.bytes - prior.unwrap_or(0) + bytes.len() as u64;
  383 |         state.metrics.entries += u64::from(prior.is_none());
  384 |         state.metrics.writes += 1;
  385 |         *state.pins.entry(key.into()).or_default() += 1;
  386 |         state.trim(128, false)?;
  387 |         Ok(Some(CachePin {
  388 |             inner: self.inner.clone(),
  389 |             key: key.into(),
  390 |         }))
```
## E17 — Media lookup and bounded TAR-offset reads already exist
文件：`crates/studio-sources/src/backends/canonical/catalog.rs`；SHA-256：`da9b6f35f3371efddc9524bfe9106428d6c50762238218fcaf2456a376964ffa`。
### 原文件 L303–L341
```text
  303 |     pub fn read_many(&self, inputs: &[MediaInput]) -> Result<MediaBatch> {
  304 |         if inputs.len() > 16 {
  305 |             return Err(Error::invalid("媒体批次最多 16 个对象"));
  306 |         }
  307 |         let mut items: Vec<Option<Result<Media>>> = (0..inputs.len()).map(|_| None).collect();
  308 |         let mut locations = Vec::<(usize, String, u64, u64, String)>::new();
  309 |         let mut total = 0u64;
  310 |         for (i, input) in inputs.iter().enumerate() {
  311 |             let location: Option<(String,u64,u64,String)> = self.db.query_row(
  312 |                 "SELECT pack_path,offset,length,COALESCE(stored_ext,'') FROM objects WHERE sha256=?1",
  313 |                 [&input.asset_id], |r| Ok((r.get(0)?,unsigned(r,1)?,unsigned(r,2)?,r.get(3)?))
  314 |             ).optional().map_err(err)?;
  315 |             match location {
  316 |                 Some((_, _, length, _)) if length > input.byte_limit => {
  317 |                     items[i] = Some(Err(Error::new(
  318 |                         "READ_BUDGET_EXCEEDED",
  319 |                         "索引中的图片长度超过已准入的读取预算",
  320 |                     )));
  321 |                 }
  322 |                 Some((pack, offset, length, extension)) if length <= 64 << 20 => {
  323 |                     total += length;
  324 |                     locations.push((i, pack, offset, length, extension));
  325 |                 }
  326 |                 Some(_) => {
  327 |                     items[i] = Some(Err(Error::new(
  328 |                         "MEDIA_TOO_LARGE",
  329 |                         "预览支持最大 64 MiB 的单张图片",
  330 |                     )))
  331 |                 }
  332 |                 None => items[i] = Some(Err(Error::new("NOT_FOUND", "图片对象不存在"))),
  333 |             }
  334 |         }
  335 |         if total > 64 << 20 {
  336 |             return Err(Error::new(
  337 |                 "READ_BUDGET_EXCEEDED",
  338 |                 "一个媒体批次最多读取 64 MiB",
  339 |             ));
  340 |         }
  341 |         locations.sort_by(|a, b| (&a.1, a.2, a.0).cmp(&(&b.1, b.2, b.0)));
```
### 原文件 L344–L381
```text
  344 |         for (index, pack, offset, length, extension) in locations {
  345 |             let input = &inputs[index];
  346 |             let mut read = 0u64;
  347 |             let result = (|| -> Result<Media> {
  348 |                 input.check()?;
  349 |                 if current.as_ref().is_none_or(|(name, _)| name != &pack) {
  350 |                     let path = child(&self.root, &pack)?;
  351 |                     input.check()?;
  352 |                     current = Some((pack.clone(), File::open(path).map_err(Error::io)?));
  353 |                     stats.opens += 1;
  354 |                 }
  355 |                 let file = &mut current.as_mut().expect("opened pack").1;
  356 |                 if offset
  357 |                     .checked_add(length)
  358 |                     .is_none_or(|end| end > file.metadata().map(|m| m.len()).unwrap_or(0))
  359 |                 {
  360 |                     return Err(Error::new("SOURCE_CORRUPT", "图片位置超出数据包边界"));
  361 |                 }
  362 |                 input.check()?;
  363 |                 file.seek(SeekFrom::Start(offset)).map_err(Error::io)?;
  364 |                 stats.seeks += 1;
  365 |                 let mut bytes = vec![0; length as usize];
  366 |                 for chunk in bytes.chunks_mut(64 * 1024) {
  367 |                     let mut filled = 0;
  368 |                     while filled < chunk.len() {
  369 |                         input.check()?;
  370 |                         let count = file.read(&mut chunk[filled..]).map_err(Error::io)?;
  371 |                         if count == 0 {
  372 |                             return Err(Error::new("SOURCE_CORRUPT", "数据包读取提前结束"));
  373 |                         }
  374 |                         filled += count;
  375 |                         read += count as u64;
  376 |                         stats.bytes += count as u64;
  377 |                     }
  378 |                 }
  379 |                 input.check()?;
  380 |                 if hex::encode(Sha256::digest(&bytes)) != input.asset_id {
  381 |                     return Err(Error::new("SOURCE_CORRUPT", "图片内容校验失败"));
```
## E18 — Multi-source version fences and sequential query scheduling
文件：`crates/studio-engine/src/query_jobs.rs`；SHA-256：`9c3220003da828d295a9af26141c1287a4569c63d715b8d47e6ff78ad75430b3`。
### 原文件 L83–L109
```text
   83 |     pub fn validate_result(&self, store: &SqliteStore, result: &QueryResult) -> Result<()> {
   84 |         if result.state != ResultState::Ready {
   85 |             return Err(Error::new(
   86 |                 match result.state {
   87 |                     ResultState::Failed => "SCOPE_SORT_FAILED",
   88 |                     ResultState::Cancelled => "CANCELLED",
   89 |                     ResultState::Interrupted => "INTERRUPTED",
   90 |                     _ => "RESULT_NOT_READY",
   91 |                 },
   92 |                 result
   93 |                     .error
   94 |                     .clone()
   95 |                     .unwrap_or_else(|| "结果尚未完整构建或已释放".into()),
   96 |             ));
   97 |         }
   98 |         store.validate_derived(&result.project_id, &result.spec)?;
   99 |         if !result.spec.uses_only_fixed_project_data()
  100 |             && self.versions(store, &result.project_id, &result.spec)? != result.source_versions
  101 |         {
  102 |             return Err(Error::new(
  103 |                 "SOURCE_CHANGED",
  104 |                 "来源版本已变化；已有结果保留固定成员，请重新计算后使用查询范围",
  105 |             ));
  106 |         }
  107 |         Ok(())
  108 |     }
  109 |     fn build(
```
### 原文件 L299–L374
```text
  299 |         }
  300 |         if cancelled.load(Ordering::Acquire) {
  301 |             return Err(Error::new("CANCELLED", "构建已取消"));
  302 |         }
  303 |         // Multi-source builds have per-source transactions; check every source again
  304 |         // before publication. This fence is explicitly not a historical snapshot.
  305 |         if !fixed
  306 |             && self.versions(store, &result.project_id, &result.spec)? != result.source_versions
  307 |         {
  308 |             return Err(Error::new(
  309 |                 "SOURCE_CHANGED",
  310 |                 "构建期间来源已更新，请重新计算",
  311 |             ));
  312 |         }
  313 |         let stage = stage.into_inner();
  314 |         stage.seal()?;
  315 |         let (ratings, candidates) = reader.rating_usage()?;
  316 |         store.query_basis_usage(&result.project_id, &result.id, &ratings, candidates)?;
  317 |         store.query_build_phase(&result.project_id, &result.id, "publishing")?;
  318 |         store.publish_stage_with_budget(
  319 |             &result.project_id,
  320 |             &result.id,
  321 |             &stage,
  322 |             mode,
  323 |             &cancelled,
  324 |             (budget.memory_bytes / 2).min(4 << 30),
  325 |         )?;
  326 |         if !fixed
  327 |             && self.versions(store, &result.project_id, &result.spec)? != result.source_versions
  328 |         {
  329 |             return Err(Error::new(
  330 |                 "SOURCE_CHANGED",
  331 |                 "发布查询期间来源已更新，请刷新后重试",
  332 |             ));
  333 |         }
  334 |         Ok(())
  335 |     }
  336 | }
  337 | pub async fn scheduler(store: Arc<SqliteStore>, runner: Arc<QueryRunner>) {
  338 |     while !runner.stopping.load(Ordering::Acquire) {
  339 |         let s = store.clone();
  340 |         let next = tokio::task::spawn_blocking(move || -> Result<Option<QueryResult>> {
  341 |             s.reap_closed()?;
  342 |             for id in s.owned_projects()? {
  343 |                 match s.next_result(&id) {
  344 |                     Ok(Some(result)) => return Ok(Some(result)),
  345 |                     Err(error) => tracing::warn!(project_id=%id,%error,"query scheduling failed"),
  346 |                     _ => {}
  347 |                 }
  348 |             }
  349 |             Ok(None)
  350 |         })
  351 |         .await;
  352 |         match next {
  353 |             Ok(Ok(Some(result))) => {
  354 |                 let cancelled = Arc::new(AtomicBool::new(false));
  355 |                 if let Ok(mut running) = runner.running.lock() {
  356 |                     running.insert(result.id.clone(), cancelled.clone());
  357 |                 }
  358 |                 let s = store.clone();
  359 |                 let r = runner.clone();
  360 |                 let query = result.clone();
  361 |                 let completed=tokio::task::spawn_blocking(move||->Result<()> {
  362 |                     // Pin through status publication, including user cancellation.
  363 |                     let _lease=s.operation_lease(&query.project_id)?;
  364 |                     if !s.start_result(&query.project_id,&query.id)? { return Ok(()); }
  365 |                     let started=std::time::Instant::now();
  366 |                     let built=r.build(&s,&query,cancelled);
  367 |                     let error=if r.stopping.load(Ordering::Acquire) { Some(Error::new("INTERRUPTED","引擎已停止，结果需要重新计算")) } else { built.err() };
  368 |                     if let Some(e)=&error { tracing::warn!(result_id=%query.id,code=e.code,message=%e.message,"query build stopped"); }
  369 |                     let _cache_gate=r.cache.lock()?;
  370 |                     let published=s.finish_result(&query.project_id,&query.id,error.as_ref())?;
  371 |                     if published.state==ResultState::Ready {r.cache.recent(&query.project_id,&query.id);}
  372 |                     r.cache.track_committed(&s,&query.project_id);
  373 |                     r.cache.requested.store(true,Ordering::Release);
  374 |                     tracing::info!(result_id=%query.id,state=?published.state,count=?published.count,elapsed_ms=started.elapsed().as_millis(),"query build finished");
```
## E19 — Browser resets stale cursor; next-page prefetch already exists
文件：`apps/desktop/src/features/browser/Browser.tsx`；SHA-256：`7d5ca1696423aa2aaa12a88749d72d67da4b6a91777f458335c7ede3fc0d6328`。
### 原文件 L451–L469
```text
  451 |   useEffect(() => {
  452 |     if (!query.data || query.isFetching || query.data.preparing) return;
  453 |     const expected = restoreCheck.current;
  454 |     restoreCheck.current = null;
  455 |     if (
  456 |       expected &&
  457 |       (expected.version !== query.data.revision ||
  458 |         (expected.anchor &&
  459 |           query.data.items[0] &&
  460 |           assetIdentity(expected.anchor) !==
  461 |             assetIdentity(query.data.items[0].key)))
  462 |     ) {
  463 |       setHistory(initialHistory());
  464 |       setScrollTop(0);
  465 |       savedScroll.current = 0;
  466 |       setNotice("来源已更新，已返回第一页。");
  467 |       return;
  468 |     }
  469 |     if (
```
### 原文件 L509–L566
```text
  509 |   useEffect(() => {
  510 |     if (
  511 |       query.error &&
  512 |       cursor &&
  513 |       "code" in query.error &&
  514 |       ["SOURCE_CHANGED", "INVALID_INPUT", "REVISION_CONFLICT"].includes(
  515 |         String(query.error.code),
  516 |       )
  517 |     ) {
  518 |       restoreCheck.current = null;
  519 |       setHistory(initialHistory());
  520 |       setScrollTop(0);
  521 |       savedScroll.current = 0;
  522 |       setNotice("范围已更新，已返回第一页。");
  523 |     }
  524 |   }, [query.error, cursor]);
  525 |   useLayoutEffect(() => {
  526 |     if (view !== "grid" || !query.data) return;
  527 |     if (scrollRef.current) scrollRef.current.scrollTop = savedScroll.current;
  528 |     const focused = gridRef.current?.querySelector<HTMLElement>(
  529 |       ".asset-card.focused",
  530 |     );
  531 |     focused?.scrollIntoView({ block: "nearest" });
  532 |   }, [view, query.data?.revision, cursor]);
  533 |   useEffect(
  534 |     () => () => {
  535 |       if (scrollTimer.current) clearTimeout(scrollTimer.current);
  536 |     },
  537 |     [],
  538 |   );
  539 |   useEffect(() => {
  540 |     const next = query.data?.next_cursor;
  541 |     if (
  542 |       !next ||
  543 |       query.isFetching ||
  544 |       query.data?.preparing ||
  545 |       query.error ||
  546 |       view !== "grid"
  547 |     )
  548 |       return;
  549 |     const abort = new AbortController();
  550 |     const timer = setTimeout(() => {
  551 |       const options = {
  552 |         cursor: next,
  553 |         limit: 4,
  554 |         order,
  555 |         signal: abort.signal,
  556 |         priority: "prefetch" as const,
  557 |       };
  558 |       const page =
  559 |         ranked.active && ranked.target
  560 |           ? client.ranking.browseAssets(
  561 |               projectId,
  562 |               {
  563 |                 scope: ranked.target,
  564 |                 ...(ranked.settings.sort !== "saved" &&
  565 |                 ranked.settings.sort !== "off"
  566 |                   ? { order: ranked.settings.sort }
```
## E20 — Yandere/Gelbooru capability profiles are tied to HF normalizers
文件：`crates/studio-sources/src/profiles.rs`；SHA-256：`c8958cb4b831a67e218c1978c3f1c14b12af505085b23e390ab2c68d57c4b519`。
### 原文件 L1–L54
```text
    1 | //! Storage-independent site semantics. Normalized lake values are never remapped here.
    2 | use studio_domain::*;
    3 | 
    4 | #[derive(Clone, Copy)]
    5 | pub struct SiteProfile {
    6 |     pub kind: &'static str,
    7 |     pub name: &'static str,
    8 |     pub normalizer: Option<&'static str>,
    9 |     pub absent_fields: &'static [&'static str],
   10 | }
   11 | pub const SITES: [SiteProfile; 3] = [
   12 |     SiteProfile {
   13 |         kind: "danbooru",
   14 |         name: "Danbooru",
   15 |         normalizer: None,
   16 |         absent_fields: &[],
   17 |     },
   18 |     SiteProfile {
   19 |         kind: "yandere",
   20 |         name: "Yandere",
   21 |         normalizer: Some("hf_yandere_v1"),
   22 |         absent_fields: &[
   23 |             "fav_count",
   24 |             "pixiv_id",
   25 |             "is_deleted",
   26 |             "is_banned",
   27 |             "is_pending",
   28 |             "is_flagged",
   29 |             "tag_string_general",
   30 |             "tag_string_artist",
   31 |             "tag_string_copyright",
   32 |             "tag_string_meta",
   33 |         ],
   34 |     },
   35 |     SiteProfile {
   36 |         kind: "gelbooru",
   37 |         name: "Gelbooru",
   38 |         normalizer: Some("hf_gelbooru_v1"),
   39 |         absent_fields: &[
   40 |             "fav_count",
   41 |             "pixiv_id",
   42 |             "is_deleted",
   43 |             "is_banned",
   44 |             "is_pending",
   45 |             "is_flagged",
   46 |             "tag_string_general",
   47 |             "tag_string_artist",
   48 |             "tag_string_copyright",
   49 |             "tag_string_meta",
   50 |             "file_ext",
   51 |             "file_size",
   52 |         ],
   53 |     },
   54 | ];
```
### 原文件 L79–L103
```text
   79 |     let profile =
   80 |         site(kind).ok_or_else(|| Error::new("SOURCE_FORMAT_UNSUPPORTED", "未注册的数据源类型"))?;
   81 |     let mut projections = vec!["origin_width_v1".into(), "origin_groups_v1".into()];
   82 |     if profile.kind == "danbooru" {
   83 |         projections.extend(["danbooru_ranking_v1".into(), "danbooru_ranking_v2".into()]);
   84 |     }
   85 |     Ok(SourceDescriptor {
   86 |         version: 1,
   87 |         backend_id: "canonical_lake_v1".into(),
   88 |         display_name: profile.name.into(),
   89 |         site_id: Some(kind.into()),
   90 |         semantics_version: profile.normalizer.unwrap_or("danbooru-v1").into(),
   91 |         capabilities: SourceCapabilities {
   92 |             browse: true,
   93 |             media: true,
   94 |             metadata: true,
   95 |             query: true,
   96 |             post_order: true,
   97 |             relink: true,
   98 |             raw_metadata: true,
   99 |             incremental: true,
  100 |             stored_dimensions: profile.normalizer.is_some(),
  101 |         },
  102 |         projections,
  103 |     })
```
## E21 — Browse incremental update and final publication
文件：`crates/studio-sources/src/browse_index.rs`；SHA-256：`cb899821a77fac4b20f7bb853505f1ca1203ffc49573eb44b9cc5600e32bd376`。
### 原文件 L344–L355
```text
  344 |             "SELECT sha256||':'||COALESCE(CAST(MIN(post_id) AS VARCHAR),'') FROM assets WHERE sha256 IN (SELECT sha256 FROM studio_changed) GROUP BY sha256 ORDER BY sha256".to_string()
  345 |         } else {
  346 |             "SELECT sha256||':'||COALESCE(CAST(MIN(post_id) AS VARCHAR),'') FROM assets WHERE sha256 IS NOT NULL GROUP BY sha256 ORDER BY sha256".to_string()
  347 |         };
  348 |         {
  349 |             let mut insert = tx
  350 |                 .prepare("UPDATE objects SET post_id=?2 WHERE sha=?1 AND post_id IS NOT ?2")
  351 |                 .map_err(err)?;
  352 |             native.stream_strings(&sql, 128, &mut |rows| {
  353 |                 studio_application::read_cancelled(&cancelled)?;
  354 |                 for text in rows {
  355 |                     let (sha, post) = text
```
### 原文件 L375–L432
```text
  375 |         tx.execute_batch("CREATE INDEX IF NOT EXISTS objects_post ON objects(post_id,sha)")
  376 |             .map_err(err)?;
  377 |         if let Some(anchor) = next_anchor {
  378 |             tx.execute(
  379 |                 "INSERT OR REPLACE INTO anchors VALUES (?1,?2,?3)",
  380 |                 params![anchor.sequence as i64, anchor.generation, anchor.batch_id],
  381 |             )
  382 |             .map_err(err)?;
  383 |         }
  384 |         let count: i64 = tx
  385 |             .query_row("SELECT count(*) FROM objects", [], |r| r.get(0))
  386 |             .map_err(err)?;
  387 |         if !incremental {
  388 |             refreshed = count as u64;
  389 |         }
  390 |         for (key, value) in [
  391 |             ("format", "1".to_string()),
  392 |             ("generation", catalog.generation.clone()),
  393 |             ("seq", catalog.sequence.to_string()),
  394 |             ("count", count.to_string()),
  395 |             ("refreshed", refreshed.to_string()),
  396 |             ("incremental", incremental.to_string()),
  397 |         ] {
  398 |             tx.execute(
  399 |                 "INSERT OR REPLACE INTO state VALUES (?1,?2)",
  400 |                 params![key, value],
  401 |             )
  402 |             .map_err(err)?;
  403 |         }
  404 |         catalog.verify_unchanged(source)?;
  405 |         studio_application::read_cancelled(&cancelled)?;
  406 |         tx.commit().map_err(err)?;
  407 |         let mut result = stamp(&db)?;
  408 |         drop(db);
  409 |         drop(existing);
  410 |         if let Some(temp) = temporary {
  411 |             temp.as_file().sync_all().map_err(Error::io)?;
  412 |             temp.persist(&target).map_err(Error::io)?;
  413 |         }
  414 |         result.bytes = std::fs::metadata(&target).map_err(Error::io)?.len();
  415 |         Ok(result)
  416 |     }
  417 |     pub fn post_ids(&self, source: &Source, keys: &[AssetKey]) -> Result<Vec<Option<i64>>> {
  418 |         self.reader(source)?.post_ids(keys)
  419 |     }
  420 |     pub fn reader(&self, source: &Source) -> Result<BrowseIndexReader> {
  421 |         let catalog = Catalog::open(source)?;
  422 |         let gate = self.gate(&source.id)?;
  423 |         let _guard = gate
  424 |             .lock()
  425 |             .map_err(|_| Error::new("INTERNAL_ERROR", "浏览索引锁不可用"))?;
  426 |         let db = self.open(&source.id)?;
  427 |         db.execute_batch("BEGIN").map_err(err)?;
  428 |         let s = stamp(&db)?;
  429 |         if s.generation != catalog.generation || s.sequence != catalog.sequence {
  430 |             return Err(Error::new("SOURCE_CHANGED", "浏览索引需要刷新"));
  431 |         }
  432 |         Ok(BrowseIndexReader { db, stamp: s })
```
## E22 — Project migration backup has a deadline; no large production backup tested
文件：`crates/studio-storage/src/migrations.rs`；SHA-256：`11f8a8876e2c24e353bfe3a18da5b2b0e2e193c94132b02c7425f3ef27967858`。
### 原文件 L92–L114
```text
   92 |         .canonicalize()
   93 |         .map_err(Error::io)?
   94 |         .starts_with(directory.canonicalize().map_err(Error::io)?)
   95 |     {
   96 |         return Err(Error::invalid("项目备份目录必须位于项目内"));
   97 |     }
   98 |     fs::create_dir(&backup_dir).map_err(Error::io)?;
   99 |     let result = (|| {
  100 |         let mut target = Connection::open(backup_dir.join("project.sqlite")).map_err(db_error)?;
  101 |         {
  102 |             let backup = Backup::new(db, &mut target).map_err(db_error)?;
  103 |             let deadline = Instant::now() + Duration::from_secs(30);
  104 |             loop {
  105 |                 if Instant::now() >= deadline {
  106 |                     return Err(Error::new("BACKUP_TIMEOUT", "项目备份超时，未开始升级"));
  107 |                 }
  108 |                 match backup.step(128).map_err(db_error)? {
  109 |                     StepResult::Done => break,
  110 |                     StepResult::Busy | StepResult::Locked => {
  111 |                         std::thread::sleep(Duration::from_millis(25))
  112 |                     }
  113 |                     _ => {}
  114 |                 }
```
## 实验概况
固定一个 affected asset，并让重新评估确认其成员及 post_id 未变化。扩大同一 family/source 的既有成员数量。原始 UPDATE 和 COUNT 均直接取自上传源码；使用原成员表和两个索引，并执行 ANALYZE。候选 UPDATE 只适用于增量分支，以 affected asset ID 驱动索引寻址。
| 成员数 | 原始 UPDATE 约 VM 指令数 | 原始 COUNT 约 VM 指令数 | 候选 UPDATE |
|---:|---:|---:|---:|
| 10,000 | 130,000 | 70,000 | <100 |
| 100,000 | 1,300,000 | 700,000 | <100 |
| 500,000 | 6,500,000 | 3,500,000 | <100 |

计量粒度 100 条 VM 指令。数值不是毫秒，不代表真实 SSD/HDD 或 Studio 性能。小型语义样例覆盖删除、新增、post_id 改变、NULL、未受影响资产、另一来源和旧成员修订；原始与候选 UPDATE 结果相同。它不是完整属性测试、并发测试或可直接合入的补丁。
原计划主表搜索前缀为 `(family_id,source_id)`，候选计划为 `(family_id,source_id,asset_id)`；完整 EXPLAIN、SQL 和结果见 `probe_results.json`。
计数建议：维护基线计数，并在排除并发同族发布、保证 affected 完整和去重的前提下使用 `新计数 = 旧计数 - 关闭区间数 + 新增区间数`。首次构建应按封存后的去重结果计数。不能把当前合并的 changed_members 直接当净变化量。
## 原始源码一致性
与上传 ZIP 逐文件比较：检查 594 个文件，差异 0 个。
