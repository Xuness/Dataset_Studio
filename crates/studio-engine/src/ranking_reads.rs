use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// Only immutable-range identities and small progress records are retained. No
/// database handles, member lists or project leases outlive a request.
pub struct BoundedCache<T> {
    entries: Mutex<HashMap<String, (Instant, T)>>,
    capacity: usize,
}
impl<T: Clone> BoundedCache<T> {
    fn new(capacity: usize) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            capacity,
        }
    }
    pub fn get(&self, key: &str) -> Option<T> {
        let mut entries = self.entries.lock().ok()?;
        let (used, value) = entries.get_mut(key)?;
        if used.elapsed() > Duration::from_secs(1800) {
            entries.remove(key);
            return None;
        }
        *used = Instant::now();
        Some(value.clone())
    }
    #[cfg(test)]
    pub fn insert(&self, key: String, value: T) {
        self.insert_if(key, value, |_, _| true);
    }
    #[cfg(test)]
    pub fn insert_if(&self, key: String, value: T, replace: impl FnOnce(&T, &T) -> bool) {
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        if let Some((used, previous)) = entries.get_mut(&key)
            && used.elapsed() <= Duration::from_secs(1800)
            && !replace(previous, &value)
        {
            *used = Instant::now();
            return;
        }
        if !entries.contains_key(&key)
            && entries.len() >= self.capacity
            && let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, (used, _))| *used)
                .map(|(k, _)| k.clone())
        {
            entries.remove(&oldest);
        }
        entries.insert(key, (Instant::now(), value));
    }
    pub fn get_or_insert(&self, key: String, create: impl FnOnce() -> T) -> T {
        let Ok(mut entries) = self.entries.lock() else {
            return create();
        };
        if let Some((used, value)) = entries.get_mut(&key)
            && used.elapsed() <= Duration::from_secs(1800)
        {
            *used = Instant::now();
            return value.clone();
        }
        if entries.len() >= self.capacity
            && let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, (used, _))| *used)
                .map(|(k, _)| k.clone())
        {
            entries.remove(&oldest);
        }
        let value = create();
        entries.insert(key, (Instant::now(), value.clone()));
        value
    }
}
#[derive(Default)]
pub struct CountProgress {
    pub scanned: u64,
    pub count: u64,
}
pub struct RankingReadCache {
    pub counts: BoundedCache<Arc<Mutex<CountProgress>>>,
}
impl Default for RankingReadCache {
    fn default() -> Self {
        Self {
            counts: BoundedCache::new(64),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_immutable_caches_share_one_count_state() {
        let cache = BoundedCache::new(2);
        cache.insert("a".into(), "first".to_owned());
        cache.insert("b".into(), "second".to_owned());
        cache.insert("c".into(), "third".to_owned());
        assert_eq!(cache.entries.lock().unwrap().len(), 2);
        assert_eq!(cache.get("c").as_deref(), Some("third"));
        let state = RankingReadCache::default();
        let first = state.counts.get_or_insert("one".into(), || {
            Arc::new(Mutex::new(CountProgress::default()))
        });
        let second = state
            .counts
            .get_or_insert("one".into(), || panic!("duplicate count task"));
        assert!(Arc::ptr_eq(&first, &second));
    }
}
