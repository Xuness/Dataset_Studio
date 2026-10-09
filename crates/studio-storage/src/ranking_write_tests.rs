use super::*;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use std::sync::{Condvar, atomic::AtomicUsize, mpsc};
use studio_application::{ManagementRepository, ProjectRepository};

#[test]
fn cancelling_scoped_job_capture_rolls_back_and_keeps_reads_available() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-runs");
    fs::create_dir_all(&base).unwrap();
    let temp = tempfile::Builder::new()
        .prefix("job-capture-")
        .tempdir_in(base)
        .unwrap();
    let store = Arc::new(SqliteStore::new(temp.path().join("state")).unwrap());
    let project = store.create("任务成员回滚", None).unwrap();
    let pid = project.id.clone();
    let sid = new_id();
    store
        .attach(
            &pid,
            Source {
                id: sid.clone(),
                name: "测试成员".into(),
                kind: "demo".into(),
                index_root: None,
                media_root: None,
            },
        )
        .unwrap();
    let cid = new_id();
    let request = new_id();
    let handle = store.handle(&pid).unwrap();
    {
        let db = handle.db.lock().unwrap();
        db.execute(
            "INSERT INTO collections VALUES(?1,'固定成员',65536)",
            [&cid],
        )
        .unwrap();
        db.execute("WITH RECURSIVE numbers(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM numbers WHERE n<65536) INSERT INTO collection_members SELECT ?1,?2,printf('%064x',n) FROM numbers",params![cid,sid]).unwrap();
    }
    let barrier = Arc::new((Mutex::new(false), Condvar::new()));
    let paused = barrier.clone();
    let (send, receive) = mpsc::channel();
    let inserts = AtomicUsize::new(0);
    handle
        .db
        .lock()
        .unwrap()
        .authorizer(Some(move |context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Insert {
                    table_name: "job_input_legacy"
                }
            ) && inserts.fetch_add(1, Ordering::Relaxed) == 1
            {
                send.send(()).unwrap();
                let mut released = paused.0.lock().unwrap();
                while !*released {
                    released = paused.1.wait(released).unwrap();
                }
            }
            Authorization::Allow
        }))
        .unwrap();
    let worker = store.clone();
    let scope = ScopeRef {
        project_id: pid.clone(),
        target: ScopeTarget::Workset {
            collection_id: cid,
            revision: None,
        },
    };
    let args = (pid.clone(), request.clone(), scope.clone());
    let work =
        std::thread::spawn(move || worker.submit_scope_job(&args.0, &args.1, &args.2, 0, None));
    let reached = receive.recv_timeout(std::time::Duration::from_secs(10));
    let progress = store.member_write_progress(&pid, &request).unwrap();
    let visible = store.jobs(&pid).unwrap();
    store.cancel_member_write(&pid, &request).unwrap();
    *barrier.0.lock().unwrap() = true;
    barrier.1.notify_all();
    let outcome = work.join().unwrap();
    handle
        .db
        .lock()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .unwrap();
    reached.expect("cancel after the first 32768 members");
    assert_eq!(progress.completed, 32768);
    assert!(visible.is_empty());
    assert_eq!(outcome.unwrap_err().code, "CANCELLED");
    assert!(store.jobs(&pid).unwrap().is_empty());
    assert_eq!(
        handle
            .read()
            .unwrap()
            .query_row("SELECT count(*) FROM job_inputs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let job = store
        .submit_scope_job(&pid, &new_id(), &scope, 0, None)
        .unwrap();
    assert_eq!(job.total, 65536);
}

#[test]
fn cancelling_a_ranked_reference_before_commit_keeps_readers_and_retry_consistent() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-runs");
    fs::create_dir_all(&base).unwrap();
    let temp = tempfile::Builder::new()
        .prefix("member-save-")
        .tempdir_in(base)
        .unwrap();
    let store = Arc::new(SqliteStore::new(temp.path().join("state")).unwrap());
    let project = store.create("保存回滚", None).unwrap();
    let pid = project.id.clone();
    let sid = new_id();
    let jid = new_id();
    let aid = new_id();
    let request = new_id();
    store
        .attach(
            &pid,
            Source {
                id: sid.clone(),
                name: "测试成员".into(),
                kind: "demo".into(),
                index_root: None,
                media_root: None,
            },
        )
        .unwrap();
    let directory = project.directory.join("artifacts");
    fs::create_dir_all(&directory).unwrap();
    let input_path = directory.join("input.sqlite");
    let score_path = directory.join("scores.sqlite");
    {
        let mut input = ranking_tables::RankingInputTable::create(&input_path).unwrap();
        let mut scores = ranking_tables::RankingResultTable::create(&score_path).unwrap();
        for batch in 0..4 {
            let inputs = (batch * 512..(batch + 1) * 512)
                .map(|ordinal| RankingInput {
                    ordinal,
                    source_id: sid.clone(),
                    asset_id: format!("{ordinal:064x}"),
                    rating: Some("g".into()),
                    ..Default::default()
                })
                .collect::<Vec<_>>();
            let rows = inputs
                .iter()
                .map(|input| RankingScores {
                    ordinal: input.ordinal,
                    rating: Some("g".into()),
                    main_rank: Some(input.ordinal + 1),
                    rescue_rank: Some(input.ordinal + 1),
                    selected_route: RankingRoute::Ranked,
                    ..Default::default()
                })
                .collect::<Vec<_>>();
            input.append(&inputs).unwrap();
            scores.append(&rows).unwrap();
        }
        input.finalize(&["g".into()]).unwrap();
        scores
            .finish(&RankingSummary {
                input_count: 2048,
                ..Default::default()
            })
            .unwrap();
    }
    let handle = store.handle(&pid).unwrap();
    {
        let db = handle.db.lock().unwrap();
        db.execute("INSERT INTO jobs(id,operator,status,total,created_at,idempotency_key,request_hash,delay_ms) VALUES(?1,'danbooru.metarecall','succeeded',2048,'1',?1,'fixture',0)",[&jid]).unwrap();
        let provenance = ArtifactProvenance {
            run: None,
            input_scope: None,
            input_sha256: None,
            attempt: Some(1),
            input_artifacts: Vec::new(),
            fields_frozen: true,
            evidence: "isolated fixture".into(),
        };
        db.execute("INSERT INTO artifacts(id,job_id,output_id,name,kind,schema_version,status,count,created_at,files_json,provenance_json) VALUES(?1,?2,'data','fixture',?3,1,'ready',2048,'1','[]',?4)",params![aid,jid,RANKING_KIND,serde_json::to_string(&provenance).unwrap()]).unwrap();
    }
    let paused = Arc::new((Mutex::new(false), Condvar::new()));
    let barrier = paused.clone();
    let (arrived, receive) = mpsc::channel();
    let inserts = AtomicUsize::new(0);
    handle
        .db
        .lock()
        .unwrap()
        .authorizer(Some(move |context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Insert {
                    table_name: "ranking_memberships"
                }
            ) && inserts.fetch_add(1, Ordering::Relaxed) == 0
            {
                arrived.send(()).unwrap();
                let (lock, wake) = &*barrier;
                let mut released = lock.lock().unwrap();
                while !*released {
                    released = wake.wait(released).unwrap();
                }
            }
            Authorization::Allow
        }))
        .unwrap();
    let worker = store.clone();
    let args = (
        pid.clone(),
        aid.clone(),
        request.clone(),
        score_path.clone(),
        input_path.clone(),
    );
    let saved = std::thread::spawn(move || {
        worker.ranking_workset(
            &args.0,
            &args.1,
            &args.2,
            "应当回滚",
            &RankingFilter::default(),
            (&args.3, &args.4),
        )
    });
    let reached = receive.recv_timeout(std::time::Duration::from_secs(10));
    let progress = store.member_write_progress(&pid, &request).unwrap();
    let before = store.collections(&pid).unwrap();
    store.cancel_member_write(&pid, &request).unwrap();
    *paused.0.lock().unwrap() = true;
    paused.1.notify_all();
    let outcome = saved.join().unwrap();
    handle
        .db
        .lock()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .unwrap();
    reached.expect("the test must cancel before the reference is committed");
    assert_eq!(progress.completed, 0);
    assert!(
        before.is_empty(),
        "uncommitted collections must stay invisible to readers"
    );
    assert_eq!(outcome.unwrap_err().code, "CANCELLED");
    assert!(store.collections(&pid).unwrap().is_empty());
    assert_eq!(
        store.member_write_progress(&pid, &request).unwrap().state,
        "cancelled"
    );
    let complete_request = new_id();
    let created = store
        .ranking_workset(
            &pid,
            &aid,
            &complete_request,
            "完整保存",
            &RankingFilter::default(),
            (&score_path, &input_path),
        )
        .unwrap();
    assert_eq!(created.count, 2048);
    store.cancel_member_write(&pid, &complete_request).unwrap();
    let duplicate = store
        .ranking_workset(
            &pid,
            &aid,
            &complete_request,
            "完整保存",
            &RankingFilter::default(),
            (&score_path, &input_path),
        )
        .unwrap();
    assert_eq!(
        duplicate.id, created.id,
        "late cancellation must preserve committed idempotency"
    );
    assert_eq!(store.collections(&pid).unwrap().len(), 1);
    let scope = ScopeRef {
        project_id: pid.clone(),
        target: ScopeTarget::Workset {
            collection_id: created.id.clone(),
            revision: None,
        },
    };
    let reader = handle.read().unwrap();
    assert_eq!(
        reader
            .query_row("SELECT count(*) FROM collection_member_legacy", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        0
    );
    assert_eq!(reader.query_row("SELECT count(*) FROM result_members WHERE result_id=(SELECT result_id FROM collection_bases WHERE collection_id=?1)", [&created.id], |r|r.get::<_,i64>(0)).unwrap(), 2048);
    drop(reader);
    let later = store
        .browse_scope_keys(
            &pid,
            &scope,
            Some(&AssetKey {
                source_id: sid.clone(),
                asset_id: format!("{:064x}", 1500),
            }),
            96,
            false,
        )
        .unwrap();
    assert_eq!(later.len(), 96);
    assert_eq!(later[0].asset_id, format!("{:064x}", 1501));
    let reverse = store
        .browse_scope_keys(
            &pid,
            &scope,
            Some(&AssetKey {
                source_id: sid.clone(),
                asset_id: format!("{:064x}", 1500),
            }),
            96,
            true,
        )
        .unwrap();
    assert_eq!(reverse[0].asset_id, format!("{:064x}", 1499));
    let spec = QuerySpec {
        version: 3,
        source_ids: vec![sid.clone()],
        conditions: vec![QueryCondition {
            field: format!("project.{aid}.rating"),
            operator: QueryOperator::Eq,
            value: Some(QueryValue::Text("g".into())),
        }],
        observation_rule: ObservationRule::CurrentPost,
        order: QueryOrder::AssetKeyAsc,
        input_scope: Some(scope.clone()),
    };
    let versions = vec![QuerySourceVersion {
        source_id: sid.clone(),
        semantics_version: None,
        catalog_revision: "fixture-1".into(),
        analysis_sequence: None,
        consistency: "fixture".into(),
    }];
    let first = store
        .create_ranking_result(
            &pid,
            &spec,
            None,
            &versions,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap()
        .unwrap();
    let alias = store
        .create_ranking_result(
            &pid,
            &spec,
            None,
            &versions,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap()
        .unwrap();
    assert_eq!(first.state, ResultState::Ready);
    assert_eq!(first.count, Some(2048));
    assert_eq!(alias.cache.mode, "reused");
    let result_scope = |id: String| ScopeRef {
        project_id: pid.clone(),
        target: ScopeTarget::QueryResult { result_id: id },
    };
    assert_eq!(
        store
            .canonical_ranked_scope(&pid, &result_scope(first.id.clone()))
            .unwrap(),
        store
            .canonical_ranked_scope(&pid, &result_scope(alias.id.clone()))
            .unwrap()
    );
    store
        .remove_object(&pid, ObjectKind::Workset, &created.id, 0)
        .unwrap();
    let after_removal = store
        .browse_scope_keys(
            &pid,
            &result_scope(alias.id.clone()),
            Some(&AssetKey {
                source_id: sid.clone(),
                asset_id: format!("{:064x}", 1500),
            }),
            96,
            false,
        )
        .unwrap();
    assert_eq!(
        after_removal, later,
        "fixed query membership survives removal of its original workset"
    );
    assert!(
        store
            .ranked_scope(&pid, &result_scope(alias.id))
            .unwrap()
            .is_some()
    );
}
