use super::*;
use std::sync::atomic::AtomicUsize;

#[test]
fn v2_direct_and_fused_scope_indexes_have_independent_orders_after_reopen() {
    let (tmp, mut plan) = fixture();
    plan.meta.version = 2;
    let db = Connection::open(&plan.scores).unwrap();
    db.execute_batch("ALTER TABLE scores ADD COLUMN direct_rank INTEGER; ALTER TABLE scores ADD COLUMN fused_rank INTEGER; UPDATE scores SET direct_rank=65537-ordinal,fused_rank=ordinal;").unwrap();
    drop(db);
    let file = tmp.path().join("v2-index.sqlite");
    let cancel = Arc::new(AtomicBool::new(false));
    RankedIndex::build(
        &file,
        &plan,
        32 << 20,
        32 << 20,
        cancel.clone(),
        Arc::new(RankedIndexProgress::default()),
    )
    .unwrap();
    let index = RankedIndex::open(&file, &plan.meta, cancel.clone()).unwrap();
    let page = index.page(RankingOrder::Direct, false, None, 12).unwrap();
    assert_eq!(page[0].ordinal, 65536);
    assert!(page.windows(2).all(|w| w[0].ordinal > w[1].ordinal));
    assert_eq!(
        index
            .locate_position(129, RankingOrder::Direct, false)
            .unwrap()
            .unwrap()
            .ordinal,
        64512
    );
    assert_eq!(
        index
            .locate_rank(1, "g", RankingOrder::Direct, false)
            .unwrap()
            .unwrap()
            .ordinal,
        65536
    );
    drop(index);
    let index = RankedIndex::open(&file, &plan.meta, cancel).unwrap();
    let page = index.page(RankingOrder::Fused, false, None, 12).unwrap();
    assert_eq!(page[0].ordinal, 1);
    assert_eq!(page[1].ordinal, 8);
    assert_eq!(
        index
            .locate_position(129, RankingOrder::Fused, false)
            .unwrap()
            .unwrap()
            .ordinal,
        1024
    );
    assert_eq!(
        index
            .locate_rank(1024, "g", RankingOrder::Fused, true)
            .unwrap()
            .unwrap()
            .ordinal,
        1024
    );
}

fn fixture() -> (tempfile::TempDir, RankedIndexPlan) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-runs");
    std::fs::create_dir_all(&base).unwrap();
    let tmp = tempfile::Builder::new()
        .prefix("ranked-index-")
        .tempdir_in(base)
        .unwrap();
    let project = tmp.path().join("project.sqlite");
    let input = tmp.path().join("input.sqlite");
    let scores = tmp.path().join("scores.sqlite");
    let cid = new_id();
    let db = Connection::open(&project).unwrap();
    db.execute_batch("CREATE TABLE collection_members(collection_id TEXT,source_id TEXT,asset_id TEXT,PRIMARY KEY(collection_id,source_id,asset_id)) WITHOUT ROWID;").unwrap();
    db.execute("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<65536) INSERT INTO collection_members SELECT ?1,'source',printf('%064x',x) FROM n WHERE x%8=0 OR x=1",[&cid]).unwrap();
    drop(db);
    let db = Connection::open(&input).unwrap();
    db.execute_batch("CREATE TABLE input_rows(ordinal INTEGER PRIMARY KEY,source_id TEXT,asset_id BLOB,post_id INTEGER); CREATE UNIQUE INDEX input_identity ON input_rows(source_id,asset_id); WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<65536) INSERT INTO input_rows SELECT x,'source',unhex(printf('%064x',x)),42 FROM n;").unwrap();
    drop(db);
    let db = Connection::open(&scores).unwrap();
    db.execute_batch("CREATE TABLE scores(ordinal INTEGER PRIMARY KEY,rating TEXT,main_rank INTEGER,rescue_rank INTEGER); WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<65536) INSERT INTO scores SELECT x,'g',1,NULL FROM n;").unwrap();
    drop(db);
    let plan = RankedIndexPlan {
        requested_scope: ScopeRef {
            project_id: new_id(),
            target: ScopeTarget::Workset {
                collection_id: cid.clone(),
            },
        },
        meta: RankedIndexMeta {
            version: 1,
            key: "a".repeat(64),
            scope: ScopeRef {
                project_id: new_id(),
                target: ScopeTarget::Workset { collection_id: cid },
            },
            count: 8193,
        },
        project,
        input,
        scores,
    };
    (tmp, plan)
}
#[test]
fn sparse_late_pages_and_large_ties_use_bounded_index_work_and_survive_reopen() {
    let (tmp, plan) = fixture();
    let original = std::fs::read(&plan.input).unwrap();
    let output = tmp.path().join("view.sqlite");
    let cancel = Arc::new(AtomicBool::new(false));
    RankedIndex::build(
        &output,
        &plan,
        32 << 20,
        32 << 20,
        cancel.clone(),
        Arc::new(RankedIndexProgress::default()),
    )
    .unwrap();
    assert_eq!(std::fs::read(&plan.input).unwrap(), original);
    for order in [
        RankingOrder::Main,
        RankingOrder::Rescue,
        RankingOrder::Input,
    ] {
        let index = RankedIndex::open(&output, &plan.meta, cancel.clone()).unwrap();
        let steps = Arc::new(AtomicUsize::new(0));
        let counter = steps.clone();
        index
            .db
            .progress_handler(
                100,
                Some(move || {
                    counter.fetch_add(100, Ordering::Relaxed);
                    false
                }),
            )
            .unwrap();
        let key = match order {
            RankingOrder::Main | RankingOrder::Direct | RankingOrder::Fused => 1,
            RankingOrder::Rescue => i64::MAX,
            RankingOrder::Input => 64000,
        };
        let after = RankingPosition {
            group: "g".into(),
            position: key,
            ordinal: 64000,
        };
        let page = index.page(order, false, Some(&after), 97).unwrap();
        assert_eq!(page.len(), 97);
        assert_eq!(page[0].ordinal, 64008);
        assert!(
            steps.load(Ordering::Relaxed) < 10000,
            "a late tie group must not scan its prefix"
        );
        assert_eq!(index.locate(42, order, false).unwrap().unwrap().ordinal, 1);
        assert_eq!(
            index.locate(42, order, true).unwrap().unwrap().ordinal,
            65536
        );
        assert!(index.contains(64008).unwrap());
        assert!(!index.contains(64009).unwrap());
    }
    let mut wrong = plan.meta.clone();
    wrong.key = "b".repeat(64);
    assert!(RankedIndex::open(&output, &wrong, cancel).is_err());
}
#[test]
fn numeric_anchors_use_bounded_work_across_sparse_groups_and_bookmark_edges() {
    let (tmp, plan) = fixture();
    let db = Connection::open(&plan.scores).unwrap();
    db.execute_batch("UPDATE scores SET rating=CASE WHEN ordinal<257 THEN 'e' WHEN ordinal<32769 THEN 'g' ELSE 's' END,main_rank=ordinal,rescue_rank=CASE WHEN ordinal%16=0 THEN NULL ELSE ordinal END;").unwrap();
    drop(db);
    let output = tmp.path().join("positions.sqlite");
    let cancel = Arc::new(AtomicBool::new(false));
    RankedIndex::build(
        &output,
        &plan,
        32 << 20,
        32 << 20,
        cancel.clone(),
        Arc::new(RankedIndexProgress::default()),
    )
    .unwrap();
    let index = RankedIndex::open(&output, &plan.meta, cancel.clone()).unwrap();
    let steps = Arc::new(AtomicUsize::new(0));
    let counter = steps.clone();
    index
        .db
        .progress_handler(
            100,
            Some(move || {
                counter.fetch_add(100, Ordering::Relaxed);
                false
            }),
        )
        .unwrap();
    for order in [
        RankingOrder::Main,
        RankingOrder::Rescue,
        RankingOrder::Input,
        RankingOrder::Direct,
        RankingOrder::Fused,
    ] {
        let column = match order {
            RankingOrder::Rescue => "rescue_rank",
            _ => "ordinal",
        };
        let expected: Vec<u64> = index
            .db
            .prepare(&format!(
                "SELECT ordinal FROM members ORDER BY rating,{column},ordinal"
            ))
            .unwrap()
            .query_map([], |r| unsigned(r, 0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        for descending in [false, true] {
            for target in [
                1, 2, 32, 33, 127, 128, 129, 130, 4096, 4097, 8191, 8192, 8193,
            ] {
                steps.store(0, Ordering::Relaxed);
                let found = index
                    .locate_position(target, order, descending)
                    .unwrap()
                    .unwrap();
                let n = if descending {
                    expected.len() - target as usize
                } else {
                    target as usize - 1
                };
                assert_eq!(found.ordinal, expected[n]);
                assert!(
                    steps.load(Ordering::Relaxed) < 10000,
                    "large positions must not scan the full prefix"
                );
            }
        }
        assert!(index.locate_position(0, order, false).unwrap().is_none());
        assert!(index.locate_position(8194, order, false).unwrap().is_none());
    }
    assert_eq!(
        index
            .locate_rank(64000, "s", RankingOrder::Main, true)
            .unwrap()
            .unwrap()
            .ordinal,
        64000
    );
    assert!(
        index
            .locate_rank(64001, "s", RankingOrder::Main, false)
            .unwrap()
            .is_none()
    );
    assert!(
        index
            .locate_rank(64000, "g", RankingOrder::Main, false)
            .unwrap()
            .is_none()
    );
    assert!(
        index
            .locate_rank(i64::MAX as u64, "s", RankingOrder::Rescue, false)
            .unwrap()
            .is_none()
    );
    assert!(
        index
            .locate_rank(1, "e", RankingOrder::Input, false)
            .unwrap()
            .is_none()
    );
    drop(index);
    // Old derived caches are rebuildable, while their original scores are untouched.
    let db = Connection::open(&output).unwrap();
    db.execute_batch("DROP TABLE rank_positions; DELETE FROM meta WHERE key='position_stride';")
        .unwrap();
    drop(db);
    assert!(RankedIndex::open(&output, &plan.meta, cancel).is_err());
}

#[test]
fn cancelled_or_over_budget_builds_never_have_a_complete_header() {
    let (tmp, plan) = fixture();
    let output = tmp.path().join("cancelled.sqlite");
    let error = RankedIndex::build(
        &output,
        &plan,
        8 << 20,
        8 << 20,
        Arc::new(AtomicBool::new(true)),
        Arc::new(RankedIndexProgress::default()),
    )
    .unwrap_err();
    assert_eq!(error.code, "CANCELLED");
    assert!(!output.exists());
    let output = tmp.path().join("full.sqlite");
    assert!(
        RankedIndex::build(
            &output,
            &plan,
            8 << 20,
            65536,
            Arc::new(AtomicBool::new(false)),
            Arc::new(RankedIndexProgress::default())
        )
        .is_err()
    );
    assert!(RankedIndex::metadata(&output).is_err());
}
#[test]
fn missing_frozen_members_are_rejected_before_publication() {
    let (tmp, mut plan) = fixture();
    plan.meta.count += 1;
    let output = tmp.path().join("incomplete.sqlite");
    let error = RankedIndex::build(
        &output,
        &plan,
        32 << 20,
        32 << 20,
        Arc::new(AtomicBool::new(false)),
        Arc::new(RankedIndexProgress::default()),
    )
    .unwrap_err();
    assert_eq!(error.code, "ARTIFACT_INVALID");
    assert!(RankedIndex::metadata(&output).is_err());
}
