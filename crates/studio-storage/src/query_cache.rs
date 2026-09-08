use crate::*;
use sha2::{Digest, Sha256};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryCachePolicy {
    pub quota_bytes: u64,
    pub max_age_seconds: u64,
}
impl Default for QueryCachePolicy {
    fn default() -> Self {
        Self {
            quota_bytes: 4 << 30,
            max_age_seconds: 7 * 24 * 3600,
        }
    }
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct QueryCacheStats {
    pub last_used_millis: u64,
    pub retained_families: u64,
    pub member_versions: u64,
    pub storage_bytes: u64,
    pub database_free_bytes: u64,
    pub protected_results: u64,
    pub reused_results: u64,
    pub incremental_results: u64,
    pub reclaimed_families: u64,
}

/// Durable result metadata stays in the project. This independently bounded,
/// disposable staging database never contains published or user-owned state.
pub struct QueryStage {
    db: Connection,
    file: tempfile::NamedTempFile,
    full_sources: HashSet<String>,
    pub processed: u64,
    pub evaluated: u64,
    pub post_ready: bool,
}
impl QueryStage {
    pub fn new(directory: &Path) -> Result<Self> {
        Self::with_memory(directory, 256 << 20)
    }
    pub fn with_memory(directory: &Path, memory_bytes: u64) -> Result<Self> {
        fs::create_dir_all(directory).map_err(Error::io)?;
        let file = tempfile::Builder::new()
            .prefix("query-stage-")
            .suffix(".sqlite")
            .tempfile_in(directory)
            .map_err(Error::io)?;
        let db = Connection::open(file.path()).map_err(db_error)?;
        db.execute_batch(&format!("PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF; PRAGMA cache_size=-{}; PRAGMA max_page_count={}; CREATE TABLE matches(source_id TEXT,asset_id TEXT,post_id INTEGER,PRIMARY KEY(source_id,asset_id)) WITHOUT ROWID; CREATE TABLE affected(source_id TEXT,asset_id TEXT,PRIMARY KEY(source_id,asset_id)) WITHOUT ROWID; BEGIN;",memory_bytes.clamp(64<<20,2<<30)/1024,QUERY_STAGE_BYTES/4096)).map_err(db_error)?;
        Ok(Self {
            db,
            file,
            full_sources: HashSet::new(),
            processed: 0,
            evaluated: 0,
            post_ready: true,
        })
    }
    pub fn full_source(&mut self, id: &str) {
        self.full_sources.insert(id.into());
    }
    pub fn affected(&mut self, keys: &[AssetKey]) -> Result<()> {
        let mut insert = self
            .db
            .prepare_cached("INSERT OR IGNORE INTO affected VALUES (?1,?2)")
            .map_err(db_error)?;
        for key in keys {
            self.evaluated += insert
                .execute(params![key.source_id, key.asset_id])
                .map_err(db_error)? as u64;
        }
        Ok(())
    }
    pub fn append(
        &mut self,
        keys: &[AssetKey],
        posts: &[Option<i64>],
        processed: u64,
    ) -> Result<()> {
        if keys.len() > 512 || keys.len() != posts.len() {
            return Err(Error::invalid("查询暂存批次无效"));
        }
        let mut insert = self
            .db
            .prepare_cached("INSERT OR IGNORE INTO matches VALUES (?1,?2,?3)")
            .map_err(db_error)?;
        for (key, post) in keys.iter().zip(posts) {
            insert
                .execute(params![key.source_id, key.asset_id, post])
                .map_err(db_error)?;
        }
        self.processed += processed;
        Ok(())
    }
    pub fn seal(&self) -> Result<()> {
        self.db
            .execute_batch("COMMIT; PRAGMA shrink_memory;")
            .map_err(db_error)
    }
}

fn fingerprint(spec: &QuerySpec) -> Result<String> {
    let mut spec = spec.clone().normalize()?;
    spec.version = 2;
    spec.order = QueryOrder::AssetKeyAsc;
    if matches!(
        spec.input_scope.as_ref().map(|s| &s.target),
        Some(ScopeTarget::Source { .. })
    ) {
        spec.input_scope = None;
    }
    Ok(hex::encode(Sha256::digest(
        serde_json::to_vec(&spec).map_err(Error::io)?,
    )))
}
impl SqliteStore {
    /// Only cache housekeeping uses this isolated registry. It does not mark the
    /// user's closed project as opened in the daily application's registry.
    pub fn maintain_closed_cache(
        app_root: &Path,
        pid: &str,
        directory: &Path,
        policy: &QueryCachePolicy,
        force: bool,
    ) -> Result<Option<QueryCacheStats>> {
        validate_id(pid)?;
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(directory.join("project.json")).map_err(Error::io)?)
                .map_err(Error::io)?;
        if manifest["id"].as_str() != Some(pid) {
            return Err(Error::invalid("缓存目录的项目身份不匹配"));
        }
        {
            let db = Connection::open_with_flags(
                directory.join("project.sqlite"),
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .map_err(db_error)?;
            let version: u32 = db
                .query_row("PRAGMA user_version", [], |r| r.get(0))
                .map_err(db_error)?;
            if version != 6 {
                return Ok(None);
            }
            let active:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM query_results WHERE status IN ('queued','running')) OR EXISTS(SELECT 1 FROM jobs WHERE status IN ('queued','preparing','running','waiting_input'))",[],|r|r.get(0)).map_err(db_error)?;
            if active {
                return Ok(None);
            }
        }
        let temp_root = app_root.join("query-maintenance");
        fs::create_dir_all(&temp_root).map_err(Error::io)?;
        let temporary = tempfile::tempdir_in(temp_root).map_err(Error::io)?;
        let maintenance = SqliteStore::new(temporary.path().into())?;
        match maintenance.open(directory.into()) {
            Ok(_) => {}
            Err(e) if e.code == "PROJECT_BUSY" => return Ok(None),
            Err(e) => return Err(e),
        }
        let stats = maintenance.maintain_query_cache(pid, policy, &HashSet::new(), force)?;
        maintenance.close(pid)?;
        Ok(Some(stats))
    }
    pub fn result_page_ordered(
        &self,
        pid: &str,
        rid: &str,
        after: Option<&AssetKey>,
        limit: usize,
        order: QueryOrder,
    ) -> Result<ResultPage> {
        if order.by_post() {
            let p = self.handle(pid)?;
            let db = p.db.lock().map_err(lock_error)?;
            query::ready_result(&db, pid, rid)?;
            return query::post_page(&db, rid, after, limit, order);
        }
        let p = self.handle(pid)?;
        let db = p.db.lock().map_err(lock_error)?;
        query::ready_result(&db, pid, rid)?;
        let direction = if order.descending() { "DESC" } else { "ASC" };
        let op = if order.descending() { "<" } else { ">" };
        let (source, asset) = after
            .map(|k| (k.source_id.as_str(), k.asset_id.as_str()))
            .unwrap_or(("", ""));
        let condition = if after.is_some() {
            format!(" AND (source_id,asset_id){op}(?2,?3)")
        } else {
            String::new()
        };
        let mut stmt=db.prepare(&format!("SELECT source_id,asset_id FROM result_members WHERE result_id=?1{condition} ORDER BY source_id {direction},asset_id {direction} LIMIT ?4")).map_err(db_error)?;
        let limit = limit.clamp(1, 128);
        let mut keys = stmt
            .query_map(params![rid, source, asset, (limit + 1) as i64], |r| {
                Ok(AssetKey {
                    source_id: r.get(0)?,
                    asset_id: r.get(1)?,
                })
            })
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        let more = keys.len() > limit;
        keys.truncate(limit);
        Ok(ResultPage {
            next: if more { keys.last().cloned() } else { None },
            keys,
        })
    }
    pub(crate) fn invalidate_query_sizes(&self, pid: &str) {
        if let Ok(mut sizes) = self.query_sizes.lock() {
            sizes.remove(pid);
        }
    }
    fn query_sizes(&self, pid: &str, db: &Connection) -> Result<(u64, u64)> {
        if let Some(value) = self
            .query_sizes
            .lock()
            .map_err(lock_error)?
            .get(pid)
            .copied()
        {
            return Ok(value);
        }
        let value = (
            member_bytes(db)?,
            db.query_row("SELECT count(*) FROM query_member_data", [], |r| {
                unsigned(r, 0)
            })
            .map_err(db_error)?,
        );
        self.query_sizes
            .lock()
            .map_err(lock_error)?
            .insert(pid.into(), value);
        Ok(value)
    }
    pub fn create_cached_result(
        &self,
        pid: &str,
        definition: Option<(&str, u64)>,
        spec: QuerySpec,
        mut versions: Vec<QuerySourceVersion>,
        enabled: bool,
    ) -> Result<QueryResult> {
        versions.sort_by(|a, b| a.source_id.cmp(&b.source_id));
        self.create_cached_result_kind(pid, definition, spec, versions, enabled, false)
    }
    pub fn browse_result(
        &self,
        pid: &str,
        spec: QuerySpec,
        mut versions: Vec<QuerySourceVersion>,
        enabled: bool,
    ) -> Result<QueryResult> {
        versions.sort_by(|a, b| a.source_id.cmp(&b.source_id));
        let key = fingerprint(&spec)?;
        {
            let p = self.handle(pid)?;
            let db = p.db.lock().map_err(lock_error)?;
            let id:Option<String>=db.query_row("SELECT r.id FROM query_results r JOIN query_families f ON f.id=r.family_id WHERE r.internal=1 AND f.fingerprint=?1 AND r.versions_json=?2 AND (r.status IN ('queued','running','failed','interrupted','cancelled') OR (r.status='ready' AND (?3=0 OR r.post_ready=1))) ORDER BY r.created_at DESC,r.id DESC LIMIT 1",params![key,serde_json::to_string(&versions).map_err(Error::io)?,spec.order.by_post()],|r|r.get(0)).optional().map_err(db_error)?;
            if let Some(id) = id {
                return query::read_result(&db, pid, &id);
            }
        }
        self.create_cached_result_kind(pid, None, spec, versions, enabled, true)
    }
    #[allow(
        clippy::too_many_arguments,
        reason = "Common transaction for user queries and internal sorted scopes"
    )]
    fn create_cached_result_kind(
        &self,
        pid: &str,
        definition: Option<(&str, u64)>,
        spec: QuerySpec,
        mut versions: Vec<QuerySourceVersion>,
        enabled: bool,
        internal: bool,
    ) -> Result<QueryResult> {
        let spec = spec.normalize()?;
        versions.sort_by(|a, b| a.source_id.cmp(&b.source_id));
        if versions.iter().map(|v| &v.source_id).collect::<Vec<_>>()
            != spec.source_ids.iter().collect::<Vec<_>>()
        {
            return Err(Error::invalid("查询来源版本不完整"));
        }
        self.validate_derived(pid, &spec)?;
        self.mark_background(pid)?;
        let p = self.handle(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.transaction().map_err(db_error)?;
        query::validate_sources(&tx, &spec)?;
        query::validate_input(&tx, pid, &spec)?;
        if let Some((id, revision)) = definition {
            let current = query::read_definition(&tx, pid, id)?;
            if current.revision != revision || current.spec.normalize()? != spec {
                return Err(Error::new("REVISION_CONFLICT", "查询定义已变化"));
            }
        }
        let key = fingerprint(&spec)?;
        let previous: Option<(String, String, i64, i64)> = if enabled {
            tx.query_row("SELECT id,latest_result_id,latest_revision,latest_count FROM query_families WHERE fingerprint=?1 AND cached=1 AND latest_result_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM query_results r WHERE r.family_id=query_families.id AND r.status IN ('queued','running')) ORDER BY touched_at DESC LIMIT 1",[&key],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(db_error)?
        } else {
            None
        };
        let result = query::insert_result(&tx, pid, definition, &spec, &versions)?;
        if internal {
            tx.execute(
                "UPDATE query_results SET internal=1 WHERE id=?1",
                [&result.id],
            )
            .map_err(db_error)?;
        }
        if let Some((family, base, revision, count)) = previous {
            let basis = query::read_result(&tx, pid, &base)?;
            let post_ready: bool = tx
                .query_row(
                    "SELECT post_ready FROM query_families WHERE id=?1",
                    [&family],
                    |r| r.get(0),
                )
                .map_err(db_error)?;
            let hit = basis.source_versions == versions && (!spec.order.by_post() || post_ready);
            tx.execute("UPDATE query_results SET family_id=?2,member_revision=?3,cache_base=?4,cache_mode=?5,status=?6,count=?7,post_ready=?8 WHERE id=?1",params![result.id,family,revision+i64::from(!hit),base,if hit{"reused"}else{"refresh"},if hit{"ready"}else{"queued"},if hit{count}else{0},post_ready]).map_err(db_error)?;
            tx.execute("DELETE FROM query_families WHERE id=?1", [&result.id])
                .map_err(db_error)?;
            tx.execute(
                "UPDATE query_families SET touched_at=CAST(?2 AS INTEGER) WHERE id=?1",
                params![family, now()],
            )
            .map_err(db_error)?;
            if hit {
                query::clear_input_references(&tx, &result.id)?;
            }
        } else {
            tx.execute(
                "UPDATE query_families SET fingerprint=?2,cached=?3 WHERE id=?1",
                params![result.id, key, enabled],
            )
            .map_err(db_error)?;
        }
        let result = query::read_result(&tx, pid, &result.id)?;
        tx.commit().map_err(db_error)?;
        Ok(result)
    }
    pub fn cached_basis(&self, pid: &str, rid: &str) -> Result<Option<QueryResult>> {
        let p = self.handle(pid)?;
        let db = p.db.lock().map_err(lock_error)?;
        let id: Option<String> = db
            .query_row(
                "SELECT cache_base FROM query_results WHERE id=?1",
                [rid],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        id.map(|id| query::read_result(&db, pid, &id)).transpose()
    }
    pub fn query_post_order_ready(&self, pid: &str, rid: &str) -> Result<bool> {
        let p = self.handle(pid)?;
        let db = p.db.lock().map_err(lock_error)?;
        db.query_row(
            "SELECT post_ready FROM query_results WHERE id=?1",
            [rid],
            |r| r.get(0),
        )
        .map_err(db_error)
    }
    pub fn query_build_progress(
        &self,
        pid: &str,
        rid: &str,
        processed: u64,
        evaluated: u64,
    ) -> Result<()> {
        let p = self.handle(pid)?;
        let db = p.db.lock().map_err(lock_error)?;
        db.execute("UPDATE query_results SET processed=?2,evaluated_count=?3 WHERE id=?1 AND status='running'",params![rid,processed as i64,evaluated as i64]).map_err(db_error)?;
        Ok(())
    }
    pub fn query_build_phase(&self, pid: &str, rid: &str, phase: &str) -> Result<()> {
        let p = self.handle(pid)?;
        let db = p.db.lock().map_err(lock_error)?;
        db.execute(
            "UPDATE query_results SET cache_mode=?2 WHERE id=?1 AND status='running'",
            params![rid, phase],
        )
        .map_err(db_error)?;
        Ok(())
    }
    pub fn touch_query_cache(&self, pid: &str, rid: &str) -> Result<()> {
        let p = self.handle(pid)?;
        let db = p.db.lock().map_err(lock_error)?;
        let time = now().parse::<i64>().unwrap_or(0);
        db.execute("UPDATE query_families SET touched_at=?2 WHERE id=(SELECT family_id FROM query_results WHERE id=?1) AND touched_at<?2-60000",params![rid,time]).map_err(db_error)?;
        Ok(())
    }
    pub fn publish_stage(
        &self,
        pid: &str,
        rid: &str,
        stage: &QueryStage,
        mode: &str,
        cancelled: &AtomicBool,
    ) -> Result<()> {
        self.publish_stage_with_budget(pid, rid, stage, mode, cancelled, 256 << 20)
    }
    #[allow(
        clippy::too_many_arguments,
        reason = "Publication keeps explicit cancellation and memory limits"
    )]
    pub fn publish_stage_with_budget(
        &self,
        pid: &str,
        rid: &str,
        stage: &QueryStage,
        mode: &str,
        cancelled: &AtomicBool,
        cache_bytes: u64,
    ) -> Result<()> {
        self.invalidate_query_sizes(pid);
        let p = self.handle(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let result = query::read_result(&db, pid, rid)?;
        if result.state != ResultState::Running {
            return Err(Error::new("CANCELLED", "结果构建已停止"));
        }
        let (family, revision): (String, i64) = db
            .query_row(
                "SELECT family_id,member_revision FROM query_results WHERE id=?1",
                [rid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(db_error)?;
        // Only the project is written in these transactions; staging is sealed.
        db.execute(
            "ATTACH DATABASE ?1 AS query_stage",
            [stage.file.path().to_string_lossy().as_ref()],
        )
        .map_err(db_error)?;
        let old_cache: i64 = db
            .query_row("PRAGMA cache_size", [], |r| r.get(0))
            .map_err(db_error)?;
        db.execute_batch(&format!(
            "PRAGMA cache_size=-{}",
            cache_bytes.clamp(64 << 20, 4 << 30) / 1024
        ))
        .map_err(db_error)?;
        let outcome = (|| {
            let mut changed = 0u64;
            let tx = db.transaction().map_err(db_error)?;
            for source in &result.spec.source_ids {
                studio_application::read_cancelled(cancelled)?;
                let affected = if stage.full_sources.contains(source) {
                    "1=1"
                } else {
                    "EXISTS(SELECT 1 FROM query_stage.affected a WHERE a.source_id=m.source_id AND a.asset_id=m.asset_id)"
                };
                changed+=tx.execute(&format!("UPDATE query_member_data AS m SET valid_until=?3 WHERE family_id=?1 AND source_id=?2 AND valid_until IS NULL AND ({affected}) AND NOT EXISTS(SELECT 1 FROM query_stage.matches s WHERE s.source_id=m.source_id AND s.asset_id=m.asset_id AND s.post_id IS m.post_id)"),params![family,source,revision]).map_err(db_error)? as u64;
            }
            let mut after = (String::new(), String::new());
            loop {
                studio_application::read_cancelled(cancelled)?;
                let last = {
                    let mut stmt=tx.prepare("SELECT source_id,asset_id FROM query_stage.matches WHERE (source_id,asset_id)>(?1,?2) ORDER BY source_id,asset_id LIMIT 32768").map_err(db_error)?;
                    stmt.query_map(params![after.0, after.1], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                    })
                    .map_err(db_error)?
                    .last()
                    .transpose()
                    .map_err(db_error)?
                };
                let Some(last) = last else { break };
                changed+=tx.execute("INSERT INTO query_member_data(family_id,source_id,asset_id,valid_from,post_id) SELECT ?1,s.source_id,s.asset_id,?2,s.post_id FROM query_stage.matches s WHERE (s.source_id,s.asset_id)>(?3,?4) AND (s.source_id,s.asset_id)<=(?5,?6) AND NOT EXISTS(SELECT 1 FROM query_member_data m WHERE m.family_id=?1 AND m.source_id=s.source_id AND m.asset_id=s.asset_id AND m.valid_until IS NULL) ORDER BY s.source_id,s.asset_id",params![family,revision,after.0,after.1,last.0,last.1]).map_err(db_error)? as u64;
                after = last;
            }
            studio_application::read_cancelled(cancelled)?;
            let count:i64=tx.query_row("SELECT count(*) FROM query_member_data WHERE family_id=?1 AND valid_from<=?2 AND (valid_until IS NULL OR valid_until>?2)",params![family,revision],|r|r.get(0)).map_err(db_error)?;
            tx.execute("UPDATE query_results SET count=?2,processed=?3,cache_mode=?4,evaluated_count=?5,changed_members=?6,post_ready=?7 WHERE id=?1",params![rid,count,stage.processed as i64,mode,stage.evaluated as i64,changed as i64,stage.post_ready]).map_err(db_error)?;
            tx.commit().map_err(db_error)?;
            Ok(())
        })();
        let detached = db
            .execute_batch("DETACH DATABASE query_stage;")
            .map_err(db_error);
        let restored = db
            .execute_batch(&format!(
                "PRAGMA cache_size={old_cache}; PRAGMA shrink_memory;"
            ))
            .map_err(db_error);
        outcome.and(detached).and(restored)
    }
    pub fn query_cache_stats(&self, pid: &str) -> Result<QueryCacheStats> {
        let p = self.handle(pid)?;
        let db = p.db.lock().map_err(lock_error)?;
        stats(&db, self.query_sizes(pid, &db)?)
    }
    pub fn maintain_query_cache(
        &self,
        pid: &str,
        policy: &QueryCachePolicy,
        live: &HashSet<String>,
        force: bool,
    ) -> Result<QueryCacheStats> {
        let p = self.handle(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let cutoff = now()
            .parse::<u64>()
            .unwrap_or(0)
            .saturating_sub(policy.max_age_seconds.saturating_mul(1000))
            as i64;
        let mut reclaimed = 0;
        let candidates = {
            let mut stmt=db.prepare("SELECT f.id,f.touched_at,f.cached FROM query_families f WHERE (cached=1 OR latest_count>0) AND NOT EXISTS(SELECT 1 FROM query_results r WHERE r.family_id=f.id AND r.status IN ('queued','running')) ORDER BY touched_at,id").map_err(db_error)?;
            stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, bool>(2)?,
                ))
            })
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?
        };
        let mut bytes = self.query_sizes(pid, &db)?.0;
        let mut retained: u64 = db
            .query_row(
                "SELECT count(*) FROM query_families WHERE cached=1",
                [],
                |r| unsigned(r, 0),
            )
            .map_err(db_error)?;
        for (family, touched, cached) in candidates {
            let ids = {
                let mut stmt = db
                    .prepare("SELECT id FROM query_results WHERE family_id=?1")
                    .map_err(db_error)?;
                stmt.query_map([&family], |r| r.get::<_, String>(0))
                    .map_err(db_error)?
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(db_error)?
            };
            if ids.iter().any(|id| live.contains(id)) {
                continue;
            }
            if cached
                && !force
                && bytes <= policy.quota_bytes
                && retained <= 256
                && touched >= cutoff
            {
                continue;
            }
            let tx = db.transaction().map_err(db_error)?;
            tx.execute(
                "UPDATE query_families SET cached=0,latest_count=0 WHERE id=?1",
                [&family],
            )
            .map_err(db_error)?;
            tx.execute("UPDATE query_results SET status='released',count=NULL,error='查询缓存已回收，可重新计算' WHERE family_id=?1 AND status='ready' AND NOT EXISTS(SELECT 1 FROM result_references x WHERE x.result_id=query_results.id)",[&family]).map_err(db_error)?;
            prune(&tx, &family, live)?;
            event(&tx, "result.changed", &family)?;
            tx.commit().map_err(db_error)?;
            reclaimed += 1;
            retained = retained.saturating_sub(u64::from(cached));
            self.invalidate_query_sizes(pid);
            bytes = self.query_sizes(pid, &db)?.0;
            if reclaimed >= 2 {
                break;
            }
        }
        // Expired intermediate versions are removed while referenced snapshots remain.
        let families = {
            let mut stmt=db.prepare("SELECT id FROM query_families WHERE latest_revision>1 AND prune_pending=1 LIMIT 2").map_err(db_error)?;
            stmt.query_map([], |r| r.get::<_, String>(0))
                .map_err(db_error)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(db_error)?
        };
        for family in families {
            let tx = db.transaction().map_err(db_error)?;
            let removed = prune(&tx, &family, live)?;
            let old_live=live.iter().any(|id|tx.query_row("SELECT EXISTS(SELECT 1 FROM query_results r JOIN query_families f ON f.id=r.family_id WHERE r.id=?1 AND r.family_id=?2 AND r.member_revision<f.latest_revision)",params![id,family],|r|r.get::<_,bool>(0)).unwrap_or(true));
            if !old_live {
                tx.execute(
                    "UPDATE query_families SET prune_pending=0 WHERE id=?1",
                    [&family],
                )
                .map_err(db_error)?;
            }
            if removed > 0 {
                event(&tx, "result.changed", &family)?;
            }
            tx.commit().map_err(db_error)?;
            if removed > 0 {
                self.invalidate_query_sizes(pid);
            }
        }
        db.execute_batch("PRAGMA incremental_vacuum(8192);")
            .map_err(db_error)?;
        let mut value = stats(&db, self.query_sizes(pid, &db)?)?;
        value.reclaimed_families = reclaimed;
        Ok(value)
    }
}
fn member_bytes(db: &Connection) -> Result<u64> {
    db.query_row("SELECT COALESCE(SUM(pgsize),0) FROM dbstat WHERE name IN ('query_member_data','query_members_post','query_members_expired')",[],|r|unsigned(r,0)).map_err(db_error)
}
fn stats(db: &Connection, sizes: (u64, u64)) -> Result<QueryCacheStats> {
    let scalar = |sql: &str| db.query_row(sql, [], |r| unsigned(r, 0)).map_err(db_error);
    Ok(QueryCacheStats {
        last_used_millis: scalar(
            "SELECT COALESCE(MAX(touched_at),0) FROM query_families WHERE cached=1",
        )?,
        retained_families: scalar("SELECT count(*) FROM query_families WHERE cached=1")?,
        member_versions: sizes.1,
        storage_bytes: sizes.0,
        database_free_bytes: scalar("PRAGMA freelist_count")? * scalar("PRAGMA page_size")?,
        protected_results: scalar("SELECT count(DISTINCT result_id) FROM result_references")?,
        reused_results: scalar("SELECT count(*) FROM query_results WHERE cache_mode='reused'")?,
        incremental_results: scalar(
            "SELECT count(*) FROM query_results WHERE cache_mode='incremental'",
        )?,
        reclaimed_families: 0,
    })
}
fn prune(db: &Connection, family: &str, live: &HashSet<String>) -> Result<usize> {
    // Active views get ordinary in-memory references; project references are durable.
    let mut ids = live
        .iter()
        .filter(|id| validate_id(id).is_ok())
        .map(|id| format!("'{id}'"))
        .collect::<Vec<_>>();
    if ids.is_empty() {
        ids.push("''".into());
    }
    let alive = ids.join(",");
    db.execute(&format!("UPDATE query_results SET status='released',count=NULL,error='旧查询缓存已合并，可重新计算' WHERE family_id=?1 AND status='ready' AND member_revision<(SELECT latest_revision FROM query_families WHERE id=?1) AND id NOT IN ({alive}) AND NOT EXISTS(SELECT 1 FROM result_references x WHERE x.result_id=query_results.id)"),[family]).map_err(db_error)?;
    db.execute("DELETE FROM artifact_references WHERE owner_kind='query_result' AND owner_id IN (SELECT id FROM query_results WHERE family_id=?1 AND status='released')",[family]).map_err(db_error)?;
    let cached: bool = db
        .query_row(
            "SELECT cached FROM query_families WHERE id=?1",
            [family],
            |r| r.get(0),
        )
        .map_err(db_error)?;
    let candidate = if cached {
        "AND m.valid_until IS NOT NULL"
    } else {
        ""
    };
    let index = if cached {
        "INDEXED BY query_members_expired"
    } else {
        ""
    };
    db.execute(&format!("WITH kept(revision) AS MATERIALIZED (SELECT DISTINCT r.member_revision FROM query_results r WHERE r.family_id=?1 AND (r.status IN ('queued','running') OR r.id IN ({alive}) OR EXISTS(SELECT 1 FROM result_references x WHERE x.result_id=r.id)) UNION SELECT latest_revision FROM query_families WHERE id=?1 AND cached=1) DELETE FROM query_member_data AS m {index} WHERE family_id=?1 {candidate} AND NOT EXISTS(SELECT 1 FROM kept WHERE revision>=m.valid_from AND (m.valid_until IS NULL OR revision<m.valid_until))"),[family]).map_err(db_error)
}
pub(super) fn collect_family(db: &Connection, rid: &str) -> Result<()> {
    let family: String = db
        .query_row(
            "SELECT family_id FROM query_results WHERE id=?1",
            [rid],
            |r| r.get(0),
        )
        .map_err(db_error)?;
    // Explicit release only discards an uncached, unreferenced family.
    let cached: bool = db
        .query_row(
            "SELECT cached FROM query_families WHERE id=?1",
            [&family],
            |r| r.get(0),
        )
        .map_err(db_error)?;
    if !cached {
        prune(db, &family, &HashSet::new())?;
    }
    Ok(())
}
pub(super) fn rollback_revision(db: &Connection, rid: &str) -> Result<()> {
    let (family, revision): (String, i64) = db
        .query_row(
            "SELECT family_id,member_revision FROM query_results WHERE id=?1",
            [rid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(db_error)?;
    let latest: i64 = db
        .query_row(
            "SELECT latest_revision FROM query_families WHERE id=?1",
            [&family],
            |r| r.get(0),
        )
        .map_err(db_error)?;
    if revision > latest {
        db.execute(
            "DELETE FROM query_member_data WHERE family_id=?1 AND valid_from=?2",
            params![family, revision],
        )
        .map_err(db_error)?;
        db.execute(
            "UPDATE query_member_data SET valid_until=NULL WHERE family_id=?1 AND valid_until=?2",
            params![family, revision],
        )
        .map_err(db_error)?;
    }
    Ok(())
}
pub(super) fn recover(db: &Connection) -> Result<()> {
    let ids = {
        let mut stmt=db.prepare("SELECT r.id FROM query_results r JOIN query_families f ON f.id=r.family_id WHERE r.status IN ('cancelled','failed','interrupted') AND r.member_revision>f.latest_revision").map_err(db_error)?;
        stmt.query_map([], |r| r.get::<_, String>(0))
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?
    };
    for id in ids {
        rollback_revision(db, &id)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pruning_seeks_only_expired_members_after_many_reused_results() {
        let mut db = Connection::open_in_memory().unwrap();
        migrations::initialize(&mut db).unwrap();
        db.execute_batch("INSERT INTO sources VALUES('source','{}'); INSERT INTO query_results(id,spec_json,versions_json,status,count,created_at) VALUES('base','{}','[]','ready',100000,'1'); UPDATE query_families SET cached=1,latest_revision=2,latest_result_id='base' WHERE id='base'; WITH RECURSIVE n(x) AS(VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<100000) INSERT INTO query_member_data(family_id,source_id,asset_id,valid_from,post_id) SELECT 'base','source',printf('%08d',x),1,x FROM n; INSERT INTO query_member_data VALUES ('base','source','old-a',1,2,1),('base','source','old-b',1,2,2); WITH RECURSIVE n(x) AS(VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<200) INSERT INTO query_results(id,spec_json,versions_json,status,count,created_at,family_id,member_revision) SELECT printf('alias-%04d',x),'{}','[]','ready',100000,'1','base',1 FROM n;").unwrap();
        let ticks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let measured = ticks.clone();
        db.progress_handler(
            100,
            Some(move || measured.fetch_add(1, Ordering::Relaxed) > 1000),
        )
        .unwrap();
        assert_eq!(prune(&db, "base", &HashSet::new()).unwrap(), 2);
        db.progress_handler(0, None::<fn() -> bool>).unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM query_member_data", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            100000
        );
        assert!(ticks.load(Ordering::Relaxed) <= 1000);
    }
    #[test]
    fn post_id_pages_seek_to_the_cursor_without_sorting_the_remaining_result() {
        let mut db = Connection::open_in_memory().unwrap();
        migrations::initialize(&mut db).unwrap();
        db.execute_batch("INSERT INTO sources VALUES('source','{}'); INSERT INTO query_results(id,spec_json,versions_json,status,count,created_at,post_ready) VALUES('result','{}','[]','ready',100000,'1',1); WITH RECURSIVE n(x) AS(VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<100000) INSERT INTO query_member_data(family_id,source_id,asset_id,valid_from,post_id) SELECT 'result','source',printf('%08d',x),1,x FROM n;").unwrap();
        let ticks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let measured = ticks.clone();
        db.progress_handler(
            100,
            Some(move || measured.fetch_add(1, Ordering::Relaxed) > 100),
        )
        .unwrap();
        let page = query::post_page(
            &db,
            "result",
            Some(&AssetKey {
                source_id: "source".into(),
                asset_id: "00050000".into(),
            }),
            96,
            QueryOrder::PostIdDesc,
        )
        .unwrap();
        db.progress_handler(0, None::<fn() -> bool>).unwrap();
        assert_eq!(page.keys.len(), 96);
        assert_eq!(page.keys[0].asset_id, "00049999");
        assert_eq!(page.keys.last().unwrap().asset_id, "00049904");
        assert!(ticks.load(Ordering::Relaxed) <= 100);
    }
}
