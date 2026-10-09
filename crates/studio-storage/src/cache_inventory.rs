use crate::*;
use std::collections::HashSet;

pub struct CacheMember {
    pub family_id: String,
    pub result: QueryResult,
    pub cached: bool,
    pub members: u64,
    pub last_used_millis: u64,
    pub bytes: Option<u64>,
    pub in_use: bool,
    pub references: Vec<String>,
    pub reference_count: u64,
}
pub struct CacheInventory {
    pub directory: PathBuf,
    pub members: Vec<CacheMember>,
    pub cleanups: Vec<QueryCleanup>,
}

impl SqliteStore {
    /// Read closed-project metadata without opening its view or changing recency.
    fn cache_read<T>(&self, pid: &str, run: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        match self.handle(pid) {
            Ok(project) => run(&*project.read()?),
            Err(error) if error.code == "PROJECT_CLOSED" => {
                let directory = self.registered_directory(pid)?;
                let manifest: Manifest = serde_json::from_slice(
                    &fs::read(directory.join("project.json")).map_err(Error::io)?,
                )
                .map_err(Error::io)?;
                if manifest.id != pid {
                    return Err(Error::invalid("缓存项目身份不匹配"));
                }
                let db = Connection::open_with_flags(
                    directory.join("project.sqlite"),
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                )
                .map_err(db_error)?;
                db.busy_timeout(std::time::Duration::from_secs(1))
                    .map_err(db_error)?;
                db.execute_batch("PRAGMA query_only=ON; PRAGMA cache_size=-4096; BEGIN;")
                    .map_err(db_error)?;
                run(&db)
            }
            Err(error) => Err(error),
        }
    }
    pub fn cache_project_action<T>(
        &self,
        pid: &str,
        run: impl FnOnce(&SqliteStore) -> Result<T>,
    ) -> Result<T> {
        match self.handle(pid) {
            Ok(_project) => run(self),
            Err(error) if error.code == "PROJECT_CLOSED" => {
                let directory = self.registered_directory(pid)?;
                Self::with_closed_cache(self.root(), pid, &directory, run)?
                    .ok_or_else(|| Error::new("CACHE_BUSY", "项目正在其他会话使用，请稍后重试"))
            }
            Err(error) => Err(error),
        }
    }
    pub fn cache_inventory(
        &self,
        pid: &str,
        after: Option<&str>,
        limit: usize,
        live: &HashSet<String>,
    ) -> Result<CacheInventory> {
        if let Some(id) = after {
            validate_id(id)?;
        }
        let directory = self.registered_directory(pid)?;
        let memory_sizes = self
            .query_sizes
            .lock()
            .map_err(lock_error)?
            .get(pid)
            .copied();
        self.cache_read(pid, |db| {
            // Never walk dbstat from a list request. A pending accounting pass is
            // represented by an unknown size, not a guessed zero or a long scan.
            let revision = query_cache::sizes_revision(db)?;
            let saved: Option<query_cache::QuerySizes> = db.query_row("SELECT value FROM meta WHERE key='query_storage_sizes'", [], |r| r.get::<_,String>(0)).optional().map_err(db_error)?
                .and_then(|raw| serde_json::from_str(&raw).ok())
                .filter(|s: &query_cache::QuerySizes| revision == s.revision);
            let sizes = memory_sizes.filter(|s| s.revision == revision).or(saved);
            let mut stmt = db.prepare("SELECT f.id,coalesce(f.latest_result_id,f.id),f.cached,f.stored_members,f.touched_at FROM query_families f WHERE (stored_members>0 OR (cached=1 AND latest_result_id IS NOT NULL)) AND (?1 IS NULL OR f.id>?1) ORDER BY f.id LIMIT ?2").map_err(db_error)?;
            let families = stmt.query_map(params![after, limit.clamp(1,128) as i64], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,bool>(2)?,unsigned(r,3)?,unsigned(r,4)?))).map_err(db_error)?.collect::<std::result::Result<Vec<_>,_>>().map_err(db_error)?;
            let alive = query_cache::checked_ids(live)?;
            let members = families.into_iter().map(|(family_id, rid, cached, members, last_used_millis)| {
                let result = query::read_result(db, pid, &rid)?;
                let reference_count = db.query_row("SELECT count(*) FROM result_references x JOIN query_results r ON r.id=x.result_id WHERE r.family_id=?1", [&family_id], |r| unsigned(r,0)).map_err(db_error)?;
                let mut refs = db.prepare("SELECT x.owner_kind,x.owner_id,coalesce(m.name,c.name,j.operator,x.owner_id) FROM result_references x JOIN query_results r ON r.id=x.result_id LEFT JOIN object_metadata m ON m.kind=CASE x.owner_kind WHEN 'collection' THEN 'workset' ELSE x.owner_kind END AND m.id=x.owner_id LEFT JOIN collections c ON x.owner_kind='collection' AND c.id=x.owner_id LEFT JOIN jobs j ON x.owner_kind='job' AND j.id=x.owner_id WHERE r.family_id=?1 ORDER BY x.owner_kind,x.owner_id LIMIT 16").map_err(db_error)?;
                let references = refs.query_map([&family_id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(2)?))).map_err(db_error)?.map(|r| r.map(|(kind,name)| format!("{}：{name}", match kind.as_str() { "collection"=>"工作集", "job"=>"任务", "selection"=>"选择", "selection_history"=>"撤销记录", "query_input"=>"查询输入", "query_definition_input"=>"保存的查询", _=>"项目引用" }))).collect::<std::result::Result<Vec<_>,_>>().map_err(db_error)?;
                let in_use = db.query_row(&format!("SELECT EXISTS(SELECT 1 FROM query_results WHERE family_id=?1 AND (status IN ('queued','running') OR id IN ({alive})))"), [&family_id], |r| r.get(0)).map_err(db_error)?;
                Ok(CacheMember { family_id, result, cached, members, last_used_millis, bytes: sizes.map(|s| if s.members==0 {0} else {(u128::from(s.bytes)*u128::from(members)/u128::from(s.members)) as u64}), in_use, references, reference_count })
            }).collect::<Result<Vec<_>>>()?;
            Ok(CacheInventory { directory, members, cleanups: cache_cleanup::list(db)? })
        })
    }
    pub fn cache_scope_label(&self, pid: &str, scope: &ScopeRef) -> Result<String> {
        scope.validate_project(pid)?;
        self.cache_read(pid, |db| match &scope.target {
            ScopeTarget::Workset { collection_id, .. } => db.query_row("SELECT coalesce(m.name,c.name) FROM collections c LEFT JOIN object_metadata m ON m.kind='workset' AND m.id=c.id WHERE c.id=?1", [collection_id], |r|r.get(0)).map_err(db_error),
            ScopeTarget::QueryResult{result_id} => {
                let result = query::read_result(db,pid,result_id)?;
                let ratings = result.spec.conditions.iter().filter(|c| c.field.ends_with(".rating") || c.field=="rating").filter_map(|c| match &c.value { Some(QueryValue::TextList(v))=>Some(v.join("/")), Some(QueryValue::Text(v))=>Some(v.clone()), _=>None }).collect::<Vec<_>>();
                Ok(if ratings.is_empty() {format!("筛选结果 {}", &result_id[..8])} else {format!("分级 {} 的筛选结果",ratings.join(" · ").to_uppercase())})
            }
            _ => Err(Error::invalid("排名缓存范围不受支持")),
        })
    }
}
