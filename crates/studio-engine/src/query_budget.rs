use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, atomic::AtomicBool},
    time::Duration,
};
use studio_domain::{Error, QUERY_MEMORY_BYTES, Result};
use studio_resources::ReadCoordinator;
pub fn result_work_memory(total: u64) -> u64 {
    (total / 4).clamp(64 << 20, 2 << 30)
}

#[derive(Clone, Copy, Serialize, Deserialize)]
struct Config {
    memory_gib: u32,
}
impl Config {
    fn validate(self) -> Result<Self> {
        if !(1..=64).contains(&self.memory_gib) {
            return Err(Error::invalid("范围查询内存须为 1 至 64 GiB 的整数"));
        }
        Ok(self)
    }
    fn bytes(self) -> u64 {
        u64::from(self.memory_gib) << 30
    }
}
struct State {
    configured: Config,
    active: Option<Config>,
}
pub struct QueryBudget {
    path: PathBuf,
    resources: Arc<ReadCoordinator>,
    state: Mutex<State>,
    available: Condvar,
}
impl QueryBudget {
    pub fn open(path: PathBuf, resources: Arc<ReadCoordinator>) -> Result<Self> {
        let configured = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<Config>(&bytes)
                .map_err(Error::io)?
                .validate()?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config {
                memory_gib: (QUERY_MEMORY_BYTES >> 30) as u32,
            },
            Err(e) => return Err(Error::io(e)),
        };
        resources.set_query_memory(configured.bytes())?;
        Ok(Self {
            path,
            resources,
            state: Mutex::new(State {
                configured,
                active: None,
            }),
            available: Condvar::new(),
        })
    }
    pub fn status(&self) -> Result<(u64, Option<u64>)> {
        let state = self.state.lock().map_err(|_| lock_error())?;
        Ok((state.configured.bytes(), state.active.map(Config::bytes)))
    }
    pub fn configure(&self, memory_gib: u32) -> Result<()> {
        let configured = Config { memory_gib }.validate()?;
        let mut state = self.state.lock().map_err(|_| lock_error())?;
        studio_storage::atomic_json(&self.path, &configured)?;
        if state.active.is_none() {
            self.resources.set_query_memory(configured.bytes())?;
        }
        state.configured = configured;
        Ok(())
    }
    /// The worker uses this immutable snapshot for both admission and DuckDB.
    /// A setting changed during a build is applied when this lease is dropped.
    #[cfg(test)]
    pub fn begin(&self) -> Result<QueryBudgetLease<'_>> {
        let mut state = self.state.lock().map_err(|_| lock_error())?;
        if state.active.is_some() {
            return Err(Error::new("INTERNAL_ERROR", "已有范围查询正在使用预算"));
        }
        let memory_bytes = state.configured.bytes();
        self.resources.set_query_memory(memory_bytes)?;
        state.active = Some(state.configured);
        Ok(QueryBudgetLease {
            owner: self,
            memory_bytes,
        })
    }
    pub fn wait(&self, cancelled: &AtomicBool) -> Result<QueryBudgetLease<'_>> {
        loop {
            studio_application::read_cancelled(cancelled)?;
            let mut state = self.state.lock().map_err(|_| lock_error())?;
            if state.active.is_none() {
                let memory_bytes = state.configured.bytes();
                self.resources.set_query_memory(memory_bytes)?;
                state.active = Some(state.configured);
                return Ok(QueryBudgetLease {
                    owner: self,
                    memory_bytes,
                });
            }
            state = self
                .available
                .wait_timeout(state, Duration::from_millis(50))
                .map_err(|_| lock_error())?
                .0;
            drop(state);
        }
    }
}
fn lock_error() -> Error {
    Error::new("INTERNAL_ERROR", "查询预算状态不可用")
}
pub struct QueryBudgetLease<'a> {
    owner: &'a QueryBudget,
    pub memory_bytes: u64,
}
impl Drop for QueryBudgetLease<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.owner.state.lock() {
            state.active = None;
            self.owner.available.notify_all();
            if let Err(error) = self
                .owner
                .resources
                .set_query_memory(state.configured.bytes())
            {
                tracing::error!(%error, "query budget update failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studio_application::ReadResources;
    use studio_domain::{METADATA_MEMORY_BYTES, ReadClass};

    #[test]
    fn settings_are_persistent_and_wait_for_the_active_query() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("query-settings.json");
        let resources = Arc::new(ReadCoordinator::default());
        let settings = QueryBudget::open(path.clone(), resources.clone()).unwrap();
        let budget = || {
            resources
                .metrics()
                .into_iter()
                .find(|m| m.budget.class == ReadClass::NativeQuery)
                .unwrap()
                .budget
                .bytes
        };
        assert_eq!(settings.status().unwrap(), (QUERY_MEMORY_BYTES, None));
        let active = settings.begin().unwrap();
        settings.configure(2).unwrap();
        assert_eq!(active.memory_bytes, QUERY_MEMORY_BYTES);
        assert_eq!(
            settings.status().unwrap(),
            (2 << 30, Some(QUERY_MEMORY_BYTES))
        );
        assert_eq!(budget(), QUERY_MEMORY_BYTES + METADATA_MEMORY_BYTES);
        drop(active);
        assert_eq!(budget(), (2 << 30) + METADATA_MEMORY_BYTES);
        assert_eq!(
            QueryBudget::open(path, resources)
                .unwrap()
                .status()
                .unwrap(),
            (2 << 30, None)
        );
        for invalid in [0, 65, u32::MAX] {
            assert!(settings.configure(invalid).is_err());
        }
        assert_eq!(settings.status().unwrap(), (2 << 30, None));
    }
}
