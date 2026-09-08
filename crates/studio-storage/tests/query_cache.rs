use std::{collections::HashSet, sync::atomic::AtomicBool};
use studio_application::{ProjectRepository, QueryRepository, ScopeRepository};
use studio_domain::*;
use studio_storage::{QueryCachePolicy, QueryStage, SqliteStore};

struct Fixture {
    root: tempfile::TempDir,
    store: SqliteStore,
    pid: String,
    sid: String,
}
fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let store = SqliteStore::new(root.path().join("runtime")).unwrap();
    let project = store.create("缓存验证", None).unwrap();
    let sid = new_id();
    store
        .attach(
            &project.id,
            Source {
                id: sid.clone(),
                name: "源".into(),
                kind: "demo".into(),
                index_root: None,
                media_root: None,
            },
        )
        .unwrap();
    Fixture {
        root,
        store,
        pid: project.id,
        sid,
    }
}
fn spec(f: &Fixture) -> QuerySpec {
    QuerySpec {
        version: 1,
        source_ids: vec![f.sid.clone()],
        conditions: vec![],
        observation_rule: ObservationRule::CurrentPost,
        order: QueryOrder::AssetKeyAsc,
        input_scope: None,
    }
}
fn version(f: &Fixture, n: usize) -> Vec<QuerySourceVersion> {
    vec![QuerySourceVersion {
        source_id: f.sid.clone(),
        catalog_revision: format!("revision-{n}"),
        analysis_sequence: None,
        consistency: "fixture".into(),
    }]
}
fn key(f: &Fixture, n: usize) -> AssetKey {
    AssetKey {
        source_id: f.sid.clone(),
        asset_id: format!("{n:064x}"),
    }
}
fn begin(f: &Fixture, n: usize) -> QueryResult {
    let r = f
        .store
        .create_cached_result(&f.pid, None, spec(f), version(f, n), true)
        .unwrap();
    assert!(f.store.start_result(&f.pid, &r.id).unwrap());
    r
}
fn publish(f: &Fixture, r: &QueryResult, stage: &QueryStage, mode: &str) -> QueryResult {
    stage.seal().unwrap();
    f.store
        .publish_stage(&f.pid, &r.id, stage, mode, &AtomicBool::new(false))
        .unwrap();
    f.store.finish_result(&f.pid, &r.id, None).unwrap()
}
fn full(f: &Fixture) -> QueryResult {
    let r = begin(f, 1);
    let mut stage = QueryStage::new(f.root.path()).unwrap();
    stage.full_source(&f.sid);
    let keys = (1..=100).map(|n| key(f, n)).collect::<Vec<_>>();
    let posts = (1..=100).map(Some).collect::<Vec<_>>();
    stage.append(&keys, &posts, 100).unwrap();
    publish(f, &r, &stage, "full")
}

#[test]
fn equivalent_queries_share_members_but_keep_request_order_and_metadata() {
    let f = fixture();
    let first = full(&f);
    assert_eq!(first.count, Some(100));
    let mut reversed = spec(&f);
    reversed.order = QueryOrder::AssetKeyDesc;
    let reused = f
        .store
        .create_cached_result(&f.pid, None, reversed, version(&f, 1), true)
        .unwrap();
    assert_ne!(first.id, reused.id);
    assert_eq!(reused.cache.mode, "reused");
    assert_eq!(reused.state, ResultState::Ready);
    assert_eq!(
        f.store.query_cache_stats(&f.pid).unwrap().member_versions,
        100
    );
    assert_eq!(
        f.store
            .result_page(&f.pid, &reused.id, None, 5)
            .unwrap()
            .keys[0],
        key(&f, 100)
    );
    f.store.release_result(&f.pid, &first.id).unwrap();
    assert_eq!(
        f.store
            .result_page(&f.pid, &reused.id, None, 5)
            .unwrap()
            .keys
            .len(),
        5
    );
}

#[test]
fn incremental_versions_change_only_affected_members_and_keep_pinned_snapshots() {
    let f = fixture();
    let first = full(&f);
    f.store
        .change_selection_scope(
            &f.pid,
            0,
            &ScopeRef {
                project_id: f.pid.clone(),
                target: ScopeTarget::QueryResult {
                    result_id: first.id.clone(),
                },
            },
            ScopeOperation::Replace,
        )
        .unwrap();
    let next = begin(&f, 2);
    let mut stage = QueryStage::new(f.root.path()).unwrap();
    stage
        .affected(&[key(&f, 1), key(&f, 3), key(&f, 101)])
        .unwrap();
    stage
        .append(&[key(&f, 3), key(&f, 101)], &[Some(3), Some(101)], 2)
        .unwrap();
    let next = publish(&f, &next, &stage, "incremental");
    assert_eq!(next.count, Some(100));
    assert_eq!(next.cache.changed_members, 2);
    assert_eq!(
        f.store.query_cache_stats(&f.pid).unwrap().member_versions,
        101
    );
    assert_eq!(
        f.store.selection_keys(&f.pid, None, 2).unwrap()[0],
        key(&f, 1)
    );
    assert_eq!(
        f.store.result_page(&f.pid, &next.id, None, 2).unwrap().keys[0],
        key(&f, 2)
    );
    let policy = QueryCachePolicy {
        quota_bytes: 0,
        max_age_seconds: 0,
    };
    f.store
        .maintain_query_cache(&f.pid, &policy, &HashSet::from([next.id.clone()]), true)
        .unwrap();
    assert_eq!(
        f.store.query_result(&f.pid, &next.id).unwrap().state,
        ResultState::Ready
    );
    f.store
        .maintain_query_cache(&f.pid, &policy, &HashSet::new(), true)
        .unwrap();
    assert_eq!(
        f.store.query_result(&f.pid, &next.id).unwrap().state,
        ResultState::Released
    );
    assert_eq!(
        f.store.query_result(&f.pid, &first.id).unwrap().state,
        ResultState::Ready
    );
    assert_eq!(f.store.selection(&f.pid).unwrap().count, 100);
    assert_eq!(
        f.store.query_cache_stats(&f.pid).unwrap().member_versions,
        100
    );
}

#[test]
fn repeated_days_do_not_accumulate_full_result_copies() {
    let f = fixture();
    let mut previous = full(&f);
    for day in 2..=22 {
        let next = begin(&f, day);
        let mut stage = QueryStage::new(f.root.path()).unwrap();
        stage
            .affected(&[key(&f, day - 1), key(&f, day + 99)])
            .unwrap();
        stage
            .append(&[key(&f, day + 99)], &[Some((day + 99) as i64)], 1)
            .unwrap();
        let next = publish(&f, &next, &stage, "incremental");
        assert_eq!(next.count, Some(100));
        f.store.release_result(&f.pid, &previous.id).unwrap();
        let stats = f
            .store
            .maintain_query_cache(
                &f.pid,
                &QueryCachePolicy::default(),
                &HashSet::from([next.id.clone()]),
                false,
            )
            .unwrap();
        assert_eq!(stats.retained_families, 1);
        assert_eq!(stats.member_versions, 100);
        previous = next;
    }
}

#[test]
fn failed_refresh_restores_the_old_version_and_never_becomes_a_cache_hit() {
    let f = fixture();
    let first = full(&f);
    let next = begin(&f, 2);
    f.store
        .append_result(&f.pid, &next.id, &[key(&f, 101)], 1)
        .unwrap();
    f.store
        .finish_result(
            &f.pid,
            &next.id,
            Some(&Error::new("SOURCE_CHANGED", "测试中断")),
        )
        .unwrap();
    assert_eq!(
        f.store.query_cache_stats(&f.pid).unwrap().member_versions,
        100
    );
    assert_eq!(
        f.store
            .result_page(&f.pid, &first.id, None, 1)
            .unwrap()
            .keys[0],
        key(&f, 1)
    );
    let retry = f
        .store
        .create_cached_result(&f.pid, None, spec(&f), version(&f, 2), true)
        .unwrap();
    assert_eq!(retry.state, ResultState::Queued);
}

#[test]
fn post_ordering_is_global_stable_for_ties_and_keeps_unknown_ids_last() {
    let f = fixture();
    let r = begin(&f, 1);
    let mut stage = QueryStage::new(f.root.path()).unwrap();
    stage.full_source(&f.sid);
    let keys = (1..=5).map(|n| key(&f, n)).collect::<Vec<_>>();
    stage
        .append(&keys, &[Some(80), None, Some(10), Some(80), None], 5)
        .unwrap();
    let r = publish(&f, &r, &stage, "full");
    for (order, expected) in [
        (QueryOrder::PostIdAsc, vec![3, 1, 4, 2, 5]),
        (QueryOrder::PostIdDesc, vec![4, 1, 3, 5, 2]),
    ] {
        let mut after = None;
        let mut found = Vec::new();
        loop {
            let page = f
                .store
                .result_page_ordered(&f.pid, &r.id, after.as_ref(), 2, order)
                .unwrap();
            found.extend(page.keys.into_iter().map(|k| k.asset_id));
            after = page.next;
            if after.is_none() {
                break;
            }
        }
        assert_eq!(
            found,
            expected
                .into_iter()
                .map(|n| key(&f, n).asset_id)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn closed_project_cleanup_keeps_daily_registry_and_authoritative_state_unchanged() {
    let f = fixture();
    full(&f);
    let directory = f.store.directory(&f.pid).unwrap();
    f.store.close(&f.pid).unwrap();
    let before = f.store.list().unwrap();
    let policy = QueryCachePolicy {
        quota_bytes: 0,
        max_age_seconds: 0,
    };
    let stats =
        SqliteStore::maintain_closed_cache(f.store.root(), &f.pid, &directory, &policy, true)
            .unwrap()
            .unwrap();
    assert_eq!(stats.retained_families, 0);
    assert_eq!(stats.member_versions, 0);
    let after = f.store.list().unwrap();
    assert_eq!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(after).unwrap()
    );
    f.store.open_recent(&f.pid).unwrap();
    assert_eq!(f.store.selection(&f.pid).unwrap().count, 0);
    assert_eq!(f.store.sources(&f.pid).unwrap()[0].id, f.sid);
}

#[test]
fn disabled_retention_is_collected_after_the_view_expires() {
    let f = fixture();
    let r = f
        .store
        .create_cached_result(&f.pid, None, spec(&f), version(&f, 1), false)
        .unwrap();
    f.store.start_result(&f.pid, &r.id).unwrap();
    let mut stage = QueryStage::new(f.root.path()).unwrap();
    stage.full_source(&f.sid);
    stage.append(&[key(&f, 1)], &[Some(1)], 1).unwrap();
    let result = publish(&f, &r, &stage, "full");
    let live = HashSet::from([result.id.clone()]);
    f.store
        .maintain_query_cache(&f.pid, &QueryCachePolicy::default(), &live, false)
        .unwrap();
    assert_eq!(
        f.store.query_result(&f.pid, &result.id).unwrap().state,
        ResultState::Ready
    );
    f.store
        .maintain_query_cache(&f.pid, &QueryCachePolicy::default(), &HashSet::new(), false)
        .unwrap();
    assert_eq!(
        f.store.query_result(&f.pid, &result.id).unwrap().state,
        ResultState::Released
    );
    assert_eq!(
        f.store.query_cache_stats(&f.pid).unwrap().member_versions,
        0
    );
}
