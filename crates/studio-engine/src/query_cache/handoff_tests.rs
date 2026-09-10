use super::*;
use studio_application::{ProjectRepository, QueryRepository};
use studio_domain::{
    AssetKey, ObservationRule, QueryOrder, QueryResult, QuerySourceVersion, QuerySpec, ResultState,
    Source,
};
use studio_storage::QueryCachePolicy;

struct Fixture {
    store: SqliteStore,
    cache: CacheControl,
    pid: String,
    session: String,
    result: QueryResult,
    key: AssetKey,
    _root: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let store = SqliteStore::new(root.path().join("runtime")).unwrap();
        let pid = store.create("查询结果交接", None).unwrap().id;
        let sid = new_id();
        store
            .attach(
                &pid,
                Source {
                    id: sid.clone(),
                    name: "隔离来源".into(),
                    kind: "demo".into(),
                    index_root: None,
                    media_root: None,
                },
            )
            .unwrap();
        let cache = CacheControl::open(root.path().join("cache.json"), 0).unwrap();
        // Freeze the test wall clock; advance it without sleeping or a large dataset.
        cache.lease_clock.advance(Duration::ZERO);
        cache
            .configure(CacheConfig {
                temporary_session_only: true,
                ..cache.config().unwrap()
            })
            .unwrap();
        let session = new_id();
        let result = store
            .create_result_with_cache(
                &pid,
                None,
                QuerySpec {
                    version: 1,
                    source_ids: vec![sid.clone()],
                    conditions: vec![],
                    observation_rule: ObservationRule::CurrentPost,
                    order: QueryOrder::AssetKeyAsc,
                    input_scope: None,
                },
                vec![QuerySourceVersion {
                    source_id: sid.clone(),
                    catalog_revision: "fixture-1".into(),
                    analysis_sequence: None,
                    consistency: "fixture".into(),
                }],
                &cache.request(&pid, Some(&session)).unwrap(),
            )
            .unwrap();
        store.start_result(&pid, &result.id).unwrap();
        let key = AssetKey {
            source_id: sid,
            asset_id: "a".repeat(64),
        };
        store
            .append_result(&pid, &result.id, std::slice::from_ref(&key), 1)
            .unwrap();
        Self {
            store,
            cache,
            pid,
            session,
            result,
            key,
            _root: root,
        }
    }

    fn publish(&self) {
        let _gate = self.cache.lock().unwrap();
        let ready = self
            .store
            .finish_result(&self.pid, &self.result.id, None)
            .unwrap();
        assert_eq!(ready.state, ResultState::Ready);
        self.cache.recent(&self.pid, &ready.id);
        // Publication holds the cache lock while synchronous accounting runs.
        self.cache.lease_clock.advance(Duration::from_secs(120));
        self.cache.track_committed(&self.store, &self.pid);
    }

    fn sweep(&self, seconds: u64) {
        let _gate = self.cache.lock().unwrap();
        // Maintenance can exceed both the handoff grace and the viewing lease.
        self.cache.lease_clock.advance(Duration::from_secs(seconds));
        let policy = QueryCachePolicy {
            quota_bytes: 0,
            temporary_quota_bytes: 0,
            long_term_quota_bytes: 0,
            live_sessions: self.cache.live_sessions(&self.pid).unwrap(),
            ..self.cache.config().unwrap().policy()
        };
        self.store
            .maintain_query_cache(&self.pid, &policy, &self.cache.live(&self.pid), false)
            .unwrap();
    }

    fn state(&self, id: &str) -> ResultState {
        self.store.query_result(&self.pid, id).unwrap().state
    }

    fn assert_readable(&self, id: &str) {
        assert_eq!(self.state(id), ResultState::Ready);
        assert_eq!(
            self.store
                .query_result_for_session(
                    &self.pid,
                    id,
                    &self.cache.live_sessions(&self.pid).unwrap(),
                )
                .unwrap()
                .state,
            ResultState::Ready,
            "The session must survive the same blocked time as the result lease"
        );
        assert_eq!(
            self.store
                .result_page(&self.pid, id, None, 1)
                .unwrap()
                .keys
                .as_slice(),
            std::slice::from_ref(&self.key)
        );
    }
}

#[test]
fn slow_publication_and_cleanup_preserve_handoff_then_reclaim_after_view_release() {
    let f = Fixture::new();
    f.publish();
    f.sweep(180);
    f.cache.lease_clock.advance(Duration::from_secs(9));
    f.assert_readable(&f.result.id);

    let view = new_id();
    {
        let _gate = f.cache.lock().unwrap();
        f.cache
            .lease(&f.pid, &f.result.id, &view, &f.session)
            .unwrap();
    }
    f.cache.lease_clock.advance(Duration::from_secs(2));
    f.sweep(180);
    f.assert_readable(&f.result.id);

    f.cache.release(&f.pid, &f.result.id, &view);
    f.sweep(0);
    assert_eq!(f.state(&f.result.id), ResultState::Released);
    assert_eq!(
        f.store.query_cache_stats(&f.pid).unwrap().member_versions,
        0
    );
}

#[test]
fn unclaimed_result_expires_after_available_grace_without_being_revived_by_cleanup() {
    let f = Fixture::new();
    f.publish();
    f.cache.lease_clock.advance(Duration::from_secs(9));
    f.sweep(180);
    f.assert_readable(&f.result.id);

    f.cache.lease_clock.advance(Duration::from_secs(2));
    f.sweep(180);
    assert_eq!(f.state(&f.result.id), ResultState::Released);
    assert!(f.cache.live(&f.pid).is_empty());
}

#[test]
fn reused_ready_result_gets_its_own_grace_after_slow_response_accounting() {
    let f = Fixture::new();
    f.publish();
    f.cache.lease_clock.advance(Duration::from_secs(11));
    assert!(f.cache.live(&f.pid).is_empty());
    let reused = {
        let _gate = f.cache.lock().unwrap();
        let result = f
            .store
            .create_result_with_cache(
                &f.pid,
                None,
                f.result.spec.clone(),
                f.result.source_versions.clone(),
                &f.cache.request(&f.pid, Some(&f.session)).unwrap(),
            )
            .unwrap();
        assert_eq!(result.state, ResultState::Ready);
        assert_ne!(result.id, f.result.id);
        f.cache.recent(&f.pid, &result.id);
        f.cache.lease_clock.advance(Duration::from_secs(120));
        f.cache.track_committed(&f.store, &f.pid);
        result
    };
    f.sweep(180);
    f.assert_readable(&reused.id);
    assert_eq!(
        f.store.query_cache_stats(&f.pid).unwrap().member_versions,
        1
    );
    f.cache.lease_clock.advance(Duration::from_secs(11));
    f.sweep(0);
    assert_eq!(f.state(&reused.id), ResultState::Released);
}

#[test]
fn lock_error_unwind_resumes_time_and_expired_sessions_stay_expired() {
    let f = Fixture::new();
    f.publish();
    f.cache.lease_clock.advance(Duration::from_secs(91));
    let session = new_id();
    let result = new_id();
    let operation = || -> Result<()> {
        let _gate = f.cache.lock()?;
        f.cache.lease_clock.advance(Duration::from_secs(180));
        assert!(f.cache.live(&f.pid).is_empty());
        assert!(f.cache.live_sessions(&f.pid)?.is_empty());
        f.cache.session(&f.pid, Some(&session))?;
        f.cache.recent(&f.pid, &result);
        Err(Error::new("TEST_FAILURE", "模拟持锁操作提前返回"))
    };
    assert_eq!(operation().unwrap_err().code, "TEST_FAILURE");
    assert!(f.cache.live(&f.pid).contains(&result));
    assert_eq!(
        f.cache.live_sessions(&f.pid).unwrap(),
        HashSet::from([session])
    );
    f.cache.lease_clock.advance(Duration::from_secs(91));
    assert!(f.cache.live(&f.pid).is_empty());
    assert!(f.cache.live_sessions(&f.pid).unwrap().is_empty());
    assert!(f.cache.lock().is_ok());
}
