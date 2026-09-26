use super::*;
use std::sync::atomic::AtomicUsize;
use studio_application::QueryRepository;

struct Fixture {
    // Close SQLite handles before TempDir cleanup, especially on Windows.
    store: SqliteStore,
    root: tempfile::TempDir,
    pid: String,
    sources: Vec<String>,
}
impl Fixture {
    fn new(source_count: usize) -> Self {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-runs");
        fs::create_dir_all(&base).unwrap();
        let root = tempfile::Builder::new()
            .prefix("query-publication-")
            .tempdir_in(base)
            .unwrap();
        let store = SqliteStore::new(root.path().join("runtime")).unwrap();
        let project = store.create("发布验证", None).unwrap();
        let sources = (0..source_count).map(|_| new_id()).collect::<Vec<_>>();
        for id in &sources {
            store
                .attach(
                    &project.id,
                    Source {
                        id: id.clone(),
                        name: "fixture".into(),
                        kind: "demo".into(),
                        index_root: None,
                        media_root: None,
                    },
                )
                .unwrap();
        }
        Self {
            root,
            store,
            pid: project.id,
            sources,
        }
    }
    fn begin(&self, version: usize) -> QueryResult {
        let spec = QuerySpec {
            version: 1,
            source_ids: self.sources.clone(),
            conditions: vec![],
            observation_rule: ObservationRule::CurrentPost,
            order: QueryOrder::AssetKeyAsc,
            input_scope: None,
        };
        let versions = self
            .sources
            .iter()
            .map(|id| QuerySourceVersion {
                source_id: id.clone(),
                catalog_revision: format!("revision-{version}"),
                analysis_sequence: None,
                semantics_version: None,
                consistency: "fixture".into(),
            })
            .collect();
        let result = self
            .store
            .create_cached_result(&self.pid, None, spec, versions, true)
            .unwrap();
        assert!(self.store.start_result(&self.pid, &result.id).unwrap());
        result
    }
    fn key(&self, source: usize, n: i64) -> AssetKey {
        AssetKey {
            source_id: self.sources[source].clone(),
            asset_id: format!("{n:08}"),
        }
    }
    fn stage(&self) -> QueryStage {
        QueryStage::new(self.root.path()).unwrap()
    }
    fn publish(&self, result: &QueryResult, stage: &QueryStage) {
        self.store
            .publish_stage(
                &self.pid,
                &result.id,
                stage,
                "incremental",
                &AtomicBool::new(false),
            )
            .unwrap();
    }
    fn finish(&self, result: &QueryResult) -> QueryResult {
        self.store
            .finish_result(&self.pid, &result.id, None)
            .unwrap()
    }
    fn verify(&self, result: &QueryResult, count: u64) {
        let handle = self.store.handle(&self.pid).unwrap();
        let db = handle.read().unwrap();
        let stored: u64 = db
            .query_row(
                "SELECT count(*) FROM result_members WHERE result_id=?1",
                [&result.id],
                |r| unsigned(r, 0),
            )
            .unwrap();
        assert_eq!(stored, count);
        let raw: u64 = db
            .query_row(
                "SELECT count FROM query_results WHERE id=?1",
                [&result.id],
                |r| unsigned(r, 0),
            )
            .unwrap();
        assert_eq!(raw, count);
        let public = self.store.query_result(&self.pid, &result.id).unwrap();
        assert_eq!(
            public.count,
            (public.state == ResultState::Ready).then_some(count)
        );
        let (reported, actual): (u64, u64) = db.query_row("SELECT f.stored_members,(SELECT count(*) FROM query_member_data m WHERE m.family_id=f.id) FROM query_families f JOIN query_results r ON r.family_id=f.id WHERE r.id=?1", [&result.id], |r| Ok((unsigned(r, 0)?, unsigned(r, 1)?))).unwrap();
        assert_eq!(reported, actual);
    }
}

#[test]
fn mixed_full_delta_counts_links_nulls_rollback_and_replay_preserve_old_results() {
    let f = Fixture::new(2);
    let first = f.begin(1);
    let mut initial = f.stage();
    for source in &f.sources {
        initial.full_source(source);
    }
    initial
        .append(
            &(1..=5).map(|n| f.key(0, n)).collect::<Vec<_>>(),
            &[Some(1), Some(2), None, None, Some(5)],
            5,
        )
        .unwrap();
    initial
        .append(&[f.key(1, 1), f.key(1, 2)], &[Some(10), Some(20)], 2)
        .unwrap();
    initial.append(&[f.key(0, 1)], &[Some(999)], 1).unwrap(); // deduplicated stage
    initial.seal().unwrap();
    f.publish(&first, &initial);
    f.finish(&first);
    f.verify(&first, 7);

    let mut delta = f.stage();
    delta
        .affected(&[
            f.key(0, 1),
            f.key(0, 2),
            f.key(0, 3),
            f.key(0, 4),
            f.key(0, 6),
        ])
        .unwrap();
    delta
        .append(
            &[f.key(0, 2), f.key(0, 3), f.key(0, 4), f.key(0, 6)],
            &[None, Some(30), None, Some(6)],
            4,
        )
        .unwrap();
    delta.full_source(&f.sources[1]);
    delta
        .append(&[f.key(1, 2), f.key(1, 7)], &[Some(20), None], 2)
        .unwrap();
    delta.seal().unwrap();
    let failed = f.begin(2);
    f.publish(&failed, &delta);
    f.verify(&failed, 7);
    let replay = f
        .store
        .publish_stage(
            &f.pid,
            &failed.id,
            &delta,
            "incremental",
            &AtomicBool::new(false),
        )
        .unwrap_err();
    assert_eq!(replay.code, "QUERY_PUBLICATION_CONFLICT");
    f.store
        .finish_result(
            &f.pid,
            &failed.id,
            Some(&Error::new("SOURCE_CHANGED", "final fence")),
        )
        .unwrap();
    f.verify(&first, 7);

    let next = f.begin(2);
    f.publish(&next, &delta);
    let next = f.finish(&next);
    assert_eq!(next.cache.changed_members, 8);
    f.verify(&next, 7);
    f.verify(&first, 7);
    let handle = f.store.handle(&f.pid).unwrap();
    let db = handle.read().unwrap();
    let old: Option<i64> = db.query_row("SELECT post_id FROM query_member_data WHERE family_id=?1 AND source_id=?2 AND asset_id=?3 AND valid_from=1", params![first.id, f.sources[0], f.key(0, 2).asset_id], |r| r.get(0)).unwrap();
    assert_eq!(old, Some(2));
    assert_eq!(db.query_row("SELECT count(*) FROM meta WHERE key>='query_publication/' AND key<'query_publication0'", [], |r| r.get::<_, u32>(0)).unwrap(), 0);
    drop(db);
    let no_change = f.begin(3);
    let stage = f.stage();
    stage.seal().unwrap();
    f.publish(&no_change, &stage);
    assert_eq!(f.finish(&no_change).cache.changed_members, 0);
    f.verify(&no_change, 7);

    let empty = f.begin(4);
    let mut stage = f.stage();
    for source in &f.sources {
        stage.full_source(source);
    }
    stage.seal().unwrap();
    f.publish(&empty, &stage);
    f.finish(&empty);
    f.verify(&empty, 0);
    f.verify(&next, 7);
}

#[test]
fn recovery_removes_unfinished_receipts_for_changed_and_unchanged_publications() {
    for change in [true, false] {
        let mut f = Fixture::new(1);
        let first = f.begin(1);
        let mut stage = f.stage();
        stage.full_source(&f.sources[0]);
        stage.append(&[f.key(0, 1)], &[Some(1)], 1).unwrap();
        stage.seal().unwrap();
        f.publish(&first, &stage);
        f.finish(&first);
        let pending = f.begin(2);
        let mut stage = f.stage();
        if change {
            stage.affected(&[f.key(0, 1)]).unwrap();
        }
        stage.seal().unwrap();
        f.publish(&pending, &stage);
        // Drop the process-owned store after member commit but before final ready.
        drop(f.store);
        f.store = SqliteStore::new(f.root.path().join("runtime")).unwrap();
        f.store.open_recent(&f.pid).unwrap();
        assert_eq!(
            f.store.query_result(&f.pid, &pending.id).unwrap().state,
            ResultState::Interrupted
        );
        f.verify(&first, 1);
        let retry = f.begin(2);
        f.publish(&retry, &stage);
        f.finish(&retry);
        f.verify(&retry, u64::from(!change));
    }
}

#[test]
fn unsealed_cancelled_and_out_of_order_stages_leave_no_partial_members() {
    let f = Fixture::new(1);
    let first = f.begin(1);
    let mut stage = f.stage();
    stage.full_source(&f.sources[0]);
    stage.append(&[f.key(0, 1)], &[None], 1).unwrap();
    assert_eq!(
        f.store
            .publish_stage(&f.pid, &first.id, &stage, "full", &AtomicBool::new(false))
            .unwrap_err()
            .code,
        "INVALID_INPUT"
    );
    stage.seal().unwrap();
    assert_eq!(
        f.store
            .publish_stage(&f.pid, &first.id, &stage, "full", &AtomicBool::new(true))
            .unwrap_err()
            .code,
        "CANCELLED"
    );
    let handle = f.store.handle(&f.pid).unwrap();
    handle
        .db
        .lock()
        .unwrap()
        .execute(
            "UPDATE query_results SET member_revision=5 WHERE id=?1",
            [&first.id],
        )
        .unwrap();
    assert_eq!(
        f.store
            .publish_stage(&f.pid, &first.id, &stage, "full", &AtomicBool::new(false))
            .unwrap_err()
            .code,
        "QUERY_PUBLICATION_CONFLICT"
    );
    f.verify(&first, 0);
    handle
        .db
        .lock()
        .unwrap()
        .execute(
            "UPDATE query_results SET member_revision=1 WHERE id=?1",
            [&first.id],
        )
        .unwrap();
    f.publish(&first, &stage);
    f.finish(&first);
    f.verify(&first, 1);
}

#[test]
fn incremental_publication_work_stays_bounded_as_the_family_grows() {
    let mut samples = Vec::new();
    for rows in [10_000_i64, 100_000, 500_000] {
        let f = Fixture::new(1);
        let first = f.begin(1);
        let handle = f.store.handle(&f.pid).unwrap();
        {
            let mut db = handle.db.lock().unwrap();
            let tx = db.transaction().unwrap();
            tx.execute(
                "WITH RECURSIVE n(x) AS(VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<?3)
                INSERT INTO query_member_data(family_id,source_id,asset_id,valid_from,post_id)
                SELECT ?1,?2,printf('%08d',x),1,x FROM n",
                params![first.id, f.sources[0], rows],
            )
            .unwrap();
            tx.execute(
                "UPDATE query_results SET count=?2 WHERE id=?1",
                params![first.id, rows],
            )
            .unwrap();
            tx.execute(
                "UPDATE query_families SET stored_members=?2 WHERE id=?1",
                params![first.id, rows],
            )
            .unwrap();
            tx.commit().unwrap();
            db.execute_batch("ANALYZE").unwrap();
        }
        f.finish(&first);
        let next = f.begin(2);
        let mut stage = f.stage();
        stage.affected(&[f.key(0, 1)]).unwrap();
        stage.append(&[f.key(0, 1)], &[Some(1)], 1).unwrap();
        stage.seal().unwrap();
        let ticks = Arc::new(AtomicUsize::new(0));
        let observed = ticks.clone();
        handle
            .db
            .lock()
            .unwrap()
            .progress_handler(
                1,
                Some(move || {
                    observed.fetch_add(1, Ordering::Relaxed);
                    false
                }),
            )
            .unwrap();
        f.publish(&next, &stage);
        handle
            .db
            .lock()
            .unwrap()
            .progress_handler(0, None::<fn() -> bool>)
            .unwrap();
        let steps = ticks.load(Ordering::Relaxed);
        assert!(steps > 0, "publication VM observer was not active");
        assert!(
            steps < 50_000,
            "one-key publication scanned the family: {rows} rows / {steps} VM steps"
        );
        f.finish(&next);
        f.verify(&next, rows as u64);
        let db = handle.db.lock().unwrap();
        db.execute(
            "ATTACH DATABASE ?1 AS query_stage",
            [stage.file.path().to_string_lossy().as_ref()],
        )
        .unwrap();
        let plan = db
            .prepare(&format!("EXPLAIN QUERY PLAN {CLOSE_DELTA}"))
            .unwrap()
            .query_map(params![first.id, f.sources[0], 2], |r| {
                r.get::<_, String>(3)
            })
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            plan.iter()
                .any(|line| line.contains("family_id=? AND source_id=? AND asset_id=?")),
            "{plan:?}"
        );
        let old_update = CLOSE_DELTA.replace("asset_id IN (SELECT asset_id FROM query_stage.affected WHERE source_id=?2)", "EXISTS(SELECT 1 FROM query_stage.affected a WHERE a.source_id=m.source_id AND a.asset_id=m.asset_id)");
        let old_plan = db
            .prepare(&format!("EXPLAIN QUERY PLAN {old_update}"))
            .unwrap()
            .query_map(params![first.id, f.sources[0], 2], |r| {
                r.get::<_, String>(3)
            })
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        let old_ticks = Arc::new(AtomicUsize::new(0));
        let observed = old_ticks.clone();
        db.progress_handler(
            100,
            Some(move || {
                observed.fetch_add(1, Ordering::Relaxed);
                false
            }),
        )
        .unwrap();
        assert_eq!(
            db.execute(&old_update, params![first.id, f.sources[0], 2])
                .unwrap(),
            0
        );
        let old_steps = old_ticks.swap(0, Ordering::Relaxed) * 100;
        let _: u64 = db.query_row("SELECT count(*) FROM query_member_data WHERE family_id=?1 AND valid_from<=?2 AND (valid_until IS NULL OR valid_until>?2)", params![first.id, 2], |r| unsigned(r, 0)).unwrap();
        let count_steps = old_ticks.load(Ordering::Relaxed) * 100;
        db.progress_handler(0, None::<fn() -> bool>).unwrap();
        db.execute_batch("DETACH DATABASE query_stage").unwrap();
        let source_id: String = db
            .query_row("SELECT sqlite_source_id()", [], |r| r.get(0))
            .unwrap();
        samples.push(serde_json::json!({"rows": rows, "affected": 1, "new_entire_publication_vm_steps_approx": steps,
            "old_close_vm_steps_approx": old_steps, "old_count_vm_steps_approx": count_steps,
            "new_close_plan": plan, "old_close_plan": old_plan, "sqlite_version": rusqlite::version(), "sqlite_source_id": source_id}));
    }
    eprintln!(
        "publication_probe={}",
        serde_json::to_string(&samples).unwrap()
    );
}
