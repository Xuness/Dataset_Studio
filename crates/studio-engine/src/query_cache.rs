pub use crate::cache_config::CacheConfig;
mod lease_clock;
use lease_clock::LeaseClock;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use studio_domain::{Error, QueryCacheTier, Result, new_id, validate_id};
use studio_storage::{QueryCacheRequest, QueryCacheStats, SqliteStore};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedProject {
    pub id: String,
    pub directory: PathBuf,
    pub bytes: u64,
    pub members: u64,
    pub retained: u64,
    pub protected: u64,
    pub reused: u64,
    pub incremental: u64,
    pub free_bytes: u64,
    pub touched: u64,
    #[serde(default)]
    pub long_term_bytes: u64,
    #[serde(default)]
    pub temporary_bytes: u64,
    #[serde(default)]
    pub long_term_families: u64,
    #[serde(default)]
    pub temporary_families: u64,
    #[serde(default)]
    pub fixed_bytes: u64,
    #[serde(default)]
    pub session_families: u64,
    #[serde(default)]
    pub oldest_long_term_millis: u64,
    #[serde(default)]
    pub oldest_temporary_millis: u64,
}
pub struct CacheControl {
    gate: Mutex<()>,
    lease_clock: LeaseClock,
    path: PathBuf,
    config: Mutex<CacheConfig>,
    leases: Mutex<HashMap<(String, String, String), CacheLease>>,
    sessions: Mutex<HashMap<(String, String), Duration>>,
    server_session: String,
    clear_tier: Mutex<Option<QueryCacheTier>>,
    pub requested: AtomicBool,
    pub force: AtomicBool,
    pub busy: AtomicBool,
    pub reclaimed: AtomicU64,
    catalog_path: PathBuf,
    catalog: Mutex<HashMap<String, CachedProject>>,
}
struct CacheLease {
    until: Duration,
    session: Option<String>,
}
pub struct CacheGuard<'a> {
    // Fields drop in declaration order: resume lease time before unlocking.
    _pause: lease_clock::Pause<'a>,
    _gate: MutexGuard<'a, ()>,
}
impl CacheControl {
    pub fn lock(&self) -> Result<CacheGuard<'_>> {
        let gate = self
            .gate
            .lock()
            .map_err(|_| Error::new("INTERNAL_ERROR", "查询缓存锁不可用"))?;
        Ok(CacheGuard {
            _pause: self.lease_clock.pause(),
            _gate: gate,
        })
    }
    pub fn open(path: PathBuf, preview_mib: u32) -> Result<Self> {
        let config = match std::fs::read(&path) {
            Ok(bytes) => {
                let raw: serde_json::Value = serde_json::from_slice(&bytes).map_err(Error::io)?;
                if raw.get("schema_version").is_some() {
                    serde_json::from_value::<CacheConfig>(raw).map_err(Error::io)?
                } else {
                    #[derive(Deserialize)]
                    struct Legacy {
                        quota_mib: u32,
                        max_age_days: u32,
                    }
                    let old: Legacy = serde_json::from_value(raw).map_err(Error::io)?;
                    let converted =
                        CacheConfig::legacy(old.quota_mib, old.max_age_days, preview_mib)?;
                    let backup = path.with_file_name("query-cache.legacy-v1.json");
                    if !backup.exists() {
                        std::fs::copy(&path, &backup).map_err(Error::io)?;
                    }
                    studio_storage::atomic_json(&path, &converted)?;
                    converted
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut value = CacheConfig::default();
                value.preview_mib = preview_mib;
                value.total_mib = value
                    .total_mib
                    .max(value.query_mib().saturating_add(preview_mib));
                value
            }
            Err(e) => return Err(Error::io(e)),
        };
        config.validate()?;
        let catalog_path = path.with_file_name("query-cache-catalog.json");
        let catalog = match std::fs::read(&catalog_path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(Error::io)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(e) => return Err(Error::io(e)),
        };
        Ok(Self {
            gate: Mutex::new(()),
            lease_clock: LeaseClock::new(),
            path,
            config: Mutex::new(config),
            leases: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
            server_session: new_id(),
            clear_tier: Mutex::new(None),
            requested: AtomicBool::new(false),
            force: AtomicBool::new(false),
            busy: AtomicBool::new(false),
            reclaimed: AtomicU64::new(0),
            catalog_path,
            catalog: Mutex::new(catalog),
        })
    }
    pub fn projects(&self) -> Result<Vec<CachedProject>> {
        self.catalog
            .lock()
            .map(|p| p.values().cloned().collect())
            .map_err(|_| Error::new("INTERNAL_ERROR", "查询缓存目录不可用"))
    }
    pub fn record(&self, id: &str, directory: PathBuf, stats: QueryCacheStats) -> Result<()> {
        validate_id(id)?;
        let mut catalog = self
            .catalog
            .lock()
            .map_err(|_| Error::new("INTERNAL_ERROR", "查询缓存目录不可用"))?;
        let value = CachedProject {
            id: id.into(),
            directory,
            bytes: stats.storage_bytes,
            members: stats.member_versions,
            retained: stats.retained_families,
            protected: stats.protected_results,
            reused: stats.reused_results,
            incremental: stats.incremental_results,
            free_bytes: stats.database_free_bytes,
            touched: stats.last_used_millis,
            long_term_bytes: stats.long_term_bytes,
            temporary_bytes: stats.temporary_bytes,
            long_term_families: stats.long_term_families,
            temporary_families: stats.temporary_families,
            fixed_bytes: stats.fixed_bytes,
            session_families: stats.session_families,
            oldest_long_term_millis: stats.oldest_long_term_millis,
            oldest_temporary_millis: stats.oldest_temporary_millis,
        };
        if catalog.get(id) == Some(&value) {
            return Ok(());
        }
        catalog.insert(id.into(), value);
        studio_storage::atomic_json(&self.catalog_path, &*catalog)
    }
    pub fn track(&self, store: &SqliteStore, id: &str) -> Result<()> {
        if let Some(stats) = store.try_query_cache_stats(id)? {
            self.record(id, store.directory(id)?, stats)?;
        } else {
            self.requested.store(true, Ordering::Release);
        }
        Ok(())
    }
    pub fn track_committed(&self, store: &SqliteStore, id: &str) {
        if let Err(error) = self.track(store, id) {
            self.requested.store(true, Ordering::Release);
            tracing::warn!(project_id=%id,%error,"query cache bookkeeping deferred");
        }
    }
    pub fn config(&self) -> Result<CacheConfig> {
        self.config
            .lock()
            .map(|c| c.clone())
            .map_err(|_| Error::new("INTERNAL_ERROR", "查询缓存设置不可用"))
    }
    pub fn configure(&self, config: CacheConfig) -> Result<()> {
        config.validate()?;
        let mut current = self
            .config
            .lock()
            .map_err(|_| Error::new("INTERNAL_ERROR", "查询缓存设置不可用"))?;
        studio_storage::atomic_json(&self.path, &config)?;
        *current = config;
        self.requested.store(true, Ordering::Release);
        Ok(())
    }
    pub fn clear(&self) {
        if let Ok(mut tier) = self.clear_tier.lock() {
            *tier = None;
        }
        self.force.store(true, Ordering::Release);
        self.requested.store(true, Ordering::Release);
    }
    pub fn request_clear(&self, tier: Option<QueryCacheTier>) -> Result<()> {
        *self
            .clear_tier
            .lock()
            .map_err(|_| Error::new("INTERNAL_ERROR", "清理策略不可用"))? = tier;
        self.force.store(true, Ordering::Release);
        self.requested.store(true, Ordering::Release);
        Ok(())
    }
    pub fn clear_target(&self) -> Result<Option<QueryCacheTier>> {
        self.clear_tier
            .lock()
            .map(|v| *v)
            .map_err(|_| Error::new("INTERNAL_ERROR", "清理策略不可用"))
    }
    pub fn session(&self, pid: &str, provided: Option<&str>) -> Result<String> {
        validate_id(pid)?;
        let id = provided.unwrap_or(&self.server_session);
        validate_id(id)?;
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| Error::new("INTERNAL_ERROR", "缓存会话状态不可用"))?;
        let now = self.lease_clock.now();
        sessions.retain(|_, until| *until > now);
        let key = (pid.into(), id.into());
        if sessions.len() >= 1024 && !sessions.contains_key(&key) {
            return Err(Error::new("RESOURCE_LIMIT", "缓存会话数量超过上限"));
        }
        sessions.insert(key, now + Duration::from_secs(90));
        Ok(id.into())
    }
    pub fn end_session(&self, pid: &str, provided: Option<&str>) {
        let session = provided.unwrap_or(&self.server_session);
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(&(pid.into(), provided.unwrap_or(&self.server_session).into()));
        }
        if let Ok(mut leases) = self.leases.lock() {
            leases.retain(|(p, _, _), lease| p != pid || lease.session.as_deref() != Some(session));
        }
        self.requested.store(true, Ordering::Release);
    }
    pub fn live_sessions(&self, pid: &str) -> Result<HashSet<String>> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| Error::new("INTERNAL_ERROR", "缓存会话状态不可用"))?;
        let now = self.lease_clock.now();
        sessions.retain(|_, until| *until > now);
        Ok(sessions
            .keys()
            .filter(|(p, _)| p == pid)
            .map(|(_, s)| s.clone())
            .collect())
    }
    pub fn request(&self, pid: &str, provided: Option<&str>) -> Result<QueryCacheRequest> {
        let session = self.session(pid, provided)?;
        let config = self.config()?;
        Ok(QueryCacheRequest {
            enabled: config.query_enabled(),
            session_id: Some(session),
            session_only: config.temporary_session_only,
            live_sessions: self.live_sessions(pid)?,
        })
    }
    pub fn lease(&self, pid: &str, rid: &str, id: &str, session: &str) -> Result<()> {
        for value in [pid, rid, id, session] {
            validate_id(value)?;
        }
        let mut leases = self
            .leases
            .lock()
            .map_err(|_| Error::new("INTERNAL_ERROR", "查询读取占用不可用"))?;
        let now = self.lease_clock.now();
        leases.retain(|_, lease| lease.until > now);
        let key = (pid.into(), rid.into(), id.into());
        if leases.len() >= 1024 && !leases.contains_key(&key) {
            return Err(Error::new("READ_BUDGET_EXCEEDED", "查询读取占用过多"));
        }
        leases.insert(
            key,
            CacheLease {
                until: now + Duration::from_secs(90),
                session: Some(session.into()),
            },
        );
        Ok(())
    }
    pub fn release(&self, pid: &str, rid: &str, id: &str) {
        if let Ok(mut leases) = self.leases.lock() {
            leases.remove(&(pid.into(), rid.into(), id.into()));
        }
    }
    pub fn recent(&self, pid: &str, rid: &str) {
        if let Ok(mut leases) = self.leases.lock() {
            let now = self.lease_clock.now();
            leases.retain(|_, lease| lease.until > now);
            if leases.len() >= 1024
                && !leases.contains_key(&(pid.into(), rid.into(), "recent".into()))
            {
                let oldest = leases
                    .iter()
                    .filter(|((_, _, id), _)| id == "recent")
                    .min_by_key(|(_, lease)| lease.until)
                    .map(|(key, _)| key.clone());
                if let Some(oldest) = oldest {
                    leases.remove(&oldest);
                } else {
                    return;
                }
            }
            leases.insert(
                (pid.into(), rid.into(), "recent".into()),
                CacheLease {
                    until: now + Duration::from_secs(10),
                    session: None,
                },
            );
        }
    }
    pub fn live(&self, pid: &str) -> HashSet<String> {
        self.leases
            .lock()
            .map(|mut leases| {
                let now = self.lease_clock.now();
                leases.retain(|_, lease| lease.until > now);
                leases
                    .keys()
                    .filter(|(p, _, _)| p == pid)
                    .map(|(_, r, _)| r.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod handoff_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_settings_are_backed_up_and_reopen_with_the_same_budgets() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("query-cache.json");
        let original = br#"{"quota_mib":2048,"max_age_days":5}"#;
        std::fs::write(&path, original).unwrap();
        let cache = CacheControl::open(path.clone(), 4096).unwrap();
        let migrated = cache.config().unwrap();
        assert_eq!(migrated.query_mib(), 2048);
        assert_eq!(migrated.temporary_idle_hours, 120);
        assert_eq!(migrated.long_term_idle_days, Some(5));
        assert_eq!(migrated.total_mib, 6144);
        assert_eq!(
            std::fs::read(path.with_file_name("query-cache.legacy-v1.json")).unwrap(),
            original
        );
        drop(cache);
        assert_eq!(
            CacheControl::open(path, 1024).unwrap().config().unwrap(),
            migrated
        );
    }
}
