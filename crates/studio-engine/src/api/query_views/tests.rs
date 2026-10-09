use super::*;
use domain::{
    AssetKey, QueryHit, QueryOrder, QuerySourceVersion, QuerySpec, Source, SourceQueryPage,
};
use std::sync::{
    Barrier,
    atomic::{AtomicBool, Ordering},
};
use studio_application::{ReadLease, ReadResources, SourceReadContext};

// Real default budgets, with a test watchdog so a reintroduced nested acquire
// fails the test instead of hanging the test process forever.
struct Watchdog(studio_resources::ReadCoordinator);
impl ReadResources for Watchdog {
    fn acquire(
        &self,
        r: domain::ReadRequest,
        c: &AtomicBool,
    ) -> domain::Result<Box<dyn ReadLease>> {
        self.acquire_until(r, c, None)
    }
    fn acquire_until(
        &self,
        r: domain::ReadRequest,
        c: &AtomicBool,
        d: Option<Instant>,
    ) -> domain::Result<Box<dyn ReadLease>> {
        let bound = Instant::now() + Duration::from_secs(2);
        self.0
            .acquire_until(r, c, Some(d.map_or(bound, |v| v.min(bound))))
    }
    fn metrics(&self) -> Vec<domain::ReadMetrics> {
        self.0.metrics()
    }
}

fn context() -> RequestReadContext {
    RequestReadContext {
        cancelled: Arc::new(AtomicBool::new(false)),
        priority: domain::ReadPriority::Interactive,
    }
}
fn spec(ids: Vec<String>, order: QueryOrder) -> QuerySpec {
    QuerySpec {
        version: 3,
        source_ids: ids,
        conditions: vec![],
        observation_rule: domain::ObservationRule::CurrentPost,
        order,
        input_scope: None,
    }
}
fn source(n: usize) -> Source {
    Source {
        id: format!("00000000-0000-4000-8000-{n:012}"),
        name: format!("lake-{n}"),
        kind: "demo".into(),
        index_root: None,
        media_root: None,
    }
}
fn version(source: &Source) -> QuerySourceVersion {
    QuerySourceVersion {
        source_id: source.id.clone(),
        catalog_revision: "fixture-1".into(),
        analysis_sequence: None,
        consistency: "immutable_demo".into(),
        semantics_version: None,
    }
}
fn cursor(spec: &QuerySpec) -> Cursor {
    Cursor {
        project_id: domain::new_id(),
        result_id: domain::new_id(),
        order: spec.order,
        afters: BTreeMap::new(),
        exhausted: BTreeSet::new(),
        totals: BTreeMap::new(),
        scanned: 0,
    }
}
fn streams(sources: &[Source], cursor: &Cursor) -> Vec<Stream> {
    sources
        .iter()
        .map(|s| Stream {
            source: s.clone(),
            version: version(s),
            after: cursor.afters.get(&s.id).cloned(),
            buffer: VecDeque::new(),
            next: None,
            done: cursor.exhausted.contains(&s.id),
            remaining_scan: 0,
        })
        .collect()
}

// Bounded ordered windows with sparse matches and repeated post IDs across
// lakes. Resuming by asset identity exercises the same adapter contract as the
// online SQL reader; no merge/cursor logic is replaced by the fixture.
struct Pages {
    rows: BTreeMap<String, Vec<QueryHit>>,
    sparse: bool,
    slow: Option<String>,
    scanned_per_window: u64,
}
impl Pages {
    fn new(sources: &[Source], order: QueryOrder, sparse: bool) -> Self {
        let rows = sources
            .iter()
            .map(|source| {
                let mut rows = (0..29)
                    .map(|n| QueryHit {
                        key: AssetKey {
                            source_id: source.id.clone(),
                            asset_id: format!("{n:064x}"),
                        },
                        post_id: (n % 7 != 0).then_some((n % 5) as i128),
                    })
                    .collect::<Vec<_>>();
                rows.sort_by(|a, b| compare(order, a, b));
                (source.id.clone(), rows)
            })
            .collect();
        Self {
            rows,
            sparse,
            slow: None,
            scanned_per_window: 0,
        }
    }
    fn matches(&self, hit: &QueryHit) -> bool {
        !self.sparse || u64::from_str_radix(&hit.key.asset_id, 16).unwrap() % 13 == 2
    }
    fn expected(&self, order: QueryOrder) -> Vec<AssetKey> {
        let mut hits = self
            .rows
            .values()
            .flatten()
            .filter(|h| self.matches(h))
            .cloned()
            .collect::<Vec<_>>();
        hits.sort_by(|a, b| compare(order, a, b));
        hits.into_iter().map(|h| h.key).collect()
    }
}
impl QueryAdapter for Pages {
    fn query_page(
        &self,
        source: &Source,
        _: &QuerySpec,
        _: &QuerySourceVersion,
        after: Option<&str>,
        limit: usize,
        _: Arc<AtomicBool>,
    ) -> domain::Result<SourceQueryPage> {
        if self.slow.as_ref() == Some(&source.id) {
            std::thread::sleep(Duration::from_millis(220));
        }
        let rows = &self.rows[&source.id];
        let begin = after.map_or(0, |id| {
            rows.iter().position(|h| h.key.asset_id == id).unwrap() + 1
        });
        let end = (begin + if self.sparse { 3 } else { limit }).min(rows.len());
        let window = &rows[begin..end];
        Ok(SourceQueryPage {
            hits: window.iter().filter(|h| self.matches(h)).cloned().collect(),
            next: (end < rows.len()).then(|| rows[end - 1].key.asset_id.clone()),
            scanned: self.scanned_per_window.max(window.len() as u64),
            total_objects: rows.len() as u64,
        })
    }
    fn fields(&self, _: &Source) -> domain::Result<domain::FieldDirectory> {
        unreachable!()
    }
    fn query_version(&self, _: &Source, _: &QuerySpec) -> domain::Result<QuerySourceVersion> {
        unreachable!()
    }
    fn execute_query(
        &self,
        _: &Source,
        _: &QuerySpec,
        _: &QuerySourceVersion,
        _: Arc<AtomicBool>,
        _: &mut dyn FnMut(&[AssetKey], u64) -> domain::Result<()>,
    ) -> domain::Result<()> {
        unreachable!()
    }
}

fn collect_pages(
    sources: &[Source],
    spec: &QuerySpec,
    pages: &Pages,
    limit: usize,
) -> Vec<AssetKey> {
    let mut cursor = cursor(spec);
    let context = SourceReadContext::new(
        Arc::new(AtomicBool::new(false)),
        domain::ReadPriority::Interactive,
    );
    let mut result = vec![];
    for _ in 0..512 {
        let old = serde_json::to_vec(&cursor).unwrap();
        let old_scanned = cursor.scanned;
        let mut streams = streams(sources, &cursor);
        let keys = merge_page(&mut cursor, &mut streams, spec, limit, &context, pages).unwrap();
        // Actual committed scan offsets / exhausted streams must move, not just
        // a counter or a token nonce. Unconsumed hits must never be skipped.
        let previous: Cursor = serde_json::from_slice(&old).unwrap();
        if keys.is_empty() && streams.iter().any(|s| !s.done || !s.buffer.is_empty()) {
            assert!(cursor.afters != previous.afters || cursor.exhausted != previous.exhausted);
        }
        assert!(cursor.scanned >= old_scanned);
        assert!(keys.len() <= limit);
        result.extend(keys);
        // Round-trip every page, dropping all buffers like an engine restart.
        let encoded = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&cursor).unwrap());
        assert!(encoded.len() <= 8192);
        cursor = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(encoded).unwrap()).unwrap();
        if streams.iter().all(|s| s.done && s.buffer.is_empty()) {
            return result;
        }
    }
    panic!("pagination did not terminate");
}

#[test]
fn merge_keeps_exact_order_and_members_across_sparse_windows_and_work_yields() {
    for count in [1, 2, 3, 8] {
        let sources = (1..=count).map(source).collect::<Vec<_>>();
        for order in [
            QueryOrder::AssetKeyAsc,
            QueryOrder::AssetKeyDesc,
            QueryOrder::PostIdAsc,
            QueryOrder::PostIdDesc,
        ] {
            let spec = spec(sources.iter().map(|s| s.id.clone()).collect(), order);
            for sparse in [false, true] {
                let mut pages = Pages::new(&sources, order, sparse);
                // Exercise the 8192-work boundary with all eight head pages
                // returning the maximum online scan size.
                pages.scanned_per_window = 2048;
                for limit in [1, 4, 128] {
                    assert_eq!(
                        collect_pages(&sources, &spec, &pages, limit),
                        pages.expected(order)
                    );
                }
            }
        }
    }
}

#[test]
fn slow_first_or_middle_lake_cannot_repeat_an_empty_checkpoint() {
    let sources = (1..=3).map(source).collect::<Vec<_>>();
    let spec = spec(
        sources.iter().map(|s| s.id.clone()).collect(),
        QueryOrder::PostIdDesc,
    );
    for slow in [0, 1] {
        let mut pages = Pages::new(&sources, spec.order, false);
        // One head per source and a full tail make this test fast while crossing
        // the real 200 ms time budget on every request that needs the slow lake.
        for rows in pages.rows.values_mut() {
            rows.truncate(2);
        }
        pages.slow = Some(sources[slow].id.clone());
        let mut checkpoint = cursor(&spec);
        let mut current = streams(&sources, &checkpoint);
        let context = SourceReadContext::new(
            Arc::new(AtomicBool::new(false)),
            domain::ReadPriority::Interactive,
        );
        assert_eq!(
            merge_page(&mut checkpoint, &mut current, &spec, 4, &context, &pages)
                .unwrap()
                .len(),
            4,
            "already buffered hits should fill the page after the soft time budget"
        );
        assert_eq!(
            collect_pages(&sources, &spec, &pages, 4),
            pages.expected(spec.order)
        );
    }
}

fn fixture(
    online: bool,
) -> (
    tempfile::TempDir,
    AppState,
    String,
    domain::QueryResult,
    Vec<Source>,
) {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-runs");
    std::fs::create_dir_all(&base).unwrap();
    let directory = tempfile::Builder::new()
        .prefix("query-views-")
        .tempdir_in(base)
        .unwrap();
    let root = directory.path();
    let store = Arc::new(SqliteStore::new(root.join("registry")).unwrap());
    let coordinator = Arc::new(studio_resources::ReadCoordinator::default());
    let budget = Arc::new(
        crate::query_budget::QueryBudget::open(root.join("budget.json"), coordinator.clone())
            .unwrap(),
    );
    let resources: Arc<dyn ReadResources> = Arc::new(Watchdog((*coordinator).clone()));
    let cache =
        Arc::new(crate::query_cache::CacheControl::open(root.join("cache.json"), 1024).unwrap());
    let sources = crate::sources::SourceService::new(
        studio_sources::registry(root.join("query"), None, None).unwrap(),
        resources.clone(),
    );
    let indexes = crate::source_indexes::SourceIndexService::new(
        crate::source_indexes::SourceIndexHandles::new(root.join("browse")),
        sources.clone(),
        budget.clone(),
        cache.clone(),
        root.join("query"),
    );
    let previews = crate::previews::PreviewService::new(
        resources.clone(),
        studio_resources::PreviewCache::open(&root.join("preview")).unwrap(),
        sources.clone(),
    );
    let queries = Arc::new(crate::query_jobs::QueryRunner::new(
        root.join("query"),
        budget,
        cache,
        indexes,
        sources.clone(),
        root.join("browse"),
    ));
    let credentials = Arc::new(studio_llm::credentials::CredentialVault::new(
        root.join("credentials"),
    ));
    let backend = Arc::new(studio_llm::RemoteLlm::new(credentials.clone()));
    let state = AppState {
        lake_updates: Arc::new(crate::lake_updates::Backend::new(root.join("registry"))),
        aesthetic: Arc::new(crate::aesthetic::Runner::default()),
        aesthetic_analysis: Arc::new(crate::aesthetic::analysis::Runner::default()),
        llm: Arc::new(studio_application::llm::LlmService::new(
            store.clone(),
            backend,
            credentials,
        )),
        llm_invocations: Default::default(),
        store: store.clone(),
        connection: EngineConnection {
            api_version: API_VERSION,
            instance_id: domain::new_id(),
            pid: std::process::id(),
            endpoint: "http://127.0.0.1:0".into(),
            token: "fixture".into(),
        },
        resources,
        previews,
        sources,
        queries,
        ranking_reads: Arc::new(crate::ranking_reads::RankingReadCache::default()),
        shutdown: tokio::sync::watch::channel(false).0,
    };
    let project = store
        .create("Query view fixture", Some(root.join("projects")))
        .unwrap();
    let mut sources = vec![];
    let mut versions = vec![];
    for n in 1..=2 {
        let mut source = source(n);
        if online {
            source.kind = "danbooru".into();
            let index = root.join(format!("index-{n}"));
            std::fs::create_dir_all(&index).unwrap();
            std::fs::write(index.join("ONLINE.json"), serde_json::to_vec(&serde_json::json!({"library_id":source.id,"schema_version":2,"generation":"fixture","file":"online.sqlite","site":"danbooru"})).unwrap()).unwrap();
            let db = rusqlite::Connection::open(index.join("online.sqlite")).unwrap();
            db.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
            db.execute_batch(studio_sources::online::SCHEMA).unwrap();
            for (key, value) in [
                ("library_id", source.id.as_str()),
                ("generation", "fixture"),
                ("served_seq", "1"),
                ("min_seq", "1"),
            ] {
                db.execute("INSERT INTO online_state VALUES(?1,?2)", [key, value])
                    .unwrap();
            }
            db.execute(
                "INSERT INTO publications VALUES(1,'baseline','now',0,0,0)",
                [],
            )
            .unwrap();
            source.index_root = Some(index);
        }
        store.attach(&project.id, source.clone()).unwrap();
        let read = state.sources.inspect().unwrap();
        versions.push(
            read.query(domain::METADATA_MEMORY_BYTES, false)
                .read_version(&source, false)
                .unwrap(),
        );
        sources.push(source);
    }
    let spec = spec(
        sources.iter().map(|s| s.id.clone()).collect(),
        QueryOrder::AssetKeyAsc,
    );
    let result = store
        .create_snapshot_result(&project.id, spec, versions, true)
        .unwrap();
    (directory, state, project.id, result, sources)
}
fn assert_idle(state: &AppState) {
    for m in state.resources.metrics() {
        assert_eq!((m.active, m.queued, m.reserved_bytes), (0, 0, 0));
    }
}

#[tokio::test]
async fn cancelled_release_admission_leaves_the_result_owned_and_retryable() {
    let (_dir, state, pid, result, _) = fixture(true);
    let cancelled = context();
    cancelled.cancelled.store(true, Ordering::Release);
    assert!(
        query::release(
            State(state.clone()),
            Extension(cancelled),
            Path((pid.clone(), result.id.clone()))
        )
        .await
        .is_err()
    );
    assert_eq!(
        state.store.query_result(&pid, &result.id).unwrap().state,
        domain::ResultState::Ready
    );
    let released = query::release(
        State(state.clone()),
        Extension(context()),
        Path((pid.clone(), result.id.clone())),
    )
    .await
    .map_err(|error| error.0)
    .unwrap();
    assert_eq!(released.0.id, result.id);
    assert_eq!(
        state.store.query_result(&pid, &result.id).unwrap().state,
        domain::ResultState::Released
    );
    assert_idle(&state);
}

#[test]
fn saturated_admissions_can_create_retain_validate_capture_and_clean_up() {
    let (_dir, state, pid, base, _) = fixture(true);
    let reads = (0..4)
        .map(|_| read_permit(&state, domain::ReadClass::Index, &context()).unwrap())
        .collect::<Vec<_>>();
    let metrics = state.resources.metrics();
    let index = metrics
        .iter()
        .find(|m| m.budget.class == domain::ReadClass::Index)
        .unwrap();
    assert_eq!((index.active, index.reserved_bytes), (4, 128 << 20));
    let barrier = Arc::new(Barrier::new(4));
    let threads = reads
        .into_iter()
        .map(|read| {
            let state = state.clone();
            let pid = pid.clone();
            let base = base.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                let result = state
                    .store
                    .create_snapshot_result(
                        &pid,
                        base.spec.clone(),
                        base.source_versions.clone(),
                        true,
                    )
                    .unwrap();
                retain_created(&state, &pid, &result, false, &read).unwrap();
                retain(&state, &pid, &result, false, &read).unwrap();
                state
                    .queries
                    .validate_result(&state.store, &result, &read)
                    .unwrap();
                let scope = domain::ScopeRef {
                    project_id: pid.clone(),
                    target: domain::ScopeTarget::QueryResult {
                        result_id: result.id.clone(),
                    },
                };
                let (_, versions) = query::source_capture(&state, &pid, &scope, &read).unwrap();
                assert_eq!(versions, result.source_versions);
                let job = state
                    .store
                    .submit_scope_job(
                        &pid,
                        &domain::new_id(),
                        &scope,
                        0,
                        Some((result.spec.clone(), versions)),
                    )
                    .unwrap();
                retain_job(&state, &pid, &job, &read).unwrap();
                // Both cancellation and deadline expiry must fail closed without
                // blocking the independent release scope under this admission.
                read.context.cancelled.store(true, Ordering::Release);
                assert_eq!(
                    retain(&state, &pid, &result, false, &read)
                        .unwrap_err()
                        .code,
                    "CANCELLED"
                );
                release_versions(&state, &pid, &result, &read).unwrap();
                let mut read = read;
                read.context.cancelled.store(false, Ordering::Release);
                read.context.deadline = Some(Instant::now());
                assert_eq!(
                    retain(&state, &pid, &result, false, &read)
                        .unwrap_err()
                        .code,
                    "SOURCE_TIMEOUT"
                );
            })
        })
        .collect::<Vec<_>>();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_idle(&state);
}

#[test]
fn partial_creation_cancellation_releases_prior_leases_under_same_admission() {
    let (_dir, state, pid, result, sources) = fixture(true);
    let first = rusqlite::Connection::open(
        sources[0]
            .index_root
            .as_ref()
            .unwrap()
            .join("online.sqlite"),
    )
    .unwrap();
    let second = rusqlite::Connection::open(
        sources[1]
            .index_root
            .as_ref()
            .unwrap()
            .join("online.sqlite"),
    )
    .unwrap();
    second.execute_batch("BEGIN IMMEDIATE").unwrap();
    let context = context();
    let cancelled = context.cancelled.clone();
    let read = read_permit(&state, domain::ReadClass::Index, &context).unwrap();
    let lease_id = format!("result/{}", result.id);
    let thread = {
        let state = state.clone();
        let pid = pid.clone();
        let result = result.clone();
        std::thread::spawn(move || retain_created(&state, &pid, &result, false, &read))
    };
    let until = Instant::now() + Duration::from_secs(4);
    loop {
        let count: i64 = first
            .query_row(
                "SELECT count(*) FROM leases WHERE id=?1",
                [&lease_id],
                |r| r.get(0),
            )
            .unwrap();
        if count == 1 {
            break;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(5));
    }
    cancelled.store(true, Ordering::Release);
    while state.store.query_result(&pid, &result.id).unwrap().state == domain::ResultState::Ready
        && Instant::now() < until
    {
        std::thread::sleep(Duration::from_millis(5));
    }
    second.execute_batch("ROLLBACK").unwrap();
    assert_eq!(thread.join().unwrap().unwrap_err().code, "CANCELLED");
    assert_eq!(
        state.store.query_result(&pid, &result.id).unwrap().state,
        domain::ResultState::Released
    );
    let count: i64 = first
        .query_row("SELECT count(*) FROM leases WHERE id=?1", [lease_id], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
    assert_idle(&state);
}

#[test]
fn assets_retry_is_stable_and_expired_or_wrong_scope_cursors_fail() {
    let (_dir, state, pid, result, _) = fixture(false);
    let context = context();
    let read = read_permit(&state, domain::ReadClass::Index, &context).unwrap();
    let page = assets(
        &state,
        &pid,
        &result.id,
        QueryListParams {
            cursor: None,
            limit: Some(4),
            order: None,
        },
        &context,
        &read,
    )
    .unwrap()
    .page;
    let cursor = page.next_cursor.unwrap();
    let request = || QueryListParams {
        cursor: Some(cursor.clone()),
        limit: Some(4),
        order: None,
    };
    let first = assets(&state, &pid, &result.id, request(), &context, &read).unwrap();
    let again = assets(&state, &pid, &result.id, request(), &context, &read).unwrap();
    assert_eq!(
        serde_json::to_value(first).unwrap(),
        serde_json::to_value(again).unwrap()
    );
    let mut wrong: Cursor =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(&cursor).unwrap()).unwrap();
    wrong.result_id = domain::new_id();
    let wrong = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&wrong).unwrap());
    let error = assets(
        &state,
        &pid,
        &result.id,
        QueryListParams {
            cursor: Some(wrong),
            limit: Some(4),
            order: None,
        },
        &context,
        &read,
    )
    .err()
    .unwrap();
    assert_eq!(error.code, "INVALID_INPUT");
    state.store.release_result(&pid, &result.id).unwrap();
    assert_eq!(
        assets(&state, &pid, &result.id, request(), &context, &read)
            .err()
            .unwrap()
            .code,
        "VIEW_EXPIRED"
    );
}
