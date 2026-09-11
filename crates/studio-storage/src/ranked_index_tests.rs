use super::*;
use std::sync::atomic::AtomicUsize;

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
            RankingOrder::Main => 1,
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
