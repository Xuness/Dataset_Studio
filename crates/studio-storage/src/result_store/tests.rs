use super::*;
use std::sync::mpsc;
use studio_application::{DraftRepository, ProjectRepository, QueryRepository, ScopeRepository};

fn fixture() -> (tempfile::TempDir, Arc<SqliteStore>, Project, Source) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-runs");
    fs::create_dir_all(&root).unwrap();
    let temp = tempfile::Builder::new()
        .prefix("fixed-members-")
        .tempdir_in(root)
        .unwrap();
    let store = Arc::new(SqliteStore::new(temp.path().join("app")).unwrap());
    let project = store.create("成员隔离", None).unwrap();
    let source = Source {
        id: new_id(),
        name: "source".into(),
        kind: "demo".into(),
        index_root: None,
        media_root: None,
    };
    store.attach(&project.id, source.clone()).unwrap();
    (temp, store, project, source)
}
fn result(store: &SqliteStore, pid: &str, source: &Source) -> QueryResult {
    store
        .create_snapshot_result(
            pid,
            QuerySpec {
                version: 3,
                source_ids: vec![source.id.clone()],
                conditions: vec![],
                observation_rule: ObservationRule::CurrentPost,
                order: QueryOrder::AssetKeyAsc,
                input_scope: None,
            },
            vec![QuerySourceVersion {
                source_id: source.id.clone(),
                catalog_revision: "demo-v1".into(),
                analysis_sequence: None,
                semantics_version: None,
                consistency: "immutable_demo".into(),
            }],
            false,
        )
        .unwrap()
}
fn stage(root: &Path, source: &Source, n: usize) -> QueryStage {
    let mut stage = QueryStage::new(root).unwrap();
    stage.full_source(&source.id);
    for start in (0..n).step_by(512) {
        let keys = (start..(start + 512).min(n))
            .map(|n| AssetKey {
                source_id: source.id.clone(),
                asset_id: format!("asset-{n:08}"),
            })
            .collect::<Vec<_>>();
        stage
            .append(&keys, &vec![Some(1); keys.len()], keys.len() as u64)
            .unwrap();
    }
    stage.seal().unwrap();
    stage
}

#[test]
fn bulk_members_do_not_hold_the_control_writer_and_publish_only_after_ready() {
    let (temp, store, project, source) = fixture();
    let result = result(&store, &project.id, &source);
    store.start_result(&project.id, &result.id).unwrap();
    let staged = stage(temp.path(), &source, 20000);
    let handle = store.handle(&project.id).unwrap();
    std::thread::scope(|scope| {
        let writer = handle.db.lock().unwrap();
        let (send, receive) = mpsc::channel();
        let worker = store.clone();
        let pid = project.id.clone();
        let rid = result.id.clone();
        scope.spawn(move || {
            send.send(worker.publish_snapshot_stage(&pid, &rid, &staged, &AtomicBool::new(false)))
                .unwrap()
        });
        let independent = Connection::open_with_flags(
            project.directory.join("members.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let ready: bool = independent
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM datasets WHERE id=?1 AND state='sealed')",
                    [&result.id],
                    |r| r.get(0),
                )
                .unwrap();
            if ready {
                break;
            }
            assert!(
                std::time::Instant::now() < until,
                "member writer waited for the control writer"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            store
                .draft(&project.id, "query", "default")
                .unwrap()
                .is_none()
        );
        let read = handle.read().unwrap();
        assert_eq!(
            read.query_row(
                "SELECT count(*) FROM result_members WHERE result_id=?1",
                [&result.id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        drop(read);
        drop(writer);
        receive
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap()
            .unwrap();
    });
    assert_eq!(
        store
            .finish_result(&project.id, &result.id, None)
            .unwrap()
            .count,
        Some(20000)
    );
    let read = handle.read().unwrap();
    assert_eq!(
        read.query_row("SELECT count(*) FROM query_member_data", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        read.query_row(
            "SELECT count(*) FROM result_members WHERE result_id=?1",
            [&result.id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        20000
    );
    drop(read);
    let scope = ScopeRef {
        project_id: project.id.clone(),
        target: ScopeTarget::QueryResult {
            result_id: result.id.clone(),
        },
    };
    let collection = store
        .save_scope_collection(&project.id, "固定成员", &scope)
        .unwrap();
    assert_eq!(collection.count, 20000);
    let read = handle.read().unwrap();
    assert_eq!(
        read.query_row("SELECT count(*) FROM collection_member_legacy", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
    assert_eq!(
        store
            .collection_keys(&project.id, &collection.id, None, 3)
            .unwrap()
            .len(),
        3
    );
    drop(read);
    let workset = ScopeRef {
        project_id: project.id.clone(),
        target: ScopeTarget::Workset {
            collection_id: collection.id,
            revision: None,
        },
    };
    let job = store
        .submit_scope_job(&project.id, &new_id(), &workset, 0, None)
        .unwrap();
    assert_eq!(job.total, 20000);
    assert_eq!(
        store.job_inputs(&project.id, &job.id, None).unwrap().len(),
        256
    );
    let read = handle.read().unwrap();
    assert_eq!(
        read.query_row("SELECT count(*) FROM job_input_legacy", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let stats = store.query_cache_stats(&project.id).unwrap();
    assert!(stats.fixed_bytes > 0);
    assert_eq!(stats.member_versions, 20000);
}

#[test]
fn control_snapshot_precedes_members_and_collection_references_preserve_membership() {
    let (temp, store, project, source) = fixture();
    let handle = store.handle(&project.id).unwrap();
    let first = result(&store, &project.id, &source);
    store.start_result(&project.id, &first.id).unwrap();
    store
        .publish_snapshot_stage(
            &project.id,
            &first.id,
            &stage(temp.path(), &source, 10),
            &AtomicBool::new(false),
        )
        .unwrap();
    store.finish_result(&project.id, &first.id, None).unwrap();
    let old = handle.read().unwrap();
    let second = result(&store, &project.id, &source);
    store.start_result(&project.id, &second.id).unwrap();
    store
        .publish_snapshot_stage(
            &project.id,
            &second.id,
            &stage(temp.path(), &source, 20),
            &AtomicBool::new(false),
        )
        .unwrap();
    store.finish_result(&project.id, &second.id, None).unwrap();
    assert_eq!(
        old.query_row(
            "SELECT count(*) FROM result_members WHERE result_id=?1",
            [&second.id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        handle
            .read()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM result_members WHERE result_id=?1",
                [&second.id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        20
    );
    store.release_result(&project.id, &first.id).unwrap();
    assert_eq!(store.collect_snapshot_orphans(&project.id).unwrap(), 0);
    assert_eq!(
        old.query_row(
            "SELECT count(*) FROM result_members WHERE result_id=?1",
            [&first.id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        10
    );
    drop(old);
    assert_eq!(store.collect_snapshot_orphans(&project.id).unwrap(), 10);
}
