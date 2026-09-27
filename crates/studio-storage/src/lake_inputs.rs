//! Global fixed-input preparation records; project results retain their own members.
use crate::*;
use studio_domain::lake_updates::LakeInputPreparation;

impl SqliteStore {
    /// Freeze mutable members through a read snapshot, writing bulk keys only to
    /// the separate member store. The control writer publishes a small header.
    pub fn capture_lake_members(
        &self,
        pid: &str,
        scope: &ScopeRef,
        spec: QuerySpec,
        versions: Vec<QuerySourceVersion>,
        preparation: &str,
        cancelled: &AtomicBool,
    ) -> Result<QueryResult> {
        let spec = spec.normalize()?;
        let p = self.handle(pid)?;
        let snapshot = p.read()?;
        let resolved = scopes::resolve(&snapshot, pid, scope)?;
        let staging = p.project.directory.join(".staging");
        fs::create_dir_all(&staging).map_err(Error::io)?;
        let staging = staging.canonicalize().map_err(Error::io)?;
        if !staging.starts_with(&p.project.directory) {
            return Err(Error::new(
                "WORKER_PATH_INVALID",
                "范围准备暂存超出项目目录",
            ));
        }
        let mut stage = QueryStage::with_memory(&staging, 64 << 20)?;
        stage.post_ready = false;
        let mut stmt = snapshot
            .prepare(&format!(
                "SELECT source_id,asset_id FROM ({}) ORDER BY source_id,asset_id",
                resolved.sql
            ))
            .map_err(db_error)?;
        let mut rows = stmt.query([]).map_err(db_error)?;
        let mut chunk = Vec::with_capacity(512);
        while let Some(row) = rows.next().map_err(db_error)? {
            chunk.push(AssetKey {
                source_id: row.get(0).map_err(db_error)?,
                asset_id: row.get(1).map_err(db_error)?,
            });
            if chunk.len() == 512 {
                studio_application::read_cancelled(cancelled)?;
                if self.lake_input_preparation(preparation)?.state == "cancelling" {
                    return Err(Error::new("CANCELLED", "范围准备已取消"));
                }
                stage.append(&chunk, &vec![None; chunk.len()], chunk.len() as u64)?;
                chunk.clear();
            }
        }
        if !chunk.is_empty() {
            stage.append(&chunk, &vec![None; chunk.len()], chunk.len() as u64)?;
        }
        stage.seal()?;
        let id = new_id();
        {
            let mut members = p.result_writer.lock().map_err(lock_error)?;
            stage.copy_to_members(&mut members, &id, cancelled)?;
        }
        if self.lake_input_preparation(preparation)?.state == "cancelling" {
            return Err(Error::new("CANCELLED", "范围准备已取消"));
        }
        studio_application::read_cancelled(cancelled)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.project_transaction().map_err(db_error)?;
        // The scope records provenance; these memberships no longer depend on it.
        tx.execute("INSERT INTO query_results(id,spec_json,versions_json,status,count,created_at,storage_kind,cache_mode,processed,post_ready) VALUES(?1,?2,?3,'ready',?4,?5,'sealed','full',?4,0)",params![id,serde_json::to_string(&spec).map_err(Error::io)?,serde_json::to_string(&versions).map_err(Error::io)?,stage.processed as i64,now()]).map_err(db_error)?;
        tx.execute(
            "UPDATE query_families SET fixed=1,cached=0 WHERE id=?1",
            [&id],
        )
        .map_err(db_error)?;
        tx.execute(
            "INSERT INTO result_references VALUES('lake_input',?1,?2)",
            params![preparation, id],
        )
        .map_err(db_error)?;
        event(&tx, "result.created", &id)?;
        let result = query::read_result(&tx, pid, &id)?;
        tx.commit().map_err(db_error)?;
        Ok(result)
    }
    pub fn lake_input_activity(&self) -> Result<(u64, u64, Vec<LakeInputPreparation>)> {
        let db = self.registry.lock().map_err(lock_error)?;
        let pending=db.query_row("SELECT count(*) FROM lake_input_preparations WHERE state IN ('preparing','cancelling')",[],|r|unsigned(r,0)).map_err(db_error)?;
        let attention = db
            .query_row(
                "SELECT count(*) FROM lake_input_preparations WHERE state='needs_review'",
                [],
                |r| unsigned(r, 0),
            )
            .map_err(db_error)?;
        let mut stmt=db.prepare("SELECT json FROM lake_input_preparations WHERE state IN ('preparing','cancelling','needs_review') ORDER BY rowid DESC LIMIT 20").map_err(db_error)?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(db_error)?
            .map(|v| serde_json::from_str(&v.map_err(db_error)?).map_err(Error::io))
            .collect::<Result<Vec<_>>>()?;
        Ok((pending, attention, rows))
    }
    pub fn lake_input_lease(&self, pid: &str) -> Result<ProjectLease> {
        if self.handle(pid).is_err() {
            self.open_tracked(self.registered_directory(pid)?, false)?;
        }
        self.operation_lease(pid)
    }
    pub fn initialize_lake_inputs(&self) -> Result<()> {
        let db = self.registry.lock().map_err(lock_error)?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS lake_input_preparations(id TEXT PRIMARY KEY,project_id TEXT NOT NULL,state TEXT NOT NULL,json TEXT NOT NULL); CREATE INDEX IF NOT EXISTS lake_input_queue ON lake_input_preparations(state,id);").map_err(db_error)
    }
    pub fn lake_input_create(&self, row: &LakeInputPreparation) -> Result<LakeInputPreparation> {
        validate_id(&row.id)?;
        let db = self.registry.lock().map_err(lock_error)?;
        let old: Option<String> = db
            .query_row(
                "SELECT json FROM lake_input_preparations WHERE id=?1",
                [&row.id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?;
        if let Some(old) = old {
            let old: LakeInputPreparation = serde_json::from_str(&old).map_err(Error::io)?;
            if serde_json::to_value(&old.scope).map_err(Error::io)?
                != serde_json::to_value(&row.scope).map_err(Error::io)?
                || old.project_id != row.project_id
            {
                return Err(Error::new(
                    "IDEMPOTENCY_CONFLICT",
                    "准备请求标识已被不同范围使用",
                ));
            }
            return Ok(old);
        }
        db.execute(
            "INSERT INTO lake_input_preparations VALUES(?1,?2,?3,?4)",
            params![
                row.id,
                row.project_id,
                row.state,
                serde_json::to_string(row).map_err(Error::io)?
            ],
        )
        .map_err(db_error)?;
        Ok(row.clone())
    }
    pub fn lake_input_preparations(&self, pending: bool) -> Result<Vec<LakeInputPreparation>> {
        let db = self.registry.lock().map_err(lock_error)?;
        let mut stmt = db.prepare(if pending {"SELECT json FROM lake_input_preparations WHERE state IN ('preparing','cancelling') ORDER BY id LIMIT 1"} else {"SELECT json FROM lake_input_preparations ORDER BY rowid DESC LIMIT 100"}).map_err(db_error)?;
        stmt.query_map([], |r| r.get::<_, String>(0))
            .map_err(db_error)?
            .map(|v| serde_json::from_str(&v.map_err(db_error)?).map_err(Error::io))
            .collect()
    }
    pub fn lake_input_preparation(&self, id: &str) -> Result<LakeInputPreparation> {
        let db = self.registry.lock().map_err(lock_error)?;
        let text: String = db
            .query_row(
                "SELECT json FROM lake_input_preparations WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?
            .ok_or_else(|| Error::new("NOT_FOUND", "未找到范围准备记录"))?;
        serde_json::from_str(&text).map_err(Error::io)
    }
    pub fn lake_input_save(&self, row: &LakeInputPreparation) -> Result<()> {
        let db = self.registry.lock().map_err(lock_error)?;
        // Preserve a cancel against a successful in-flight append; cleanup errors
        // stop for review instead of retrying an unavailable disk forever.
        db.execute("UPDATE lake_input_preparations SET state=?2,json=?3 WHERE id=?1 AND (state!='cancelling' OR ?2 IN ('cancelled','needs_review'))",params![row.id,row.state,serde_json::to_string(row).map_err(Error::io)?]).map_err(db_error)?;
        Ok(())
    }
    pub fn lake_input_action(&self, id: &str, action: &str) -> Result<LakeInputPreparation> {
        let mut row = self.lake_input_preparation(id)?;
        match (action, row.state.as_str()) {
            ("resume", "needs_review") => {
                row.state = "preparing".into();
                row.error = None;
            }
            ("cancel", "preparing" | "needs_review") => row.state = "cancelling".into(),
            _ => return Err(Error::new("UPDATE_CONFLICT", "当前准备状态不接受此操作")),
        }
        self.lake_input_save(&row)?;
        Ok(row)
    }
    pub fn lake_input_pin(&self, pid: &str, id: &str, rid: &str, retain: bool) -> Result<()> {
        let p = self.handle(pid)?;
        let db = p.db.lock().map_err(lock_error)?;
        if retain {
            db.execute(
                "INSERT OR IGNORE INTO result_references VALUES('lake_input',?1,?2)",
                params![id, rid],
            )
            .map_err(db_error)?;
        } else {
            db.execute("DELETE FROM result_references WHERE owner_kind='lake_input' AND owner_id=?1 AND result_id=?2",params![id,rid]).map_err(db_error)?;
        }
        Ok(())
    }
    pub fn lake_input_pinned_result(&self, pid: &str, id: &str) -> Result<Option<String>> {
        let p = self.handle(pid)?;
        let db = p.read()?;
        db.query_row(
            "SELECT result_id FROM result_references WHERE owner_kind='lake_input' AND owner_id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_error)
    }
}
