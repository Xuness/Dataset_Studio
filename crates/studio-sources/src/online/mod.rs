//! Versioned serving reads. No request opens the mutable analytical DuckDB file.
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use studio_domain::*;
mod dimensions;
pub(crate) mod metadata;
pub(crate) mod query;
pub(crate) mod ranking;
pub(crate) mod ranking_simple;
pub(crate) mod ranking_source;
pub(crate) mod raw;
#[cfg(test)]
mod tests;

pub const SCHEMA_VERSION: u32 = 2;
pub const VIEW_TTL_MS: u64 = 30 * 60 * 1000;
pub const SCHEMA: &str = include_str!("schema.sql");
static BUSY_ERRORS: AtomicU64 = AtomicU64::new(0);
static PROTOCOL_ERRORS: AtomicU64 = AtomicU64::new(0);
static LEASE_RETRIES: AtomicU64 = AtomicU64::new(0);
static LEASE_MAX_MS: AtomicU64 = AtomicU64::new(0);
/// Per-engine counters, including errors recovered by bounded lease retries.
pub fn contention_metrics() -> [u64; 4] {
    [
        &BUSY_ERRORS,
        &PROTOCOL_ERRORS,
        &LEASE_RETRIES,
        &LEASE_MAX_MS,
    ]
    .map(|v| v.load(Ordering::Relaxed))
}
type RecentLeases = HashMap<(PathBuf, String, u64, String), u64>;
fn recent_leases() -> &'static Mutex<RecentLeases> {
    static RECENT: OnceLock<Mutex<RecentLeases>> = OnceLock::new();
    RECENT.get_or_init(Default::default)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pointer {
    pub schema_version: u32,
    pub library_id: String,
    pub generation: String,
    pub file: String,
    pub site: String,
}
pub fn available(source: &Source) -> bool {
    source.index_root.as_ref().is_some_and(|root| {
        if root.join("ONLINE.json").is_file() {
            return true;
        }
        let marker = root.join("ONLINE-BUILD.json");
        if !marker.exists() {
            return false;
        }
        // A damaged/missing active pointer must not silently reopen stale native
        // indexes. Private, not-yet-activated builds can still serve legacy data.
        let state = fs::metadata(&marker)
            .ok()
            .filter(|m| m.len() <= 16384)
            .and_then(|_| fs::read(marker).ok())
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
        !matches!(
            state.as_ref().and_then(|v| v["state"].as_str()),
            Some("building" | "built" | "verified")
        )
    })
}
fn error(value: impl std::fmt::Display) -> Error {
    Error::new("SOURCE_FORMAT_ERROR", value.to_string())
}
pub(super) fn read_post_id(
    row: &rusqlite::Row<'_>,
    column: usize,
) -> rusqlite::Result<Option<i128>> {
    use rusqlite::types::ValueRef;
    match row.get_ref(column)? {
        ValueRef::Null => Ok(None),
        ValueRef::Integer(value) => Ok(Some(value as i128)),
        ValueRef::Text(value) => std::str::from_utf8(value)
            .ok()
            .and_then(|v| v.parse().ok())
            .map(Some)
            .ok_or_else(|| {
                rusqlite::Error::InvalidColumnType(
                    column,
                    "post_id".into(),
                    rusqlite::types::Type::Text,
                )
            }),
        value => Err(rusqlite::Error::InvalidColumnType(
            column,
            "post_id".into(),
            value.data_type(),
        )),
    }
}
pub(crate) fn sql_error(value: rusqlite::Error) -> Error {
    if value.sqlite_error_code() == Some(rusqlite::ErrorCode::FileLockingProtocolFailed) {
        PROTOCOL_ERRORS.fetch_add(1, Ordering::Relaxed);
    }
    match value.sqlite_error_code() {
        Some(rusqlite::ErrorCode::OperationInterrupted) => {
            Error::new("SOURCE_TIMEOUT", "在线读取已取消或超过时间预算")
        }
        Some(
            rusqlite::ErrorCode::DatabaseBusy
            | rusqlite::ErrorCode::DatabaseLocked
            | rusqlite::ErrorCode::FileLockingProtocolFailed,
        ) => {
            BUSY_ERRORS.fetch_add(1, Ordering::Relaxed);
            Error::new("SOURCE_BUSY", "在线发布繁忙，请重试当前页面")
        }
        _ => error(value),
    }
}
pub(crate) fn millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub fn pointer(source: &Source) -> Result<(Pointer, PathBuf)> {
    let root = source
        .index_root
        .as_ref()
        .ok_or_else(|| Error::invalid("缺少在线索引目录"))?
        .canonicalize()
        .map_err(Error::io)?;
    let path = root.join("ONLINE.json");
    if fs::metadata(&path).map_err(Error::io)?.len() > 16384 {
        return Err(error("在线指针过大"));
    }
    let pointer: Pointer =
        serde_json::from_slice(&fs::read(path).map_err(Error::io)?).map_err(error)?;
    if !matches!(
        (pointer.schema_version, pointer.site.as_str()),
        (2, "danbooru" | "yandere" | "gelbooru") | (3, "pixiv")
    ) || (!source.id.is_empty() && source.id != pointer.library_id)
        || source.kind != pointer.site
    {
        return Err(Error::new(
            "SOURCE_ID_MISMATCH",
            "在线索引的身份、站点或格式不匹配",
        ));
    }
    if Path::new(&pointer.file)
        .components()
        .any(|p| !matches!(p, Component::Normal(_)))
    {
        return Err(Error::invalid("在线数据库必须位于索引目录内"));
    }
    let file = root.join(&pointer.file).canonicalize().map_err(Error::io)?;
    if !file.starts_with(&root) {
        return Err(Error::invalid("在线数据库超出索引目录"));
    }
    // Keep canonical containment validation above. SQLite's Windows VFS treats
    // every leading double backslash as UNC, including a local verbatim drive.
    // Prefer the equivalent DOS path when it is safe (dunce preserves long,
    // reserved-name and actual UNC paths).
    Ok((pointer, dunce::simplified(&file).to_path_buf()))
}
pub(crate) fn parse_revision(pointer: &Pointer, value: &str) -> Result<u64> {
    let prefix = format!("online-v{}:{}:", pointer.schema_version, pointer.generation);
    let metadata_prefix = format!(
        "metadata-v{}:{}:{}:",
        pointer.schema_version, pointer.library_id, pointer.generation
    );
    let sequence = value
        .strip_prefix(&prefix)
        .or_else(|| value.strip_prefix(&metadata_prefix))
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v <= i64::MAX as u64)
        .ok_or_else(|| Error::new("VIEW_EXPIRED", "读取视图不属于当前在线库，请刷新视图"))?;
    Ok(sequence)
}
fn state(db: &Connection, key: &str) -> Result<String> {
    db.query_row("SELECT value FROM online_state WHERE key=?1", [key], |r| {
        r.get(0)
    })
    .map_err(sql_error)
}
fn number(db: &Connection, key: &str) -> Result<u64> {
    state(db, key)?.parse().map_err(error)
}

fn check_read(cancelled: &AtomicBool, deadline: Instant) -> Result<()> {
    studio_application::read_cancelled(cancelled)?;
    if Instant::now() >= deadline {
        return Err(Error::new("SOURCE_TIMEOUT", "在线读取超过请求时间预算"));
    }
    Ok(())
}

// SQLite's busy handler has no request context. Use short waits around the
// complete transaction so cancellation and deadlines also interrupt lease I/O.
fn lease_write<T>(
    path: &Path,
    cancelled: &AtomicBool,
    deadline: Instant,
    mut write: impl FnMut(&mut Connection) -> Result<T>,
) -> Result<T> {
    struct LeaseTiming(Instant);
    impl Drop for LeaseTiming {
        fn drop(&mut self) {
            LEASE_MAX_MS.fetch_max(self.0.elapsed().as_millis() as u64, Ordering::Relaxed);
        }
    }
    let _timing = LeaseTiming(Instant::now());
    check_read(cancelled, deadline)?;
    let mut db =
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE).map_err(sql_error)?;
    db.busy_timeout(Duration::from_millis(20))
        .map_err(sql_error)?;
    let until = deadline.min(Instant::now() + Duration::from_secs(3));
    loop {
        check_read(cancelled, deadline)?;
        match write(&mut db) {
            Err(e) if e.code == "SOURCE_BUSY" => {
                LEASE_RETRIES.fetch_add(1, Ordering::Relaxed);
                check_read(cancelled, deadline)?;
                if Instant::now() >= until {
                    return Err(e);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            result => return result,
        }
    }
}

pub struct Snapshot {
    pub(crate) db: Connection,
    pub pointer: Pointer,
    pub path: PathBuf,
    pub sequence: u64,
    pub latest_sequence: u64,
    pub count: u64,
    pub revision: String,
    pub(crate) post_order_ready: bool,
    cancelled: Arc<AtomicBool>,
    deadline: Instant,
}
impl Snapshot {
    pub fn open(
        source: &Source,
        revision: Option<&str>,
        cancelled: Arc<AtomicBool>,
        deadline: Option<Instant>,
    ) -> Result<Self> {
        Self::open_mapped(source, revision, cancelled, deadline, 1 << 30)
    }
    pub(crate) fn open_bulk(
        source: &Source,
        revision: Option<&str>,
        cancelled: Arc<AtomicBool>,
        deadline: Option<Instant>,
    ) -> Result<Self> {
        // This reserves virtual address space, not a heap buffer. Read-only
        // pages share the OS file cache; closing each bounded snapshot releases
        // its mapping, including before lake maintenance or relocation.
        Self::open_mapped(
            source,
            revision,
            cancelled,
            deadline,
            if cfg!(target_pointer_width = "64") {
                128 << 30
            } else {
                1 << 30
            },
        )
    }
    fn open_mapped(
        source: &Source,
        revision: Option<&str>,
        cancelled: Arc<AtomicBool>,
        deadline: Option<Instant>,
        mmap_bytes: i64,
    ) -> Result<Self> {
        let until = deadline.unwrap_or_else(|| Instant::now() + Duration::from_secs(8));
        check_read(&cancelled, until)?;
        let (pointer, path) = pointer(source)?;
        let db = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(sql_error)?;
        db.busy_timeout(Duration::from_millis(500))
            .map_err(sql_error)?;
        db.execute_batch("PRAGMA cache_size=-8192;")
            .map_err(sql_error)?;
        db.pragma_update(None, "mmap_size", mmap_bytes)
            .map_err(sql_error)?;
        if mmap_bytes > 1 << 30 {
            let limit: i64 = db
                .pragma_query_value(None, "mmap_size", |r| r.get(0))
                .map_err(sql_error)?;
            tracing::debug!(mmap_limit = limit, "ranking bulk SQLite mapping");
        }
        let version: u32 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(sql_error)?;
        if version != pointer.schema_version
            || state(&db, "library_id")? != pointer.library_id
            || state(&db, "generation")? != pointer.generation
        {
            return Err(error("在线数据库身份不匹配"));
        }
        let latest_sequence = number(&db, "served_seq")?;
        let sequence = revision
            .map(|v| parse_revision(&pointer, v))
            .transpose()?
            .unwrap_or(latest_sequence);
        if sequence < number(&db, "min_seq")? || sequence > latest_sequence {
            return Err(Error::new("VIEW_EXPIRED", "该浏览视图已经过期或尚未发布"));
        }
        Self::lease(
            &path,
            sequence,
            &format!("read/{sequence}"),
            "interactive",
            "read",
            false,
            (&cancelled, until),
        )?;
        let flag = cancelled.clone();
        db.progress_handler(
            1000,
            Some(move || flag.load(Ordering::Acquire) || Instant::now() >= until),
        )
        .map_err(sql_error)?;
        db.execute_batch("BEGIN").map_err(sql_error)?;
        let count_sql = if version == 3 {
            "SELECT CAST(json_extract(counts_json,'$.objects') AS INTEGER) FROM publications WHERE seq=?1 AND state='published'"
        } else {
            "SELECT objects_count FROM publications WHERE seq=?1"
        };
        let count: i64 = if sequence == 0 && version == 3 {
            0
        } else {
            db.query_row(count_sql, [sequence as i64], |r| r.get(0))
                .optional()
                .map_err(sql_error)?
                .ok_or_else(|| Error::new("VIEW_EXPIRED", "该版本未完整发布或已回收"))?
        };
        // Read-only main plus request-local views: all metadata relations resolve
        // at one retained version, while each request holds only a short WAL read.
        let post_order_ready = version != 3
            || (state(&db, "pixiv_post_order_version").is_ok_and(|v| v == "1")
                && number(&db, "pixiv_post_order_seq").is_ok_and(|seq| seq >= sequence));
        if version == 3 {
            db.execute_batch(&format!("CREATE TEMP VIEW visible_objects AS SELECT * FROM main.objects WHERE first_seq<={sequence} AND media_category='image';
                CREATE TEMP VIEW visible_assets AS SELECT * FROM main.assets WHERE commit_seq<={sequence};
                CREATE TEMP VIEW visible_works AS SELECT * FROM main.work_observations WHERE commit_seq<={sequence};
                CREATE TEMP VIEW visible_manifests AS SELECT * FROM main.media_manifests WHERE commit_seq<={sequence};
                CREATE TEMP VIEW visible_media AS SELECT * FROM main.media_entries WHERE commit_seq<={sequence};
                CREATE TEMP VIEW current_works AS SELECT * FROM main.work_versions WHERE valid_from<={sequence} AND (valid_until IS NULL OR valid_until>{sequence});
                CREATE TEMP VIEW current_media_assets AS SELECT * FROM main.media_asset_versions WHERE valid_from<={sequence} AND (valid_until IS NULL OR valid_until>{sequence});
                ")).map_err(sql_error)?;
            if post_order_ready {
                db.execute_batch(&format!("CREATE TEMP VIEW object_order AS SELECT sha256,post_id,page_ordinal FROM main.pixiv_object_order WHERE valid_from<={sequence} AND (valid_until IS NULL OR valid_until>{sequence});")).map_err(sql_error)?;
            } else {
                db.execute_batch("CREATE TEMP VIEW object_order AS SELECT sha256,NULL AS post_id,0 AS page_ordinal FROM visible_objects;").map_err(sql_error)?;
            }
        } else {
            db.execute_batch(&format!("CREATE TEMP VIEW visible_objects AS SELECT * FROM main.objects WHERE first_seq<={sequence};
            CREATE TEMP VIEW visible_assets AS SELECT * FROM main.assets WHERE commit_seq<={sequence};
            CREATE TEMP VIEW visible_observations AS SELECT * FROM main.observations WHERE commit_seq<={sequence};
            CREATE TEMP VIEW current_posts AS SELECT post_id,row_id,asset_id FROM main.post_versions WHERE valid_from<={sequence} AND (valid_until IS NULL OR valid_until>{sequence});
            CREATE TEMP VIEW object_order AS SELECT sha256,post_id,0 AS page_ordinal FROM main.object_versions WHERE valid_from<={sequence} AND (valid_until IS NULL OR valid_until>{sequence});")).map_err(sql_error)?;
        }
        Ok(Self {
            db,
            pointer: pointer.clone(),
            path,
            sequence,
            latest_sequence,
            count: count as u64,
            revision: format!("online-v{version}:{}:{sequence}", pointer.generation),
            post_order_ready,
            cancelled,
            deadline: until,
        })
    }
    fn check(&self) -> Result<()> {
        check_read(&self.cancelled, self.deadline)
    }
    /// End each bounded WAL snapshot while retaining the read-only connection
    /// and its file mapping. Repeatedly unmapping a large lake discards the
    /// process's page translations even when the OS file cache is still warm.
    pub(crate) fn next_bulk_page(&mut self) -> Result<()> {
        self.check()?;
        self.db.execute_batch("COMMIT;").map_err(sql_error)?;
        self.deadline = Instant::now() + Duration::from_secs(60);
        let until = self.deadline;
        let flag = self.cancelled.clone();
        self.db
            .progress_handler(
                1000,
                Some(move || flag.load(Ordering::Acquire) || Instant::now() >= until),
            )
            .map_err(sql_error)?;
        Self::lease(
            &self.path,
            self.sequence,
            &format!("read/{}", self.sequence),
            "interactive",
            "read",
            false,
            (&self.cancelled, until),
        )?;
        self.db.execute_batch("BEGIN;").map_err(sql_error)?;
        if self.sequence < number(&self.db, "min_seq")?
            || self.sequence > number(&self.db, "served_seq")?
        {
            return Err(Error::new("VIEW_EXPIRED", "全湖读取的保留版本已失效"));
        }
        if self.sequence > 0
            && !self
                .db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM publications WHERE seq=?1)",
                    [self.sequence as i64],
                    |r| r.get::<_, bool>(0),
                )
                .map_err(sql_error)?
        {
            return Err(Error::new("VIEW_EXPIRED", "全湖读取的发布记录已回收"));
        }
        Ok(())
    }
    pub fn latest(source: &Source) -> Result<Self> {
        Self::open(source, None, Arc::new(AtomicBool::new(false)), None)
    }
    fn lease(
        path: &Path,
        seq: u64,
        id: &str,
        owner: &str,
        purpose: &str,
        permanent: bool,
        control: (&AtomicBool, Instant),
    ) -> Result<()> {
        let (cancelled, deadline) = control;
        check_read(cancelled, deadline)?;
        let current = millis();
        let key = (path.to_path_buf(), id.to_owned(), seq, owner.to_owned());
        if !permanent
            && recent_leases()
                .lock()
                .map_err(error)?
                .get(&key)
                .is_some_and(|until| *until > current + VIEW_TTL_MS / 2)
        {
            return Ok(());
        }
        lease_write(path, cancelled, deadline, |db| {
            let tx = db
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(sql_error)?;
            if seq < number(&tx, "min_seq")? || seq > number(&tx, "served_seq")? {
                return Err(Error::new(
                    "VIEW_EXPIRED",
                    "该浏览视图已经过期，请刷新以使用最新数据",
                ));
            }
            let expires = (!permanent).then_some((current + VIEW_TTL_MS) as i64);
            let written=tx.execute("INSERT INTO leases VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET expires_ms=CASE WHEN leases.expires_ms IS NULL THEN NULL ELSE excluded.expires_ms END WHERE leases.seq=excluded.seq AND leases.owner=excluded.owner AND leases.purpose=excluded.purpose",params![id,seq as i64,expires,owner,purpose]).map_err(sql_error)?;
            if written != 1 {
                return Err(Error::new("LEASE_CONFLICT", "读取租约身份或版本不一致"));
            }
            tx.commit().map_err(sql_error)?;
            Ok(())
        })?;
        if !permanent {
            let mut recent = recent_leases().lock().map_err(error)?;
            if recent.len() >= 2048 {
                recent.retain(|_, v| *v > current);
                if recent.len() >= 2048 {
                    recent.clear();
                }
            }
            recent.insert(key, current + VIEW_TTL_MS);
        }
        Ok(())
    }
    pub fn retain(&self, id: &str, owner: &str, purpose: &str, permanent: bool) -> Result<()> {
        Self::lease(
            &self.path,
            self.sequence,
            id,
            owner,
            purpose,
            permanent,
            (&self.cancelled, self.deadline),
        )
    }
    pub fn release(
        source: &Source,
        id: &str,
        cancelled: &AtomicBool,
        deadline: Option<Instant>,
    ) -> Result<()> {
        let deadline = deadline.unwrap_or_else(|| Instant::now() + Duration::from_secs(3));
        check_read(cancelled, deadline)?;
        let (_, path) = pointer(source)?;
        lease_write(&path, cancelled, deadline, |db| {
            db.execute("DELETE FROM leases WHERE id=?1", [id])
                .map_err(sql_error)?;
            Ok(())
        })?;
        recent_leases()
            .lock()
            .map_err(error)?
            .retain(|(p, k, _, _), _| p != &path || k != id);
        Ok(())
    }
    pub fn version(&self, source: &Source) -> QuerySourceVersion {
        QuerySourceVersion {
            source_id: source.id.clone(),
            catalog_revision: self.revision.clone(),
            analysis_sequence: Some(self.sequence.to_string()),
            consistency: "retained_online_snapshot".into(),
            semantics_version: Some(format!(
                "{}:online-v{}",
                crate::profiles::descriptor(&source.kind)
                    .map(|s| s.semantics_version)
                    .unwrap_or_default(),
                self.pointer.schema_version
            )),
        }
    }
    pub fn post_ids(&self, keys: &[AssetKey]) -> Result<Vec<Option<i64>>> {
        let mut query = self
            .db
            .prepare_cached("SELECT post_id FROM object_order WHERE sha256=?1")
            .map_err(sql_error)?;
        keys.iter()
            .map(|k| {
                query
                    .query_row([&k.asset_id], |r| read_post_id(r, 0))
                    .optional()
                    .map(|v| v.flatten().and_then(|id| i64::try_from(id).ok()))
                    .map_err(sql_error)
            })
            .collect()
    }
    pub fn post_positions(&self, keys: &[AssetKey]) -> Result<Vec<(Option<i128>, i64)>> {
        self.require_post_order()?;
        let mut query = self
            .db
            .prepare_cached("SELECT post_id,page_ordinal FROM object_order WHERE sha256=?1")
            .map_err(sql_error)?;
        keys.iter()
            .map(|key| {
                query
                    .query_row([&key.asset_id], |row| {
                        Ok((read_post_id(row, 0)?, row.get(1)?))
                    })
                    .optional()
                    .map(|row| row.unwrap_or((None, 0)))
                    .map_err(sql_error)
            })
            .collect()
    }
    pub(crate) fn require_post_order(&self) -> Result<()> {
        if !self.post_order_ready && self.count > 0 {
            return Err(Error::new(
                "SOURCE_INDEX_NOT_READY",
                "Pixiv 作品排序索引尚未就绪，请更新数据湖在线索引",
            ));
        }
        Ok(())
    }
    pub(crate) fn strings(&self, sql: &str, limit: usize) -> Result<Vec<Vec<Option<String>>>> {
        let mut stmt = self.db.prepare(sql).map_err(sql_error)?;
        let n = stmt.column_count();
        let mut rows = stmt.query([]).map_err(sql_error)?;
        let mut output = Vec::new();
        let mut bytes = 0usize;
        while let Some(row) = rows.next().map_err(sql_error)? {
            if output.len() >= limit {
                return Err(Error::new(
                    "READ_BUDGET_EXCEEDED",
                    "在线元数据输出超过行数预算",
                ));
            }
            let values = (0..n)
                .map(|i| match row.get_ref(i).map_err(sql_error)? {
                    rusqlite::types::ValueRef::Null => Ok(None),
                    rusqlite::types::ValueRef::Integer(v) => Ok(Some(v.to_string())),
                    rusqlite::types::ValueRef::Real(v) => Ok(Some(v.to_string())),
                    rusqlite::types::ValueRef::Text(v) => {
                        Ok(Some(String::from_utf8(v.to_vec()).map_err(error)?))
                    }
                    rusqlite::types::ValueRef::Blob(v) => Ok(Some(hex::encode(v))),
                })
                .collect::<Result<Vec<_>>>()?;
            bytes += values.iter().flatten().map(String::len).sum::<usize>();
            if bytes > 4 << 20 {
                return Err(Error::new(
                    "READ_BUDGET_EXCEEDED",
                    "在线元数据输出超过字节预算",
                ));
            }
            output.push(values);
        }
        Ok(output)
    }
}
