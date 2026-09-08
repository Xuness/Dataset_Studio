use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use studio_domain::{Error, Result, validate_id};
use studio_storage::{QueryCachePolicy, QueryCacheStats, SqliteStore};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    pub quota_mib: u32,
    pub max_age_days: u32,
}
impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            quota_mib: 4096,
            max_age_days: 7,
        }
    }
}
impl CacheConfig {
    fn validate(&self) -> Result<()> {
        if self.quota_mib > 65536 || !(1..=90).contains(&self.max_age_days) {
            return Err(Error::invalid(
                "查询缓存须为 0–65536 MiB，保留时间须为 1–90 天",
            ));
        }
        Ok(())
    }
    pub fn policy(&self) -> QueryCachePolicy {
        QueryCachePolicy {
            quota_bytes: u64::from(self.quota_mib) << 20,
            max_age_seconds: u64::from(self.max_age_days) * 86400,
        }
    }
}
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
}
pub struct CacheControl {
    pub gate: Mutex<()>,
    path: PathBuf,
    config: Mutex<CacheConfig>,
    leases: Mutex<HashMap<(String, String, String), Instant>>,
    pub requested: AtomicBool,
    pub force: AtomicBool,
    pub busy: AtomicBool,
    pub reclaimed: AtomicU64,
    catalog_path: PathBuf,
    catalog: Mutex<HashMap<String, CachedProject>>,
}
impl CacheControl {
    pub fn open(path: PathBuf) -> Result<Self> {
        let config = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<CacheConfig>(&bytes).map_err(Error::io)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => CacheConfig::default(),
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
            path,
            config: Mutex::new(config),
            leases: Mutex::new(HashMap::new()),
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
        };
        if catalog.get(id) == Some(&value) {
            return Ok(());
        }
        catalog.insert(id.into(), value);
        studio_storage::atomic_json(&self.catalog_path, &*catalog)
    }
    pub fn track(&self, store: &SqliteStore, id: &str) -> Result<()> {
        self.record(id, store.directory(id)?, store.query_cache_stats(id)?)
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
        self.force.store(true, Ordering::Release);
        self.requested.store(true, Ordering::Release);
    }
    pub fn lease(&self, pid: &str, rid: &str, id: &str) -> Result<()> {
        for value in [pid, rid, id] {
            validate_id(value)?;
        }
        let mut leases = self
            .leases
            .lock()
            .map_err(|_| Error::new("INTERNAL_ERROR", "查询读取占用不可用"))?;
        leases.retain(|_, until| *until > Instant::now());
        let key = (pid.into(), rid.into(), id.into());
        if leases.len() >= 1024 && !leases.contains_key(&key) {
            return Err(Error::new("READ_BUDGET_EXCEEDED", "查询读取占用过多"));
        }
        leases.insert(key, Instant::now() + Duration::from_secs(90));
        Ok(())
    }
    pub fn release(&self, pid: &str, rid: &str, id: &str) {
        if let Ok(mut leases) = self.leases.lock() {
            leases.remove(&(pid.into(), rid.into(), id.into()));
        }
    }
    pub fn recent(&self, pid: &str, rid: &str) {
        if let Ok(mut leases) = self.leases.lock() {
            leases.retain(|_, until| *until > Instant::now());
            if leases.len() >= 1024
                && !leases.contains_key(&(pid.into(), rid.into(), "recent".into()))
            {
                let oldest = leases
                    .iter()
                    .filter(|((_, _, id), _)| id == "recent")
                    .min_by_key(|(_, until)| *until)
                    .map(|(key, _)| key.clone());
                if let Some(oldest) = oldest {
                    leases.remove(&oldest);
                } else {
                    return;
                }
            }
            leases.insert(
                (pid.into(), rid.into(), "recent".into()),
                Instant::now() + Duration::from_secs(10),
            );
        }
    }
    pub fn live(&self, pid: &str) -> HashSet<String> {
        self.leases
            .lock()
            .map(|mut leases| {
                leases.retain(|_, until| *until > Instant::now());
                leases
                    .keys()
                    .filter(|(p, _, _)| p == pid)
                    .map(|(_, r, _)| r.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
}
