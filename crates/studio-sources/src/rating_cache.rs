//! Application-owned current-post candidates shared by every project using a lake.
//! Observation identities preserve conjunction semantics when an image has several posts.
use crate::{
    danbooru::{Catalog, err},
    duckdb::{Runtime, Session},
    query::{ChangeAnchor, analysis_sequence, changes},
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::Serialize;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::{SystemTime, UNIX_EPOCH},
};
use studio_domain::*;

pub const RATINGS: [&str; 4] = ["g", "s", "q", "e"];
const OWNER: &str = "Dataset Studio rating candidates v1\n";

pub fn rating_candidates(spec: &QuerySpec) -> Option<Vec<String>> {
    if spec.observation_rule != ObservationRule::CurrentPost {
        return None;
    }
    let mut ratings: Option<Vec<String>> = None;
    for condition in spec.conditions.iter().filter(|c| c.field == "rating") {
        let values = match (&condition.operator, &condition.value) {
            (QueryOperator::Eq, Some(QueryValue::Text(value))) => vec![value.clone()],
            (QueryOperator::In, Some(QueryValue::TextList(values))) => values.clone(),
            _ => return None,
        };
        if values.iter().any(|v| !RATINGS.contains(&v.as_str())) {
            return None;
        }
        ratings = Some(match ratings {
            None => values,
            Some(previous) => previous
                .into_iter()
                .filter(|v| values.contains(v))
                .collect(),
        });
    }
    ratings
        .map(|mut values| {
            values.sort();
            values.dedup();
            values
        })
        .filter(|v| !v.is_empty())
}

// SQLite instr is case-sensitive. Literal-space token boundaries retain the
// source query semantics, including NULLs and values containing a space.
fn tag_predicates(spec: &QuerySpec) -> Option<(String, Vec<String>)> {
    let mut clauses = Vec::new();
    let mut values = Vec::new();
    for condition in &spec.conditions {
        if condition.field == "rating" {
            continue;
        }
        if condition.field != "tags" {
            return None;
        }
        if condition.operator == QueryOperator::IsMissing {
            clauses.push("tags IS NULL".to_owned());
            continue;
        }
        if condition.operator == QueryOperator::IsPresent {
            clauses.push("tags IS NOT NULL".to_owned());
            continue;
        }
        let tags = match (&condition.operator, &condition.value) {
            (QueryOperator::HasTag, Some(QueryValue::Text(value))) => vec![value.clone()],
            (
                QueryOperator::HasAllTags | QueryOperator::HasAnyTags | QueryOperator::HasNoTags,
                Some(QueryValue::TextList(values)),
            ) => values.clone(),
            _ => return None,
        };
        let mut tests = Vec::new();
        for tag in tags {
            if tag.contains(' ') {
                tests.push("0".to_owned());
            } else {
                values.push(format!(" {tag} "));
                tests.push(format!("instr(' '||tags||' ',?{})>0", values.len()));
            }
        }
        let expression = if tests.is_empty() {
            if condition.operator == QueryOperator::HasAllTags {
                "1".into()
            } else {
                "0".into()
            }
        } else {
            tests.join(if condition.operator == QueryOperator::HasAllTags {
                " AND "
            } else {
                " OR "
            })
        };
        let expression = if condition.operator == QueryOperator::HasNoTags {
            format!("NOT ({expression})")
        } else {
            expression
        };
        clauses.push(format!("tags IS NOT NULL AND ({expression})"));
    }
    Some((
        if clauses.is_empty() {
            "1=1".into()
        } else {
            clauses
                .into_iter()
                .map(|v| format!("({v})"))
                .collect::<Vec<_>>()
                .join(" AND ")
        },
        values,
    ))
}

#[derive(Debug, Clone, Serialize)]
pub struct RatingCacheEntry {
    pub source_id: String,
    pub rating: String,
    pub generation: String,
    pub sequence: u64,
    pub records: u64,
    pub bytes: u64,
    pub last_used_millis: u64,
    pub refreshed_records: u64,
    pub incremental: bool,
    pub fixed: bool,
    pub active: bool,
}

pub struct RatingCache {
    root: PathBuf,
    root_gate: Mutex<bool>,
    gates: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    pins: Arc<Mutex<HashMap<String, usize>>>,
}
pub struct RatingCachePin {
    keys: Vec<String>,
    pins: Arc<Mutex<HashMap<String, usize>>>,
}
impl Drop for RatingCachePin {
    fn drop(&mut self) {
        if let Ok(mut pins) = self.pins.lock() {
            for key in &self.keys {
                if let Some(count) = pins.get_mut(key) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        pins.remove(key);
                    }
                }
            }
        }
    }
}

fn millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn gate_error() -> Error {
    Error::new("INTERNAL_ERROR", "分级基础缓存锁不可用")
}

impl RatingCache {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            root_gate: Mutex::new(false),
            gates: Mutex::new(HashMap::new()),
            pins: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    fn check_root(&self) -> Result<()> {
        let _guard = self.root_gate.lock().map_err(|_| gate_error())?;
        self.check_root_unlocked()
    }
    fn check_root_unlocked(&self) -> Result<()> {
        if std::fs::read_to_string(self.root.join("cache-owner.txt")).map_err(Error::io)? != OWNER {
            return Err(Error::new(
                "CACHE_PATH_INVALID",
                "分级缓存目录不属于 Dataset Studio",
            ));
        }
        Ok(())
    }
    fn prepare_root(&self) -> Result<()> {
        let mut recovered = self.root_gate.lock().map_err(|_| gate_error())?;
        std::fs::create_dir_all(&self.root).map_err(Error::io)?;
        let marker = self.root.join("cache-owner.txt");
        if !marker.exists() {
            if std::fs::read_dir(&self.root)
                .map_err(Error::io)?
                .next()
                .is_some()
            {
                return Err(Error::new(
                    "CACHE_PATH_INVALID",
                    "分级缓存目录必须为空或由 Dataset Studio 管理",
                ));
            }
            std::fs::write(marker, OWNER).map_err(Error::io)?;
        }
        self.check_root_unlocked()?;
        if !*recovered {
            // Recover only our unpublished files, before any build on this instance.
            // Completed bases keep their SQLite journals for normal crash recovery.
            for entry in std::fs::read_dir(&self.root).map_err(Error::io)? {
                let entry = entry.map_err(Error::io)?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with("rating-build-")
                    && (name.ends_with(".sqlite") || name.ends_with(".sqlite-journal"))
                    && entry.file_type().map_err(Error::io)?.is_file()
                    && entry
                        .path()
                        .canonicalize()
                        .map_err(Error::io)?
                        .starts_with(self.root.canonicalize().map_err(Error::io)?)
                {
                    std::fs::remove_file(entry.path()).map_err(Error::io)?;
                }
            }
            *recovered = true;
        }
        Ok(())
    }
    pub fn pin(&self, source: &str, ratings: &[String]) -> Result<RatingCachePin> {
        let mut keys = Vec::new();
        for rating in ratings {
            self.path(source, rating)?;
            keys.push(format!("{source}:{rating}"));
        }
        let mut pins = self.pins.lock().map_err(|_| gate_error())?;
        for key in &keys {
            *pins.entry(key.clone()).or_default() += 1;
        }
        Ok(RatingCachePin {
            keys,
            pins: self.pins.clone(),
        })
    }
    fn path(&self, source_id: &str, rating: &str) -> Result<PathBuf> {
        validate_id(source_id)?;
        if !RATINGS.contains(&rating) {
            return Err(Error::invalid("未知的基础分级"));
        }
        let path = self.root.join(format!("{source_id}-{rating}.sqlite"));
        if path.exists()
            && !path
                .canonicalize()
                .map_err(Error::io)?
                .starts_with(self.root.canonicalize().map_err(Error::io)?)
        {
            return Err(Error::new(
                "CACHE_PATH_INVALID",
                "分级缓存文件必须位于应用缓存目录中",
            ));
        }
        Ok(path)
    }
    fn gate(&self, source_id: &str, rating: &str) -> Result<Arc<Mutex<()>>> {
        let key = format!("{source_id}:{rating}");
        Ok(self
            .gates
            .lock()
            .map_err(|_| gate_error())?
            .entry(key)
            .or_default()
            .clone())
    }
    fn read(&self, source_id: &str, rating: &str) -> Result<Connection> {
        self.check_root()?;
        let db = Connection::open_with_flags(
            self.path(source_id, rating)?,
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(err)?;
        db.busy_timeout(std::time::Duration::from_millis(50))
            .map_err(err)?;
        Ok(db)
    }
    pub fn entries(&self) -> Result<Vec<RatingCacheEntry>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        self.check_root()?;
        let mut result = Vec::new();
        for file in std::fs::read_dir(&self.root).map_err(Error::io)? {
            let file = file.map_err(Error::io)?;
            let name = file.file_name().to_string_lossy().into_owned();
            let Some(stem) = name.strip_suffix(".sqlite") else {
                continue;
            };
            let Some((source, rating)) = stem.rsplit_once('-') else {
                continue;
            };
            if validate_id(source).is_err() || !RATINGS.contains(&rating) {
                continue;
            }
            if let Ok(db) = self.read(source, rating)
                && let Ok(mut entry) = stamp(&db, source, rating)
            {
                entry.bytes = match file.metadata() {
                    Ok(metadata) => metadata.len(),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(Error::io(error)),
                };
                entry.active = self
                    .pins
                    .lock()
                    .map_err(|_| gate_error())?
                    .contains_key(&format!("{source}:{rating}"));
                result.push(entry);
            }
        }
        result.sort_by(|a, b| (&a.source_id, &a.rating).cmp(&(&b.source_id, &b.rating)));
        Ok(result)
    }
    pub fn storage_bytes(&self) -> Result<u64> {
        self.measured_bytes(false)
    }
    pub fn working_bytes(&self) -> Result<u64> {
        self.measured_bytes(true)
    }
    fn measured_bytes(&self, temporary: bool) -> Result<u64> {
        if !self.root.exists() {
            return Ok(0);
        }
        self.check_root()?;
        let mut bytes = 0;
        for file in std::fs::read_dir(&self.root).map_err(Error::io)? {
            let file = file.map_err(Error::io)?;
            let name = file.file_name().to_string_lossy().into_owned();
            let working = name.starts_with("rating-build-") || name.ends_with("-journal");
            if working == temporary {
                match std::fs::symlink_metadata(file.path()) {
                    Ok(metadata) if metadata.is_file() => bytes += metadata.len(),
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(Error::io(error)),
                }
            }
        }
        Ok(bytes)
    }
    pub fn set_fixed(&self, source: &str, rating: &str, fixed: bool) -> Result<()> {
        self.check_root()?;
        let gate = self.gate(source, rating)?;
        let _guard = gate.lock().map_err(|_| gate_error())?;
        let db = Connection::open_with_flags(
            self.path(source, rating)?,
            OpenFlags::SQLITE_OPEN_READ_WRITE,
        )
        .map_err(err)?;
        stamp(&db, source, rating)?;
        db.execute(
            "INSERT OR REPLACE INTO state VALUES('fixed',?1)",
            [fixed.to_string()],
        )
        .map_err(err)?;
        Ok(())
    }
    pub fn remove(&self, source: &str, rating: &str, allow_fixed: bool) -> Result<bool> {
        let pins = self.pins.lock().map_err(|_| gate_error())?;
        if pins.contains_key(&format!("{source}:{rating}")) {
            return Ok(false);
        }
        let gate = self.gate(source, rating)?;
        let Ok(_guard) = gate.try_lock() else {
            return Ok(false);
        };
        let db = self.read(source, rating)?;
        if stamp(&db, source, rating)?.fixed && !allow_fixed {
            return Ok(false);
        }
        drop(db);
        std::fs::remove_file(self.path(source, rating)?).map_err(Error::io)?;
        Ok(true)
    }
    pub fn ensure(
        &self,
        source: &Source,
        rating: &str,
        memory: u64,
        scratch: &Path,
        cancelled: Arc<AtomicBool>,
    ) -> Result<RatingCacheEntry> {
        let gate = self.gate(&source.id, rating)?;
        let _guard = gate.lock().map_err(|_| gate_error())?;
        let target = self.path(&source.id, rating)?;
        let catalog = Catalog::open(source)?;
        self.prepare_root()?;
        if target.exists()
            && !target
                .canonicalize()
                .map_err(Error::io)?
                .starts_with(self.root.canonicalize().map_err(Error::io)?)
        {
            return Err(Error::invalid("基础缓存路径超出应用目录"));
        }
        let mut old = if target.is_file() {
            Connection::open(&target).ok()
        } else {
            None
        };
        let previous = old
            .as_ref()
            .and_then(|db| stamp(db, &source.id, rating).ok());
        if let (Some(db), Some(entry)) = (old.as_ref(), previous.as_ref())
            && entry.generation == catalog.generation
            && entry.sequence == catalog.sequence
        {
            if millis().saturating_sub(entry.last_used_millis) >= 60_000 {
                db.execute(
                    "INSERT OR REPLACE INTO state VALUES('used',?1)",
                    [millis().to_string()],
                )
                .map_err(err)?;
            }
            let mut result = stamp(db, &source.id, rating)?;
            result.bytes = std::fs::metadata(&target).map_err(Error::io)?.len();
            return Ok(result);
        }
        let sqlite_memory = (memory / 16).clamp(8 << 20, 128 << 20);
        let runtime = Runtime::default()
            .with_query_directory(scratch.to_owned())
            .with_query_memory(memory.saturating_sub(sqlite_memory));
        let native = runtime.open_query(&catalog.analysis_path()?, cancelled.clone())?;
        analysis_sequence(&native, &catalog)?;
        let anchor = changes::anchor(&native, &catalog)?;
        let previous_anchor = if let (Some(db), Some(entry)) = (&old, &previous) {
            db.query_row("SELECT value FROM state WHERE key='batch'", [], |r| {
                r.get::<_, String>(0)
            })
            .optional()
            .map_err(err)?
            .map(|batch_id| ChangeAnchor {
                generation: entry.generation.clone(),
                sequence: entry.sequence,
                batch_id,
            })
        } else {
            None
        };
        let delta = previous_anchor
            .as_ref()
            .map(|a| changes::change_sql(&native, &catalog, a))
            .transpose()?
            .flatten();
        let incremental = delta.is_some();
        let temporary = if incremental {
            None
        } else {
            Some(
                tempfile::Builder::new()
                    .prefix("rating-build-")
                    .suffix(".sqlite")
                    .tempfile_in(&self.root)
                    .map_err(Error::io)?,
            )
        };
        let mut db = if let Some(temp) = &temporary {
            Connection::open(temp.path()).map_err(err)?
        } else {
            old.take()
                .ok_or_else(|| Error::new("INTERNAL_ERROR", "分级增量基线缺失"))?
        };
        db.execute_batch(&format!("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA cache_size=-{}; CREATE TABLE IF NOT EXISTS members(row_id INTEGER PRIMARY KEY,sha BLOB NOT NULL,post_id INTEGER NOT NULL,tags TEXT); CREATE TABLE IF NOT EXISTS state(key TEXT PRIMARY KEY,value TEXT NOT NULL);",sqlite_memory/1024)).map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        let changed_predicate = if let Some(delta) = delta {
            native.query(&format!("CREATE TEMP TABLE studio_changed AS {delta}"))?;
            let mut remove = tx
                .prepare("DELETE FROM members WHERE sha=?1")
                .map_err(err)?;
            native.stream_ids("SELECT sha256 FROM studio_changed", &mut |ids| {
                studio_application::read_cancelled(&cancelled)?;
                for sha in ids {
                    remove
                        .execute([hex::decode(sha).map_err(Error::io)?])
                        .map_err(err)?;
                }
                Ok(())
            })?;
            " AND a.sha256 IN (SELECT sha256 FROM studio_changed)"
        } else {
            ""
        };
        let sql = format!(
            "SELECT a.sha256||':'||CAST(o.row_id AS VARCHAR)||':'||CAST(cp.post_id AS VARCHAR)||':'||CASE WHEN o.tag_string IS NULL THEN '0' ELSE '1'||o.tag_string END FROM current_posts cp JOIN assets a ON a.asset_id=cp.asset_id JOIN observations o ON o.row_id=cp.row_id WHERE a.sha256 IS NOT NULL AND o.rating='{rating}'{changed_predicate} ORDER BY o.row_id"
        );
        let mut refreshed = 0u64;
        {
            let mut insert = tx
                .prepare("INSERT OR REPLACE INTO members VALUES(?1,?2,?3,?4)")
                .map_err(err)?;
            native.stream_strings(&sql, 128 * 1024, &mut |rows| {
                studio_application::read_cancelled(&cancelled)?;
                for row in rows {
                    let values = row.splitn(4, ':').collect::<Vec<_>>();
                    if values.len() != 4 {
                        return Err(Error::new("SOURCE_FORMAT_ERROR", "分级候选记录格式无效"));
                    }
                    let sha = hex::decode(values[0]).map_err(Error::io)?;
                    if sha.len() != 32 {
                        return Err(Error::new("SOURCE_FORMAT_ERROR", "分级候选图片身份无效"));
                    }
                    let row_id = values[1].parse::<i64>().map_err(Error::io)?;
                    let post = values[2].parse::<i64>().map_err(Error::io)?;
                    let tags = if values[3] == "0" {
                        None
                    } else {
                        Some(values[3].strip_prefix('1').ok_or_else(|| {
                            Error::new("SOURCE_FORMAT_ERROR", "分级候选标签格式无效")
                        })?)
                    };
                    insert
                        .execute(params![row_id, sha, post, tags])
                        .map_err(err)?;
                    refreshed += 1;
                }
                Ok(())
            })?;
        }
        drop(native);
        tx.execute_batch("CREATE INDEX IF NOT EXISTS members_sha ON members(sha)")
            .map_err(err)?;
        let count: i64 = tx
            .query_row("SELECT count(*) FROM members", [], |r| r.get(0))
            .map_err(err)?;
        for (key, value) in [
            ("format", "2".into()),
            ("generation", catalog.generation.clone()),
            ("sequence", catalog.sequence.to_string()),
            ("count", count.to_string()),
            ("used", millis().to_string()),
            ("refreshed", refreshed.to_string()),
            ("incremental", incremental.to_string()),
            (
                "fixed",
                previous.as_ref().is_some_and(|v| v.fixed).to_string(),
            ),
            ("batch", anchor.map(|a| a.batch_id).unwrap_or_default()),
        ] {
            tx.execute(
                "INSERT OR REPLACE INTO state VALUES(?1,?2)",
                params![key, value],
            )
            .map_err(err)?;
        }
        catalog.verify_unchanged(source)?;
        studio_application::read_cancelled(&cancelled)?;
        tx.commit().map_err(err)?;
        let mut result = stamp(&db, &source.id, rating)?;
        drop(db);
        drop(old);
        if let Some(temp) = temporary {
            temp.as_file().sync_all().map_err(Error::io)?;
            temp.persist(&target).map_err(Error::io)?;
        }
        result.bytes = std::fs::metadata(&target).map_err(Error::io)?.len();
        Ok(result)
    }

    pub(crate) fn import(
        &self,
        source: &Source,
        ratings: &[String],
        native: &Session,
        generation: &str,
        sequence: u64,
        spec: &QuerySpec,
    ) -> Result<u64> {
        let (predicate, parameters) =
            tag_predicates(spec).unwrap_or_else(|| ("1=1".into(), Vec::new()));
        native.import_candidates(|append| {
            for rating in ratings {
                let gate = self.gate(&source.id, rating)?;
                let _guard = gate.lock().map_err(|_| gate_error())?;
                let db = self.read(&source.id, rating)?;
                db.execute_batch("BEGIN").map_err(err)?;
                db.progress_handler(10000, Some(native.cancellation_probe()))
                    .map_err(err)?;
                let entry = stamp(&db, &source.id, rating)?;
                if entry.generation != generation || entry.sequence != sequence {
                    return Err(Error::new(
                        "SOURCE_CHANGED",
                        "基础分级缓存版本已变化，请重新计算",
                    ));
                }
                // Scan Tag payloads sequentially; let the budgeted native
                // relation order matches, avoiding random reads of the Tag table.
                let sql = if predicate == "1=1" {
                    "SELECT row_id,sha FROM members ORDER BY sha,row_id".to_owned()
                } else {
                    format!("SELECT row_id,sha FROM members NOT INDEXED WHERE {predicate}")
                };
                let mut statement = db.prepare(&sql).map_err(err)?;
                let mut rows = statement
                    .query(rusqlite::params_from_iter(&parameters))
                    .map_err(err)?;
                loop {
                    let next = rows.next();
                    native.check_cancelled()?;
                    let Some(row) = next.map_err(err)? else {
                        break;
                    };
                    append(
                        row.get(0).map_err(err)?,
                        row.get_ref(1).map_err(err)?.as_blob().map_err(Error::io)?,
                    )?;
                }
            }
            Ok(())
        })
    }
    pub(crate) fn filters_metadata(spec: &QuerySpec) -> bool {
        tag_predicates(spec).is_some()
    }
}

fn stamp(db: &Connection, source_id: &str, rating: &str) -> Result<RatingCacheEntry> {
    let mut statement = db.prepare("SELECT key,value FROM state").map_err(err)?;
    let data = statement
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(err)?
        .collect::<std::result::Result<HashMap<_, _>, _>>()
        .map_err(err)?;
    let value = |key: &str| {
        data.get(key)
            .cloned()
            .ok_or_else(|| Error::new("CACHE_CORRUPT", "基础分级缓存状态不完整"))
    };
    if value("format")? != "2" {
        return Err(Error::new("CACHE_CORRUPT", "基础分级缓存版本不支持"));
    }
    Ok(RatingCacheEntry {
        source_id: source_id.into(),
        rating: rating.into(),
        generation: value("generation")?,
        sequence: value("sequence")?.parse().map_err(Error::io)?,
        records: value("count")?.parse().map_err(Error::io)?,
        bytes: 0,
        last_used_millis: value("used")?.parse().map_err(Error::io)?,
        refreshed_records: value("refreshed")?.parse().map_err(Error::io)?,
        incremental: value("incremental")? == "true",
        fixed: value("fixed")? == "true",
        active: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_removes_only_owned_unpublished_files_once() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("bases");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("foreign.sqlite"), b"keep").unwrap();
        let cache = RatingCache::new(root.clone());
        assert_eq!(cache.prepare_root().unwrap_err().code, "CACHE_PATH_INVALID");
        assert!(root.join("foreign.sqlite").exists());
        std::fs::write(root.join("cache-owner.txt"), OWNER).unwrap();
        std::fs::write(root.join("rating-build-orphan.sqlite"), b"discard").unwrap();
        std::fs::write(root.join("rating-build-orphan.sqlite-journal"), b"discard").unwrap();
        cache.prepare_root().unwrap();
        assert!(!root.join("rating-build-orphan.sqlite").exists());
        assert!(!root.join("rating-build-orphan.sqlite-journal").exists());
        assert!(root.join("foreign.sqlite").exists());
        std::fs::write(root.join("rating-build-active.sqlite"), b"keep").unwrap();
        cache.prepare_root().unwrap();
        assert!(root.join("rating-build-active.sqlite").exists());
        assert_eq!(cache.working_bytes().unwrap(), 4);
        assert_eq!(cache.storage_bytes().unwrap(), OWNER.len() as u64 + 4);
    }
}
