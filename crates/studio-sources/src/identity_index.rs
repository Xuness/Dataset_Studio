use crate::{
    canonical::{Catalog, err},
    duckdb::Runtime,
    query::{ChangeAnchor, analysis_sequence, changes},
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use studio_application::read_cancelled;
use studio_domain::*;

#[derive(Clone, Serialize, Deserialize)]
struct Stamp {
    version: u32,
    generation: String,
    sequence: u64,
    batch_id: Option<String>,
}
pub struct IdentityIndex {
    root: PathBuf,
    gates: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}
pub struct IdentityReader {
    db: Connection,
    pub generation: String,
    pub sequence: u64,
}
fn preparing() -> Error {
    Error::new(
        "SOURCE_INDEX_PREPARING",
        "正在准备图片身份索引，完成后会自动显示帖子信息",
    )
}
fn digest(value: &str) -> Result<Vec<u8>> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(Error::invalid("无效的图片或记录身份"));
    }
    hex::decode(value).map_err(Error::io)
}
fn stamp(db: &Connection) -> Result<Stamp> {
    let raw: String = db
        .query_row("SELECT value FROM meta WHERE key='stamp'", [], |r| r.get(0))
        .map_err(err)?;
    let value: Stamp = serde_json::from_str(&raw).map_err(Error::io)?;
    if value.version != 1 {
        return Err(preparing());
    }
    Ok(value)
}
impl IdentityReader {
    pub fn record_ids(
        &self,
        asset: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>> {
        let sha = digest(asset)?;
        let after = after.map(digest).transpose()?.unwrap_or_default();
        self.db.prepare("SELECT record_id FROM identities WHERE sha=?1 AND record_id>?2 ORDER BY record_id LIMIT ?3").map_err(err)?
            .query_map(params![sha,after,limit.clamp(1,51) as u32], |r| r.get::<_,Vec<u8>>(0)).map_err(err)?
            .map(|r| r.map(hex::encode).map_err(err)).collect()
    }
    pub fn summaries(&self, source: &str, assets: &[String]) -> Result<Vec<AssetSummary>> {
        if assets.len() > 128 {
            return Err(Error::invalid("身份摘要最多 128 项"));
        }
        let mut stmt = self
            .db
            .prepare_cached("SELECT post_count,posts_json FROM summaries WHERE sha=?1")
            .map_err(err)?;
        assets
            .iter()
            .map(|asset| {
                let value: Option<(u64, String)> = stmt
                    .query_row([digest(asset)?], |r| {
                        let count: i64 = r.get(0)?;
                        Ok((
                            u64::try_from(count)
                                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, count))?,
                            r.get(1)?,
                        ))
                    })
                    .optional()
                    .map_err(err)?;
                let (post_count, post_ids) = match value {
                    Some((count, raw)) => (count, serde_json::from_str(&raw).map_err(Error::io)?),
                    None => (0, Vec::new()),
                };
                Ok(AssetSummary {
                    asset_id: asset.clone(),
                    post_ids,
                    post_count,
                    version: format!("metadata-v1:{source}:{}:{}", self.generation, self.sequence),
                })
            })
            .collect()
    }
}
impl IdentityIndex {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            gates: Mutex::new(HashMap::new()),
        }
    }
    fn path(&self, id: &str) -> Result<PathBuf> {
        validate_id(id)?;
        Ok(self.root.join(format!("{id}.identity.sqlite")))
    }
    fn gate(&self, id: &str) -> Result<Arc<Mutex<()>>> {
        validate_id(id)?;
        Ok(self
            .gates
            .lock()
            .map_err(|_| Error::new("INTERNAL_ERROR", "身份索引锁不可用"))?
            .entry(id.into())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone())
    }
    fn open(&self, id: &str) -> Result<Connection> {
        let db = Connection::open_with_flags(self.path(id)?, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(err)?;
        db.busy_timeout(std::time::Duration::from_millis(100))
            .map_err(err)?;
        db.execute_batch("PRAGMA query_only=ON; PRAGMA cache_size=-8192;")
            .map_err(err)?;
        Ok(db)
    }
    pub fn is_current(&self, source: &Source) -> Result<bool> {
        if crate::online::available(source) {
            return Ok(true);
        }
        let catalog = Catalog::open(source)?;
        let gate = self.gate(&source.id)?;
        let Ok(_guard) = gate.try_lock() else {
            return Ok(false);
        };
        Ok(self
            .open(&source.id)
            .ok()
            .and_then(|db| stamp(&db).ok())
            .is_some_and(|s| s.generation == catalog.generation && s.sequence == catalog.sequence))
    }
    pub fn reader(&self, source: &Source) -> Result<IdentityReader> {
        let catalog = Catalog::open(source)?;
        let gate = self.gate(&source.id)?;
        let _guard = gate.try_lock().map_err(|_| preparing())?;
        let db = self.open(&source.id).map_err(|_| preparing())?;
        db.execute_batch("BEGIN").map_err(err)?;
        let stamp = stamp(&db)?;
        if stamp.generation != catalog.generation || stamp.sequence != catalog.sequence {
            return Err(preparing());
        }
        Ok(IdentityReader {
            db,
            generation: stamp.generation,
            sequence: stamp.sequence,
        })
    }
    pub fn ensure(
        &self,
        source: &Source,
        memory: u64,
        temporary_root: &Path,
        cancelled: Arc<AtomicBool>,
    ) -> Result<()> {
        let gate = self.gate(&source.id)?;
        let _guard = gate.lock().map_err(|_| preparing())?;
        read_cancelled(&cancelled)?;
        let catalog = Catalog::open(source)?;
        std::fs::create_dir_all(&self.root).map_err(Error::io)?;
        let target = self.path(&source.id)?;
        if target.exists()
            && !target
                .canonicalize()
                .map_err(Error::io)?
                .starts_with(self.root.canonicalize().map_err(Error::io)?)
        {
            return Err(Error::invalid("身份索引超出缓存目录"));
        }
        let mut existing = if target.is_file() {
            Connection::open(&target).ok()
        } else {
            None
        };
        let previous = existing.as_ref().and_then(|db| stamp(db).ok());
        if previous
            .as_ref()
            .is_some_and(|s| s.generation == catalog.generation && s.sequence == catalog.sequence)
        {
            return Ok(());
        }
        let sqlite_memory = (memory / 8).clamp(8 << 20, 128 << 20);
        let runtime = Runtime::default()
            .with_query_directory(temporary_root.into())
            .with_query_memory(memory.saturating_sub(sqlite_memory));
        let native = runtime.open_query(&catalog.analysis_path()?, cancelled.clone())?;
        analysis_sequence(&native, &catalog)?;
        let next_anchor = changes::anchor(&native, &catalog)?;
        let delta = previous
            .as_ref()
            .and_then(|s| {
                s.batch_id.as_ref().map(|batch_id| ChangeAnchor {
                    generation: s.generation.clone(),
                    sequence: s.sequence,
                    batch_id: batch_id.clone(),
                })
            })
            .map(|anchor| changes::change_sql(&native, &catalog, &anchor))
            .transpose()?
            .flatten();
        let temporary = if delta.is_none() {
            Some(
                tempfile::Builder::new()
                    .prefix("identity-build-")
                    .suffix(".sqlite")
                    .tempfile_in(&self.root)
                    .map_err(Error::io)?,
            )
        } else {
            None
        };
        let mut db = if let Some(file) = &temporary {
            Connection::open(file.path()).map_err(err)?
        } else {
            existing.take().ok_or_else(preparing)?
        };
        db.busy_timeout(std::time::Duration::from_secs(3))
            .map_err(err)?;
        // Each rebuild stays bounded even when a source reports malformed or
        // unexpectedly large input. This application-owned index is accounted
        // as source-index storage; the source database remains read-only.
        let page_size: i64 = db
            .pragma_query_value(None, "page_size", |r| r.get(0))
            .map_err(err)?;
        db.pragma_update(None, "max_page_count", (8_i64 << 30) / page_size)
            .map_err(err)?;
        db.execute_batch(&format!("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA cache_size=-{}; PRAGMA temp_store=FILE; CREATE TABLE IF NOT EXISTS identities(sha BLOB NOT NULL,record_id BLOB NOT NULL,post_id INTEGER,PRIMARY KEY(sha,record_id)) WITHOUT ROWID; CREATE TABLE IF NOT EXISTS summaries(sha BLOB PRIMARY KEY,post_count INTEGER NOT NULL,posts_json TEXT NOT NULL) WITHOUT ROWID; CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY,value TEXT); CREATE TEMP TABLE changed(sha BLOB PRIMARY KEY) WITHOUT ROWID;",sqlite_memory/1024)).map_err(err)?;
        let flag = cancelled.clone();
        db.progress_handler(1000, Some(move || flag.load(Ordering::Acquire)))
            .map_err(err)?;
        let outcome = (|| {
            let tx = db.transaction().map_err(err)?;
            let incremental = delta.is_some();
            if let Some(sql) = &delta {
                native.query(&format!("CREATE TEMP TABLE studio_changed AS {sql}"))?;
                let mut mark = tx
                    .prepare("INSERT OR IGNORE INTO changed VALUES (?1)")
                    .map_err(err)?;
                native.stream_ids(
                    "SELECT sha256 FROM studio_changed ORDER BY sha256",
                    &mut |rows| {
                        for sha in rows {
                            mark.execute([digest(sha)?]).map_err(err)?;
                        }
                        Ok(())
                    },
                )?;
                tx.execute_batch("DELETE FROM identities WHERE sha IN (SELECT sha FROM changed); DELETE FROM summaries WHERE sha IN (SELECT sha FROM changed);").map_err(err)?;
            }
            let predicate = if incremental {
                " AND sha256 IN (SELECT sha256 FROM studio_changed)"
            } else {
                ""
            };
            let sql = format!(
                "SELECT sha256||':'||asset_id||':'||coalesce(CAST(post_id AS VARCHAR),'') FROM assets WHERE sha256 IS NOT NULL{predicate} ORDER BY sha256,asset_id"
            );
            {
                let mut insert = tx
                    .prepare("INSERT INTO identities VALUES (?1,?2,?3)")
                    .map_err(err)?;
                native.stream_strings(&sql, 256, &mut |rows| {
                    read_cancelled(&cancelled)?;
                    for row in rows {
                        let parts = row.split(':').collect::<Vec<_>>();
                        if parts.len() != 3 {
                            return Err(Error::new("SOURCE_FORMAT_ERROR", "身份索引输入无效"));
                        }
                        let post = if parts[2].is_empty() {
                            None
                        } else {
                            Some(parts[2].parse::<i64>().map_err(Error::io)?)
                        };
                        insert
                            .execute(params![digest(parts[0])?, digest(parts[1])?, post])
                            .map_err(err)?;
                    }
                    Ok(())
                })?;
            }
            tx.execute_batch(
                "CREATE INDEX IF NOT EXISTS identities_post ON identities(sha,post_id)",
            )
            .map_err(err)?;
            let predicate = if incremental {
                " AND sha IN (SELECT sha FROM changed)"
            } else {
                ""
            };
            tx.execute_batch(&format!("WITH posts AS (SELECT DISTINCT sha,post_id FROM identities WHERE post_id IS NOT NULL{predicate}), ordered AS (SELECT sha,post_id,row_number() OVER (PARTITION BY sha ORDER BY post_id) AS n FROM posts) INSERT INTO summaries SELECT sha,count(*),json_group_array(CAST(post_id AS TEXT)) FILTER (WHERE n<=8) FROM ordered GROUP BY sha;")).map_err(err)?;
            let next = Stamp {
                version: 1,
                generation: catalog.generation.clone(),
                sequence: catalog.sequence,
                batch_id: next_anchor.map(|a| a.batch_id),
            };
            tx.execute(
                "INSERT OR REPLACE INTO meta VALUES ('stamp',?1)",
                [serde_json::to_string(&next).map_err(Error::io)?],
            )
            .map_err(err)?;
            catalog.verify_unchanged(source)?;
            read_cancelled(&cancelled)?;
            tx.commit().map_err(err)
        })();
        read_cancelled(&cancelled)?;
        outcome?;
        drop(native);
        drop(db);
        drop(existing);
        if let Some(file) = temporary {
            file.as_file().sync_all().map_err(Error::io)?;
            file.persist(&target).map_err(Error::io)?;
        }
        Ok(())
    }
}
