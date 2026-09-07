use fs2::FileExt;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions, ReadDir},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use studio_domain::*;

const MARKER: &str = "dataset-studio-rebuildable-preview-cache-v1\n";
const MAX_ENTRY: u64 = 16 << 20;
pub fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub fn preview_key(source: &Source, asset_id: &str, content_version: &str, edge: u32) -> String {
    // Library/content identities survive relinking and append-only index generations.
    // Change renderer/encoder tags when output semantics or decoder policy changes.
    hex::encode(Sha256::digest(
        serde_json::to_vec(&(
            "preview-key-v1",
            &source.kind,
            &source.id,
            asset_id,
            content_version,
            edge.clamp(96, 1600),
            "fit-no-crop-v1",
            "image-0.25-v1",
            "jpeg-q86-v1",
        ))
        .expect("cache key is serializable"),
    ))
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct CacheMetrics {
    pub directory: String,
    pub quota_bytes: u64,
    pub bytes: u64,
    pub entries: u64,
    pub pinned: usize,
    pub hits: u64,
    pub misses: u64,
    pub corrupt: u64,
    pub evicted: u64,
    pub writes: u64,
    pub read_bytes: u64,
    pub maintenance_removed: u64,
    pub maintenance_pending: bool,
    pub index_rebuilt: bool,
    pub clear_pending: bool,
}
struct State {
    db: Connection,
    objects: PathBuf,
    _lock: File,
    pins: HashMap<String, usize>,
    metrics: CacheMetrics,
    scan: Option<ReadDir>,
    settings: PathBuf,
}
#[derive(Clone)]
pub struct PreviewCache {
    inner: Arc<Mutex<State>>,
}
pub struct CachedPreview {
    pub bytes: Vec<u8>,
    pub verified_ms: u64,
    pub pin: CachePin,
}
pub struct CachePin {
    inner: Arc<Mutex<State>>,
    key: String,
}
impl Drop for CachePin {
    fn drop(&mut self) {
        if let Ok(mut state) = self.inner.lock() {
            if let Some(count) = state.pins.get_mut(&self.key) {
                *count -= 1;
                if *count == 0 {
                    state.pins.remove(&self.key);
                }
            }
            // A quota change may have been waiting for this last reader.
            let clear = state.metrics.clear_pending;
            let _ = state.trim(32, clear);
        }
    }
}
fn cache_error(error: impl std::fmt::Display) -> Error {
    Error::new("CACHE_IO_ERROR", error.to_string())
}
fn unsigned(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}
fn valid_key(key: &str) -> bool {
    key.len() == 64
        && key
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn open_index(path: &Path) -> rusqlite::Result<(Connection, u64, u64, u64)> {
    let db = Connection::open(path)?;
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
        CREATE TABLE IF NOT EXISTS entries(key TEXT PRIMARY KEY,bytes INTEGER NOT NULL,sha256 TEXT NOT NULL,verified_ms INTEGER NOT NULL,used_ms INTEGER NOT NULL) WITHOUT ROWID;
        CREATE INDEX IF NOT EXISTS cache_lru ON entries(used_ms,key);
        CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,value INTEGER NOT NULL);
        INSERT OR IGNORE INTO settings VALUES('quota_bytes',2147483648);")?;
    let (entries, bytes) = db.query_row(
        "SELECT COUNT(*),COALESCE(SUM(bytes),0) FROM entries",
        [],
        |r| Ok((unsigned(r, 0)?, unsigned(r, 1)?)),
    )?;
    let quota = db.query_row(
        "SELECT value FROM settings WHERE key='quota_bytes'",
        [],
        |r| unsigned(r, 0),
    )?;
    Ok((db, entries, bytes, quota))
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CacheSettings {
    schema_version: u32,
    quota_bytes: u64,
    #[serde(default)]
    clear_pending: bool,
}
fn save_settings(path: &Path, quota_bytes: u64, clear_pending: bool) -> Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| Error::new("CACHE_PATH_INVALID", "缓存配置目录无效"))?;
    let mut file = tempfile::Builder::new()
        .prefix(".settings-")
        .tempfile_in(directory)
        .map_err(cache_error)?;
    serde_json::to_writer(
        &mut file,
        &CacheSettings {
            schema_version: 1,
            quota_bytes,
            clear_pending,
        },
    )
    .map_err(cache_error)?;
    file.as_file().sync_all().map_err(cache_error)?;
    file.persist(path).map_err(cache_error)?;
    Ok(())
}
impl PreviewCache {
    /// `directory` must be an empty directory or one bearing our ownership marker.
    /// No source/project path is ever enumerated by maintenance or clearing.
    pub fn open(directory: &Path) -> Result<Self> {
        fs::create_dir_all(directory).map_err(cache_error)?;
        let directory = directory.canonicalize().map_err(cache_error)?;
        let marker = directory.join("cache-owner.txt");
        if !marker.exists() {
            if fs::read_dir(&directory)
                .map_err(cache_error)?
                .next()
                .is_some()
            {
                return Err(Error::new(
                    "CACHE_PATH_INVALID",
                    "缓存目录必须为空或由 Dataset Studio 管理",
                ));
            }
            fs::write(&marker, MARKER).map_err(cache_error)?;
        }
        if fs::read_to_string(&marker).map_err(cache_error)? != MARKER {
            return Err(Error::new("CACHE_PATH_INVALID", "缓存目录标记不匹配"));
        }
        for name in [
            "cache-owner.txt",
            "cache.lock",
            "cache.sqlite",
            "cache.sqlite-wal",
            "cache.sqlite-shm",
            "cache-settings.json",
        ] {
            let path = directory.join(name);
            if path.exists() && path.canonicalize().map_err(cache_error)? != path {
                return Err(Error::new("CACHE_PATH_INVALID", "缓存控制文件不能是链接"));
            }
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join("cache.lock"))
            .map_err(cache_error)?;
        lock.try_lock_exclusive()
            .map_err(|_| Error::new("CACHE_BUSY", "另一个引擎正在管理该缓存目录"))?;
        let objects = directory.join("objects");
        fs::create_dir_all(&objects).map_err(cache_error)?;
        if objects.canonicalize().map_err(cache_error)? != objects {
            return Err(Error::new("CACHE_PATH_INVALID", "缓存材料目录不能是链接"));
        }
        let mut index_rebuilt = false;
        let (db, entries, bytes, mut quota_bytes) =
            match open_index(&directory.join("cache.sqlite")) {
                Ok(index) => index,
                Err(error)
                    if matches!(
                        error.sqlite_error_code(),
                        Some(
                            rusqlite::ErrorCode::DatabaseCorrupt
                                | rusqlite::ErrorCode::NotADatabase
                        )
                    ) =>
                {
                    // Cache authority is disposable. Isolate the damaged index before
                    // rebuilding; bounded maintenance later removes unindexed previews.
                    let isolated = directory.join(format!("damaged-index-{}", new_id()));
                    fs::create_dir(&isolated).map_err(cache_error)?;
                    for name in ["cache.sqlite", "cache.sqlite-wal", "cache.sqlite-shm"] {
                        let old = directory.join(name);
                        if old.exists() {
                            fs::rename(old, isolated.join(name)).map_err(cache_error)?;
                        }
                    }
                    index_rebuilt = true;
                    open_index(&directory.join("cache.sqlite")).map_err(cache_error)?
                }
                Err(error) => return Err(cache_error(error)),
            };
        let settings = directory.join("cache-settings.json");
        let mut clear_pending = false;
        if settings.exists() {
            if fs::metadata(&settings).map_err(cache_error)?.len() > 4096 {
                return Err(Error::new("CACHE_FORMAT_UNSUPPORTED", "缓存配置过大"));
            }
            let saved: CacheSettings =
                serde_json::from_slice(&fs::read(&settings).map_err(cache_error)?)
                    .map_err(cache_error)?;
            if saved.schema_version != 1 || saved.quota_bytes > 1024 * 1024 * 1024 * 1024 {
                return Err(Error::new(
                    "CACHE_FORMAT_UNSUPPORTED",
                    "缓存配置版本或配额不受支持",
                ));
            }
            quota_bytes = saved.quota_bytes;
            clear_pending = saved.clear_pending;
        } else {
            save_settings(&settings, quota_bytes, false)?;
        }
        let cache = Self {
            inner: Arc::new(Mutex::new(State {
                db,
                objects: objects.clone(),
                _lock: lock,
                pins: HashMap::new(),
                metrics: CacheMetrics {
                    directory: directory
                        .to_string_lossy()
                        .trim_start_matches("\\\\?\\")
                        .into(),
                    entries,
                    bytes,
                    quota_bytes,
                    maintenance_pending: true,
                    index_rebuilt,
                    clear_pending,
                    ..Default::default()
                },
                scan: Some(fs::read_dir(objects).map_err(cache_error)?),
                settings,
            })),
        };
        cache.maintain(128)?;
        Ok(cache)
    }
    pub fn metrics(&self) -> CacheMetrics {
        self.inner
            .lock()
            .map(|s| {
                let mut m = s.metrics.clone();
                m.pinned = s.pins.len();
                m
            })
            .unwrap_or_default()
    }
    pub fn get(&self, key: &str, verified_online: bool) -> Result<Option<CachedPreview>> {
        if !valid_key(key) {
            return Err(Error::invalid("缓存身份无效"));
        }
        let mut state = self.inner.lock().map_err(cache_error)?;
        let entry: Option<(u64, String, u64)> = state
            .db
            .query_row(
                "SELECT bytes,sha256,verified_ms FROM entries WHERE key=?1",
                [key],
                |r| Ok((unsigned(r, 0)?, r.get(1)?, unsigned(r, 2)?)),
            )
            .optional()
            .map_err(cache_error)?;
        let Some((size, hash, mut verified_ms)) = entry else {
            state.metrics.misses += 1;
            return Ok(None);
        };
        let path = state.file(key);
        let bytes = if size <= MAX_ENTRY && path.canonicalize().ok().as_ref() == Some(&path) {
            fs::metadata(&path)
                .ok()
                .filter(|m| m.is_file() && m.len() == size)
                .and_then(|_| fs::read(&path).ok())
        } else {
            None
        };
        let Some(bytes) = bytes.filter(|b| hex::encode(Sha256::digest(b)) == hash) else {
            state.metrics.corrupt += 1;
            state.metrics.misses += 1;
            state.remove(key)?;
            return Ok(None);
        };
        if verified_online {
            verified_ms = epoch_ms();
        }
        state
            .db
            .execute(
                "UPDATE entries SET used_ms=?2,verified_ms=?3 WHERE key=?1",
                params![key, epoch_ms() as i64, verified_ms as i64],
            )
            .map_err(cache_error)?;
        state.metrics.hits += 1;
        state.metrics.read_bytes += bytes.len() as u64;
        *state.pins.entry(key.into()).or_default() += 1;
        Ok(Some(CachedPreview {
            bytes,
            verified_ms,
            pin: CachePin {
                inner: self.inner.clone(),
                key: key.into(),
            },
        }))
    }
    pub fn put(&self, key: &str, bytes: &[u8]) -> Result<Option<CachePin>> {
        if !valid_key(key) || bytes.len() as u64 > MAX_ENTRY {
            return Err(Error::invalid("缩略图缓存材料超出限制"));
        }
        let mut state = self.inner.lock().map_err(cache_error)?;
        state.ensure_objects()?;
        if bytes.len() as u64 > state.metrics.quota_bytes || state.metrics.clear_pending {
            return Ok(None);
        }
        if state.pins.contains_key(key) {
            return Ok(None);
        }
        // Write data first. A crash before the index commit leaves an orphan which
        // bounded maintenance removes; the index never claims a partial file.
        let mut file = tempfile::Builder::new()
            .prefix("partial-")
            .tempfile_in(&state.objects)
            .map_err(cache_error)?;
        file.write_all(bytes).map_err(cache_error)?;
        file.as_file().sync_all().map_err(cache_error)?;
        let path = state.file(key);
        if path
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink())
        {
            return Err(Error::new("CACHE_PATH_INVALID", "缓存材料不能是链接"));
        }
        file.persist(&path).map_err(cache_error)?;
        let prior: Option<u64> = state
            .db
            .query_row("SELECT bytes FROM entries WHERE key=?1", [key], |r| {
                unsigned(r, 0)
            })
            .optional()
            .map_err(cache_error)?;
        state.db.execute("INSERT INTO entries VALUES(?1,?2,?3,?4,?4) ON CONFLICT(key) DO UPDATE SET bytes=excluded.bytes,sha256=excluded.sha256,verified_ms=excluded.verified_ms,used_ms=excluded.used_ms", params![key,bytes.len() as i64,hex::encode(Sha256::digest(bytes)),epoch_ms() as i64]).map_err(cache_error)?;
        state.metrics.bytes = state.metrics.bytes - prior.unwrap_or(0) + bytes.len() as u64;
        state.metrics.entries += u64::from(prior.is_none());
        state.metrics.writes += 1;
        *state.pins.entry(key.into()).or_default() += 1;
        state.trim(128, false)?;
        Ok(Some(CachePin {
            inner: self.inner.clone(),
            key: key.into(),
        }))
    }
    pub fn set_quota(&self, bytes: u64) -> Result<CacheMetrics> {
        if bytes > 1024 * 1024 * 1024 * 1024 {
            return Err(Error::invalid("缓存配额最多 1 TiB"));
        }
        {
            let mut state = self.inner.lock().map_err(cache_error)?;
            save_settings(&state.settings, bytes, state.metrics.clear_pending)?;
            state
                .db
                .execute(
                    "UPDATE settings SET value=?1 WHERE key='quota_bytes'",
                    [bytes as i64],
                )
                .map_err(cache_error)?;
            state.metrics.quota_bytes = bytes;
            state.trim(256, false)?;
        }
        Ok(self.metrics())
    }
    /// Start durable, bounded clearing. The maintenance loop and last-pin release
    /// continue it, and new previews bypass retention until clearing completes.
    pub fn clear(&self) -> Result<CacheMetrics> {
        {
            let mut state = self.inner.lock().map_err(cache_error)?;
            save_settings(&state.settings, state.metrics.quota_bytes, true)?;
            state.metrics.clear_pending = true;
            state.scan = Some(fs::read_dir(&state.objects).map_err(cache_error)?);
            state.trim(128, true)?;
        }
        self.maintain(128)?;
        Ok(self.metrics())
    }
    pub fn maintain(&self, limit: usize) -> Result<()> {
        let mut state = self.inner.lock().map_err(cache_error)?;
        state.ensure_objects()?;
        let clear = state.metrics.clear_pending;
        state.trim(limit.min(256), clear)?;
        for _ in 0..limit.min(256) {
            let next = state.scan.as_mut().and_then(Iterator::next);
            let Some(entry) = next else {
                state.scan = None;
                break;
            };
            let entry = entry.map_err(cache_error)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let orphan = if let Some(key) = name.strip_suffix(".jpg").filter(|k| valid_key(k)) {
                !state
                    .db
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM entries WHERE key=?1)",
                        [key],
                        |r| r.get::<_, bool>(0),
                    )
                    .map_err(cache_error)?
            } else {
                name.starts_with("partial-")
            };
            if orphan {
                let kind = match entry.file_type() {
                    Ok(kind) => kind,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(cache_error(e)),
                };
                if kind.is_file() {
                    match fs::remove_file(entry.path()) {
                        Ok(()) => state.metrics.maintenance_removed += 1,
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => return Err(cache_error(e)),
                    }
                }
            }
        }
        state.finish_clear()?;
        state.metrics.maintenance_pending = state.scan.is_some()
            || state.metrics.clear_pending
            || state.metrics.bytes > state.metrics.quota_bytes;
        Ok(())
    }
}
impl State {
    fn ensure_objects(&self) -> Result<()> {
        if self.objects.canonicalize().map_err(cache_error)? != self.objects {
            return Err(Error::new(
                "CACHE_PATH_INVALID",
                "缓存材料目录已变化，不能继续清理或写入",
            ));
        }
        Ok(())
    }
    fn file(&self, key: &str) -> PathBuf {
        self.objects.join(format!("{key}.jpg"))
    }
    fn remove(&mut self, key: &str) -> Result<()> {
        self.ensure_objects()?;
        if !valid_key(key) {
            return Err(Error::new("CACHE_PATH_INVALID", "缓存索引身份无效"));
        }
        if self.pins.contains_key(key) {
            return Ok(());
        }
        let size: Option<u64> = self
            .db
            .query_row("SELECT bytes FROM entries WHERE key=?1", [key], |r| {
                unsigned(r, 0)
            })
            .optional()
            .map_err(cache_error)?;
        if let Some(size) = size {
            // Unlink only a validated fixed child filename. Never follow its target.
            let path = self.file(key);
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(cache_error(e)),
            }
            self.db
                .execute("DELETE FROM entries WHERE key=?1", [key])
                .map_err(cache_error)?;
            self.metrics.entries -= 1;
            self.metrics.bytes -= size;
        }
        Ok(())
    }
    fn trim(&mut self, limit: usize, clear: bool) -> Result<()> {
        if !clear && self.metrics.bytes <= self.metrics.quota_bytes {
            return Ok(());
        }
        let keys = self
            .db
            .prepare("SELECT key FROM entries ORDER BY used_ms,key LIMIT ?1")
            .map_err(cache_error)?
            .query_map([(limit + self.pins.len()) as i64], |r| {
                r.get::<_, String>(0)
            })
            .map_err(cache_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(cache_error)?;
        let mut removed = 0;
        for key in keys {
            if removed >= limit || (!clear && self.metrics.bytes <= self.metrics.quota_bytes) {
                break;
            }
            if self.pins.contains_key(&key) {
                continue;
            }
            self.remove(&key)?;
            self.metrics.evicted += 1;
            removed += 1;
        }
        self.finish_clear()?;
        Ok(())
    }
    fn finish_clear(&mut self) -> Result<()> {
        if self.metrics.clear_pending && self.metrics.entries == 0 && self.scan.is_none() {
            save_settings(&self.settings, self.metrics.quota_bytes, false)?;
            self.metrics.clear_pending = false;
        }
        Ok(())
    }
}
