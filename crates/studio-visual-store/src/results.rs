use super::*;

impl VisualStore {
    pub fn version_map(&self, recipes: &[String]) -> Result<BTreeMap<String, u64>> {
        let db = self.db()?;
        recipes
            .iter()
            .map(|id| {
                let version = db
                    .query_row("SELECT version FROM versions WHERE recipe=?1", [id], |r| {
                        unsigned(r, 0)
                    })
                    .optional()
                    .map_err(err)?
                    .ok_or_else(|| Error::new("RESULT_NOT_READY", "配方尚未发布"))?;
                Ok((id.clone(), version))
            })
            .collect()
    }
    /// A reusable scope includes covered bindings too. Pre-v2 runs deliberately
    /// cannot masquerade as complete membership because they did not save it.
    pub fn scope_root(&self, run: &str) -> Result<String> {
        self.db()?
            .query_row(
                "SELECT member_run FROM run_scopes WHERE run=?1 AND ready=1",
                [run],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?
            .ok_or_else(|| {
                Error::new(
                    "RESULT_NOT_READY",
                    "任务范围尚未完整发现，或来自未保存范围的旧版任务",
                )
            })
    }
    pub fn scope_page(
        &self,
        scope: &str,
        source: &str,
        after: u64,
        limit: u32,
    ) -> Result<VisualMediaPage> {
        let db = self.db()?;
        let mut q = db.prepare("SELECT m.row_id,o.sha256,m.pack,m.offset,m.length FROM members m JOIN sources s ON s.id=m.source JOIN objects o ON o.id=m.object WHERE m.scope=?1 AND s.source_id=?2 AND m.row_id>?3 ORDER BY m.row_id LIMIT ?4").map_err(err)?;
        let mut items = q
            .query_map(params![scope, source, signed(after)?, limit + 1], |r| {
                Ok(VisualMediaRef {
                    row_id: unsigned(r, 0)?,
                    sha256: r.get(1)?,
                    pack: r.get(2)?,
                    offset: unsigned(r, 3)?,
                    length: unsigned(r, 4)?,
                    extension: String::new(),
                })
            })
            .map_err(err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)?;
        let complete = items.len() <= limit as usize;
        items.truncate(limit as usize);
        Ok(VisualMediaPage { items, complete })
    }
    pub fn feature_candidates(
        &self,
        run: Option<&str>,
        after: i64,
        limit: u32,
    ) -> Result<Vec<i64>> {
        let scope = run.map(|id| self.scope_root(id)).transpose()?;
        let db = self.db()?;
        if let Some(scope) = scope {
            let mut q = db.prepare("SELECT DISTINCT object FROM members WHERE scope=?1 AND object>?2 ORDER BY object LIMIT ?3").map_err(err)?;
            q.query_map(params![scope, after, limit], |r| r.get(0))
                .map_err(err)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(err)
        } else {
            let mut q = db
                .prepare("SELECT id FROM objects WHERE id>?1 ORDER BY id LIMIT ?2")
                .map_err(err)?;
            q.query_map(params![after, limit], |r| r.get(0))
                .map_err(err)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(err)
        }
    }
    pub fn metrics(&self, run: &str) -> Result<VisualMetrics> {
        self.run(run)?;
        let value: Option<String> = self
            .db()?
            .query_row("SELECT json FROM run_metrics WHERE run=?1", [run], |r| {
                r.get(0)
            })
            .optional()
            .map_err(err)?;
        Ok(value.map(|v| parse(&v)).transpose()?.unwrap_or_default())
    }
    pub fn save_snapshot(
        &self,
        id: &str,
        name: &str,
        versions: &BTreeMap<String, u64>,
    ) -> Result<VisualSnapshot> {
        self.db()?
            .execute(
                "INSERT INTO snapshots(id,name,versions) VALUES (?1,?2,?3)",
                params![id, name, json(versions)?],
            )
            .map_err(err)?;
        self.snapshot(id)
    }
    pub fn snapshot(&self, id: &str) -> Result<VisualSnapshot> {
        let value: Option<(String, String, String)> = self
            .db()?
            .query_row(
                "SELECT name,created_at,versions FROM snapshots WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(err)?;
        let (name, created_at, versions) =
            value.ok_or_else(|| Error::new("NOT_FOUND", "特征快照不存在"))?;
        Ok(VisualSnapshot {
            id: id.into(),
            name,
            created_at,
            versions: parse(&versions)?,
        })
    }
    pub fn snapshots(&self, after: &str, limit: u32) -> Result<Vec<VisualSnapshot>> {
        let ids = {
            let db = self.db()?;
            let mut q = db
                .prepare("SELECT id FROM snapshots WHERE id>?1 ORDER BY id LIMIT ?2")
                .map_err(err)?;
            q.query_map(params![after, limit], |r| r.get::<_, String>(0))
                .map_err(err)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(err)?
        };
        ids.iter().map(|id| self.snapshot(id)).collect()
    }
    pub fn selected_version(&self, recipe: &str, snapshot: Option<&str>) -> Result<u64> {
        if let Some(id) = snapshot {
            self.snapshot(id)?
                .versions
                .get(recipe)
                .copied()
                .ok_or_else(|| Error::new("NOT_FOUND", "该快照不包含此配方"))
        } else {
            self.version(recipe)
        }
    }
}
