use crate::query::ChangeAnchor;
use crate::{
    canonical::{Catalog, err},
    duckdb::Runtime,
    query::{analysis_sequence, changes},
};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, atomic::AtomicBool},
};
use studio_domain::*;

#[derive(Debug, Clone)]
pub struct BrowseIndexStamp {
    pub generation: String,
    pub sequence: u64,
    pub count: u64,
    pub bytes: u64,
    pub refreshed_objects: u64,
    pub incremental: bool,
}
pub struct BrowseIndex {
    root: PathBuf,
    gates: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}
pub struct BrowseIndexReader {
    db: BrowseBackend,
    pub stamp: BrowseIndexStamp,
}
enum BrowseBackend {
    Legacy(Connection),
    Online(Box<crate::online::Snapshot>),
}
impl std::ops::Deref for BrowseBackend {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        match self {
            Self::Legacy(db) => db,
            Self::Online(view) => &view.db,
        }
    }
}
impl BrowseIndexReader {
    pub fn revision(&self) -> String {
        match &self.db {
            BrowseBackend::Online(view) => view.revision.clone(),
            BrowseBackend::Legacy(_) => format!(
                "catalog-v1:{}:{}",
                self.stamp.generation, self.stamp.sequence
            ),
        }
    }
    pub fn page(
        &self,
        source_id: &str,
        order: QueryOrder,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(AssetKey, Option<i128>)>> {
        if let BrowseBackend::Online(view) = &self.db {
            let source = Source {
                id: source_id.into(),
                name: String::new(),
                kind: view.pointer.site.clone(),
                index_root: None,
                media_root: None,
            };
            let spec = QuerySpec {
                version: 3,
                source_ids: vec![source_id.into()],
                conditions: Vec::new(),
                observation_rule: ObservationRule::CurrentPost,
                order,
                input_scope: None,
            };
            return Ok(view
                .page_query(&source, &spec, after, limit)?
                .hits
                .into_iter()
                .map(|h| (h.key, h.post_id))
                .collect());
        }
        let digest = after.map(hex::decode).transpose().map_err(Error::io)?;
        let post: Option<i64> = if let Some(digest) = &digest {
            self.db
                .query_row("SELECT post_id FROM objects WHERE sha=?1", [digest], |r| {
                    r.get(0)
                })
                .optional()
                .map_err(err)?
                .ok_or_else(|| Error::invalid("浏览排序游标不属于当前索引"))?
        } else {
            None
        };
        let direction = if order.descending() { "DESC" } else { "ASC" };
        let op = if order.descending() { "<" } else { ">" };
        let limit = limit.clamp(1, 129);
        let mut rows = Vec::new();
        for missing in [false, true] {
            if !missing && after.is_some() && post.is_none() {
                continue;
            }
            let remaining = limit - rows.len();
            if remaining == 0 {
                break;
            }
            let condition = if missing {
                if after.is_some() && post.is_none() {
                    format!("post_id IS NULL AND sha{op}?2")
                } else {
                    "post_id IS NULL".into()
                }
            } else if post.is_some() {
                format!("post_id IS NOT NULL AND (post_id,sha){op}(?1,?2)")
            } else {
                "post_id IS NOT NULL".into()
            };
            let mut stmt=self.db.prepare(&format!("SELECT sha,post_id FROM objects WHERE {condition} ORDER BY post_id {direction},sha {direction} LIMIT ?3")).map_err(err)?;
            rows.extend(
                stmt.query_map(params![post, digest, remaining as i64], |r| {
                    Ok((
                        AssetKey {
                            source_id: source_id.into(),
                            asset_id: hex::encode(r.get::<_, Vec<u8>>(0)?),
                        },
                        r.get::<_, Option<i64>>(1)?.map(i128::from),
                    ))
                })
                .map_err(err)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(err)?,
            );
        }
        Ok(rows)
    }
    pub fn post_ids(&self, keys: &[AssetKey]) -> Result<Vec<Option<i64>>> {
        if let BrowseBackend::Online(view) = &self.db {
            return view.post_ids(keys);
        }
        let mut stmt = self
            .db
            .prepare_cached("SELECT post_id FROM objects WHERE sha=?1")
            .map_err(err)?;
        keys.iter()
            .map(|k| {
                let digest = hex::decode(&k.asset_id).map_err(Error::io)?;
                stmt.query_row([digest], |r| r.get(0))
                    .optional()
                    .map(|v| v.flatten())
                    .map_err(err)
            })
            .collect()
    }
    pub fn post_positions(&self, keys: &[AssetKey]) -> Result<Vec<(Option<i128>, i64)>> {
        if let BrowseBackend::Online(view) = &self.db {
            return view.post_positions(keys);
        }
        Ok(self
            .post_ids(keys)?
            .into_iter()
            .map(|id| (id.map(i128::from), 0))
            .collect())
    }
}
impl BrowseIndex {
    pub fn storage(&self) -> Result<(u64, u64)> {
        if !self.root.exists() {
            return Ok((0, 0));
        }
        let mut bytes = 0;
        let mut count = 0;
        for entry in std::fs::read_dir(&self.root).map_err(Error::io)? {
            let entry = entry.map_err(Error::io)?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("sqlite")
                || path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .is_none_or(|s| validate_id(s.strip_suffix(".identity").unwrap_or(s)).is_err())
            {
                continue;
            }
            let meta = entry.metadata().map_err(Error::io)?;
            if meta.is_file() {
                bytes += meta.len();
                count += 1;
            }
        }
        Ok((bytes, count))
    }
    pub fn verify_revision(source: &Source, revision: &str) -> Result<()> {
        if Catalog::open_at(source, Some(revision))?.revision != revision {
            return Err(Error::new("SOURCE_CHANGED", "浏览排序期间来源已更新"));
        }
        Ok(())
    }
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            gates: Mutex::new(HashMap::new()),
        }
    }
    fn path(&self, id: &str) -> Result<PathBuf> {
        validate_id(id)?;
        Ok(self.root.join(format!("{id}.sqlite")))
    }
    fn gate(&self, id: &str) -> Result<Arc<Mutex<()>>> {
        Ok(self
            .gates
            .lock()
            .map_err(|_| Error::new("INTERNAL_ERROR", "浏览索引锁不可用"))?
            .entry(id.into())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone())
    }
    fn open(&self, id: &str) -> Result<Connection> {
        let db =
            Connection::open_with_flags(self.path(id)?, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(err)?;
        db.execute_batch("PRAGMA mmap_size=1073741824;")
            .map_err(err)?;
        Ok(db)
    }
    pub fn is_current(&self, source: &Source) -> Result<bool> {
        if crate::online::available(source) {
            return Ok(true);
        }
        let catalog = Catalog::open(source)?;
        let gate = self.gate(&source.id)?;
        let _guard = match gate.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::WouldBlock) => return Ok(false),
            Err(_) => return Err(Error::new("INTERNAL_ERROR", "浏览索引锁不可用")),
        };
        Ok(self
            .open(&source.id)
            .ok()
            .and_then(|db| stamp(&db).ok())
            .is_some_and(|s| s.generation == catalog.generation && s.sequence == catalog.sequence))
    }
    pub fn anchor(&self, source_id: &str, sequence: u64) -> Result<Option<ChangeAnchor>> {
        let gate = self.gate(source_id)?;
        let _guard = gate
            .lock()
            .map_err(|_| Error::new("INTERNAL_ERROR", "浏览索引锁不可用"))?;
        let db = match self.open(source_id) {
            Ok(db) => db,
            Err(_) => return Ok(None),
        };
        db.query_row(
            "SELECT generation,batch_id FROM anchors WHERE seq=?1",
            [sequence as i64],
            |r| {
                Ok(ChangeAnchor {
                    generation: r.get(0)?,
                    sequence,
                    batch_id: r.get(1)?,
                })
            },
        )
        .optional()
        .map_err(err)
    }
    /// Caller acquires the shared native-query budget before entering this gate.
    pub fn ensure(
        &self,
        source: &Source,
        memory_bytes: u64,
        temporary_root: &Path,
        cancelled: Arc<AtomicBool>,
    ) -> Result<BrowseIndexStamp> {
        let gate = self.gate(&source.id)?;
        let _guard = gate
            .lock()
            .map_err(|_| Error::new("INTERNAL_ERROR", "浏览索引锁不可用"))?;
        let catalog = Catalog::open(source)?;
        std::fs::create_dir_all(&self.root).map_err(Error::io)?;
        let target = self.path(&source.id)?;
        if target.exists()
            && !target
                .canonicalize()
                .map_err(Error::io)?
                .starts_with(self.root.canonicalize().map_err(Error::io)?)
        {
            return Err(Error::invalid("浏览索引路径超出应用缓存目录"));
        }
        let mut existing = if target.is_file() {
            Connection::open(&target).ok()
        } else {
            None
        };
        let previous = existing.as_ref().and_then(|db| stamp(db).ok());
        if let Some(ref s) = previous
            && s.generation == catalog.generation
            && s.sequence == catalog.sequence
        {
            return Ok(s.clone());
        }
        let index_memory = (memory_bytes / 8).clamp(8 << 20, 128 << 20);
        let runtime = Runtime::default()
            .with_query_directory(temporary_root.to_owned())
            .with_query_memory(memory_bytes.saturating_sub(index_memory));
        let native = runtime.open_query(&catalog.analysis_path()?, cancelled.clone())?;
        analysis_sequence(&native, &catalog)?;
        let next_anchor = changes::anchor(&native, &catalog)?;
        let old_anchor = if let (Some(db), Some(s)) = (&existing, &previous) {
            db.query_row(
                "SELECT generation,batch_id FROM anchors WHERE seq=?1",
                [s.sequence as i64],
                |r| {
                    Ok(ChangeAnchor {
                        generation: r.get(0)?,
                        sequence: s.sequence,
                        batch_id: r.get(1)?,
                    })
                },
            )
            .optional()
            .map_err(err)?
        } else {
            None
        };
        let delta = old_anchor
            .as_ref()
            .map(|a| changes::change_sql(&native, &catalog, a))
            .transpose()?
            .flatten();
        let incremental = delta.is_some();
        let temporary = if !incremental {
            Some(
                tempfile::Builder::new()
                    .prefix("index-build-")
                    .suffix(".sqlite")
                    .tempfile_in(&self.root)
                    .map_err(Error::io)?,
            )
        } else {
            None
        };
        let mut db = if let Some(temp) = &temporary {
            Connection::open(temp.path()).map_err(err)?
        } else {
            existing
                .take()
                .ok_or_else(|| Error::new("INTERNAL_ERROR", "增量索引基线丢失"))?
        };
        let expected_count: i64 = if incremental {
            previous.as_ref().map(|s| s.count as i64).unwrap_or(0)
        } else {
            catalog
                .connection()
                .query_row("SELECT count(*) FROM objects", [], |r| r.get(0))
                .map_err(err)?
        };
        let temporary_store = if (expected_count as u64).saturating_mul(96) <= memory_bytes / 2 {
            "MEMORY"
        } else {
            "FILE"
        };
        db.execute_batch(&format!("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA cache_size=-{}; PRAGMA temp_store={temporary_store}; CREATE TABLE IF NOT EXISTS objects(sha BLOB PRIMARY KEY,post_id INTEGER) WITHOUT ROWID; CREATE TABLE IF NOT EXISTS state(key TEXT PRIMARY KEY,value TEXT); CREATE TABLE IF NOT EXISTS anchors(seq INTEGER PRIMARY KEY,generation TEXT,batch_id TEXT);",index_memory/1024)).map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        if !incremental {
            let mut rows = catalog
                .connection()
                .prepare("SELECT sha256 FROM objects ORDER BY sha256")
                .map_err(err)?;
            let mut keys = rows.query([]).map_err(err)?;
            let mut insert = tx
                .prepare("INSERT INTO objects(sha) VALUES (?1)")
                .map_err(err)?;
            while let Some(row) = keys.next().map_err(err)? {
                studio_application::read_cancelled(&cancelled)?;
                let key: String = row.get(0).map_err(err)?;
                insert
                    .execute([hex::decode(key).map_err(Error::io)?])
                    .map_err(err)?;
            }
        }
        let mut refreshed = 0u64;
        let sql = if let Some(delta) = delta {
            native.query(&format!("CREATE TEMP TABLE studio_changed AS {delta}"))?;
            let mut physical = catalog
                .connection()
                .prepare("SELECT 1 FROM objects WHERE sha256=?1")
                .map_err(err)?;
            let mut insert = tx
                .prepare("INSERT OR IGNORE INTO objects(sha) VALUES (?1)")
                .map_err(err)?;
            native.stream_ids(
                "SELECT sha256 FROM studio_changed ORDER BY sha256",
                &mut |rows| {
                    for key in rows {
                        if physical.exists([key]).map_err(err)? {
                            insert
                                .execute([hex::decode(key).map_err(Error::io)?])
                                .map_err(err)?;
                            refreshed += 1;
                        }
                    }
                    Ok(())
                },
            )?;
            "SELECT sha256||':'||COALESCE(CAST(MIN(post_id) AS VARCHAR),'') FROM assets WHERE sha256 IN (SELECT sha256 FROM studio_changed) GROUP BY sha256 ORDER BY sha256".to_string()
        } else {
            "SELECT sha256||':'||COALESCE(CAST(MIN(post_id) AS VARCHAR),'') FROM assets WHERE sha256 IS NOT NULL GROUP BY sha256 ORDER BY sha256".to_string()
        };
        {
            let mut insert = tx
                .prepare("UPDATE objects SET post_id=?2 WHERE sha=?1 AND post_id IS NOT ?2")
                .map_err(err)?;
            native.stream_strings(&sql, 128, &mut |rows| {
                studio_application::read_cancelled(&cancelled)?;
                for text in rows {
                    let (sha, post) = text
                        .split_once(':')
                        .ok_or_else(|| Error::new("SOURCE_FORMAT_ERROR", "浏览索引身份格式无效"))?;
                    let digest = hex::decode(sha).map_err(Error::io)?;
                    if digest.len() != 32 {
                        return Err(Error::new("SOURCE_FORMAT_ERROR", "浏览索引身份长度无效"));
                    }
                    let post = if post.is_empty() {
                        None
                    } else {
                        Some(post.parse::<i64>().map_err(Error::io)?)
                    };
                    insert.execute(params![digest, post]).map_err(err)?;
                }
                Ok(())
            })?;
        }
        // Bulk-build the secondary index once; maintaining it during millions of
        // SHA-ordered updates would turn this reusable index into random writes.
        drop(native);
        tx.execute_batch("CREATE INDEX IF NOT EXISTS objects_post ON objects(post_id,sha)")
            .map_err(err)?;
        if let Some(anchor) = next_anchor {
            tx.execute(
                "INSERT OR REPLACE INTO anchors VALUES (?1,?2,?3)",
                params![anchor.sequence as i64, anchor.generation, anchor.batch_id],
            )
            .map_err(err)?;
        }
        let count: i64 = tx
            .query_row("SELECT count(*) FROM objects", [], |r| r.get(0))
            .map_err(err)?;
        if !incremental {
            refreshed = count as u64;
        }
        for (key, value) in [
            ("format", "1".to_string()),
            ("generation", catalog.generation.clone()),
            ("seq", catalog.sequence.to_string()),
            ("count", count.to_string()),
            ("refreshed", refreshed.to_string()),
            ("incremental", incremental.to_string()),
        ] {
            tx.execute(
                "INSERT OR REPLACE INTO state VALUES (?1,?2)",
                params![key, value],
            )
            .map_err(err)?;
        }
        catalog.verify_unchanged(source)?;
        studio_application::read_cancelled(&cancelled)?;
        tx.commit().map_err(err)?;
        let mut result = stamp(&db)?;
        drop(db);
        drop(existing);
        if let Some(temp) = temporary {
            temp.as_file().sync_all().map_err(Error::io)?;
            temp.persist(&target).map_err(Error::io)?;
        }
        result.bytes = std::fs::metadata(&target).map_err(Error::io)?.len();
        Ok(result)
    }
    pub fn post_ids(&self, source: &Source, keys: &[AssetKey]) -> Result<Vec<Option<i64>>> {
        self.reader(source)?.post_ids(keys)
    }
    pub fn reader(&self, source: &Source) -> Result<BrowseIndexReader> {
        self.reader_at(source, None)
    }
    pub fn reader_at(&self, source: &Source, revision: Option<&str>) -> Result<BrowseIndexReader> {
        if crate::online::available(source) {
            let view = crate::online::Snapshot::open(
                source,
                revision,
                Arc::new(AtomicBool::new(false)),
                None,
            )?;
            let stamp = BrowseIndexStamp {
                generation: view.pointer.generation.clone(),
                sequence: view.sequence,
                count: view.count,
                bytes: 0,
                refreshed_objects: 0,
                incremental: true,
            };
            return Ok(BrowseIndexReader {
                db: BrowseBackend::Online(Box::new(view)),
                stamp,
            });
        }
        let catalog = Catalog::open_at(source, revision)?;
        let gate = self.gate(&source.id)?;
        let _guard = gate
            .lock()
            .map_err(|_| Error::new("INTERNAL_ERROR", "浏览索引锁不可用"))?;
        let db = self.open(&source.id)?;
        db.execute_batch("BEGIN").map_err(err)?;
        let s = stamp(&db)?;
        if s.generation != catalog.generation || s.sequence != catalog.sequence {
            return Err(Error::new("SOURCE_CHANGED", "浏览索引需要刷新"));
        }
        Ok(BrowseIndexReader {
            db: BrowseBackend::Legacy(db),
            stamp: s,
        })
    }
}
fn stamp(db: &Connection) -> Result<BrowseIndexStamp> {
    let value = |key: &str| {
        db.query_row("SELECT value FROM state WHERE key=?1", [key], |r| {
            r.get::<_, String>(0)
        })
        .map_err(err)
    };
    if value("format")? != "1" {
        return Err(Error::new(
            "CACHE_FORMAT_UNSUPPORTED",
            "浏览索引格式需要重建",
        ));
    }
    Ok(BrowseIndexStamp {
        generation: value("generation")?,
        sequence: value("seq")?.parse().map_err(Error::io)?,
        count: value("count")?.parse().map_err(Error::io)?,
        bytes: 0,
        refreshed_objects: value("refreshed")?.parse().map_err(Error::io)?,
        incremental: value("incremental")? == "true",
    })
}
