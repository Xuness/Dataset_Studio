use super::*;
use studio_storage::ranked_index::RankedIndexMeta;

fn fixture() -> (tempfile::TempDir, Arc<RankedIndexes>, RankedIndexPlan) {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-runs");
    fs::create_dir_all(&base).unwrap();
    let temp = tempfile::Builder::new()
        .prefix("排名 索引缓存-")
        .tempdir_in(base)
        .unwrap();
    let cache = Arc::new(RankedIndexes::new(temp.path().into()));
    let plan = RankedIndexPlan {
        requested_scope: ScopeRef {
            project_id: new_id(),
            target: ScopeTarget::Workset {
                collection_id: new_id(),
            },
        },
        meta: RankedIndexMeta {
            version: 1,
            key: "a".repeat(64),
            scope: ScopeRef {
                project_id: new_id(),
                target: ScopeTarget::Workset {
                    collection_id: new_id(),
                },
            },
            count: 0,
        },
        project: PathBuf::new(),
        input: PathBuf::new(),
        scores: PathBuf::new(),
    };
    let db = rusqlite::Connection::open(cache.path(&plan.meta.key).unwrap()).unwrap();
    db.execute_batch("CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT); CREATE TABLE members(ordinal INTEGER PRIMARY KEY,rating TEXT,main_rank INTEGER,rescue_rank INTEGER,post_id INTEGER);").unwrap();
    db.execute_batch("CREATE TABLE rank_positions(order_name TEXT,sequence INTEGER,ordinal INTEGER,PRIMARY KEY(order_name,sequence)) WITHOUT ROWID; INSERT INTO meta VALUES('position_stride','128');").unwrap();
    for (name, column) in [
        ("main", "main_rank"),
        ("rescue", "rescue_rank"),
        ("input", "ordinal"),
    ] {
        db.execute_batch(&format!("CREATE INDEX ordered_{name} ON members(rating,{column},ordinal); CREATE INDEX post_{name} ON members(post_id,rating,{column},ordinal) WHERE post_id IS NOT NULL;")).unwrap();
    }
    db.execute(
        "INSERT INTO meta VALUES('complete',?1)",
        [serde_json::to_string(&plan.meta).unwrap()],
    )
    .unwrap();
    drop(db);
    (temp, cache, plan)
}
#[test]
fn leases_and_open_readers_protect_indexes_from_eviction() {
    let (_temp, cache, plan) = fixture();
    let lid = new_id();
    cache.lease(&plan.meta.scope, &lid, false).unwrap();
    cache.prune(0, true, 0, None).unwrap();
    assert_eq!(cache.metrics().unwrap().entries, 1);
    let reader = cache
        .open(&plan, Arc::new(AtomicBool::new(false)))
        .unwrap()
        .unwrap();
    cache.lease(&plan.meta.scope, &lid, true).unwrap();
    cache.prune(0, true, 0, None).unwrap();
    assert_eq!(cache.metrics().unwrap().entries, 1);
    drop(reader);
    cache.prune(0, true, 0, None).unwrap();
    assert_eq!(cache.metrics().unwrap().entries, 0);
}
#[test]
fn cancelled_reads_preserve_good_indexes_and_malformed_indexes_can_rebuild() {
    let (_temp, cache, plan) = fixture();
    let result = cache.open(&plan, Arc::new(AtomicBool::new(true)));
    assert_eq!(result.err().unwrap().code, "CANCELLED");
    assert!(cache.path(&plan.meta.key).unwrap().is_file());
    fs::write(cache.path(&plan.meta.key).unwrap(), b"incomplete cache").unwrap();
    assert!(
        cache
            .open(&plan, Arc::new(AtomicBool::new(false)))
            .unwrap()
            .is_none()
    );
    assert!(!cache.path(&plan.meta.key).unwrap().exists());
}
#[test]
fn unfinished_builds_stop_after_the_last_view_and_request_leave() {
    let (_temp, cache, plan) = fixture();
    let job = Arc::new(Build {
        plan: plan.clone(),
        cancelled: Arc::new(AtomicBool::new(false)),
        progress: Arc::new(RankedIndexProgress::default()),
        finished: AtomicBool::new(false),
        error: Mutex::new(None),
        touched: Mutex::new(Instant::now() - Duration::from_secs(8)),
    });
    cache
        .state
        .lock()
        .unwrap()
        .jobs
        .insert(plan.meta.key.clone(), job.clone());
    let lid = new_id();
    cache.lease(&plan.meta.scope, &lid, false).unwrap();
    cache.tick();
    assert!(!job.cancelled.load(Ordering::Acquire));
    cache.lease(&plan.meta.scope, &lid, true).unwrap();
    cache.tick();
    assert!(job.cancelled.load(Ordering::Acquire));
}
#[test]
fn session_only_indexes_are_removed_after_the_project_session_ends() {
    let (_temp, cache, plan) = fixture();
    let active = HashSet::from([plan.meta.scope.project_id.clone()]);
    cache.prune(u64::MAX, false, 3600, Some(&active)).unwrap();
    assert_eq!(cache.metrics().unwrap().entries, 1);
    cache
        .prune(u64::MAX, false, 3600, Some(&HashSet::new()))
        .unwrap();
    assert_eq!(cache.metrics().unwrap().entries, 0);
}

#[test]
fn compatible_alias_index_is_adopted_without_copying_members() {
    let (_temp, cache, original) = fixture();
    let mut plan = original.clone();
    plan.meta.key = "b".repeat(64);
    plan.meta.scope.target = ScopeTarget::QueryResult {
        result_id: new_id(),
    };
    let old_path = cache.path(&original.meta.key).unwrap();
    let size = fs::metadata(&old_path).unwrap().len();
    cache.adopt(&plan, |old| Ok(old == &original.meta)).unwrap();
    assert!(!old_path.exists());
    assert_eq!(
        fs::metadata(cache.path(&plan.meta.key).unwrap())
            .unwrap()
            .len(),
        size
    );
    assert!(
        cache
            .open(&plan, Arc::new(AtomicBool::new(false)))
            .unwrap()
            .is_some()
    );
    assert_eq!(cache.metrics().unwrap().builds, 0);
    let owner = Arc::new(RankedIndexes::new(cache.root.clone()));
    assert!(
        owner
            .open(&plan, Arc::new(AtomicBool::new(false)))
            .unwrap()
            .is_some()
    );
}
