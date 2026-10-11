use super::*;

pub(super) fn namespace(snapshot: &VisualSourceSnapshot) -> String {
    if !snapshot.discovery_namespace.is_empty() {
        return snapshot.discovery_namespace.clone();
    }
    // Upgrade the v1/v2 Booru snapshots, whose revision encoded generation:seq.
    snapshot
        .version
        .catalog_revision
        .rsplit_once(':')
        .map(|(n, _)| n)
        .unwrap_or(&snapshot.version.catalog_revision)
        .to_owned()
}
pub(super) fn seed_discovery(db: &Connection) -> Result<()> {
    let mut q = db
        .prepare("SELECT request,sources FROM runs")
        .map_err(err)?;
    let mut rows = q.query([]).map_err(err)?;
    while let Some(row) = rows.next().map_err(err)? {
        let request: VisualRunRequest = parse(&row.get::<_, String>(0).map_err(err)?)?;
        if request.scope_run_id.is_some() || !request.assets.is_empty() {
            continue;
        }
        let sources: Vec<VisualSourceSnapshot> = parse(&row.get::<_, String>(1).map_err(err)?)?;
        for s in sources {
            db.execute("INSERT INTO source_cursors VALUES (?1,?2,?3) ON CONFLICT(source_id,namespace) DO UPDATE SET after_row=max(after_row,excluded.after_row)",params![s.source.id,namespace(&s),signed(s.after_row)?]).map_err(err)?;
        }
    }
    Ok(())
}

impl VisualStore {
    pub fn last_index_result(&self, recipe: &str) -> Result<Option<serde_json::Value>> {
        let value:Option<String>=self.db()?.query_row("SELECT result FROM index_jobs WHERE json_extract(request,'$.recipe_id')=?1 AND status='completed' ORDER BY ordinal DESC LIMIT 1",[recipe],|r|r.get(0)).optional().map_err(err)?;
        value.map(|s| parse(&s)).transpose()
    }
    pub fn discovery_cursor(&self, source: &str, namespace: &str) -> Result<u64> {
        Ok(self
            .db()?
            .query_row(
                "SELECT after_row FROM source_cursors WHERE source_id=?1 AND namespace=?2",
                params![source, namespace],
                |r| unsigned(r, 0),
            )
            .optional()
            .map_err(err)?
            .unwrap_or(0))
    }
    pub fn discovery_cursors(&self) -> Result<Vec<VisualDiscoveryCursor>> {
        let db = self.db()?;
        let mut q=db.prepare("SELECT source_id,namespace,after_row FROM source_cursors ORDER BY source_id,namespace").map_err(err)?;
        q.query_map([], |r| {
            Ok(VisualDiscoveryCursor {
                source_id: r.get(0)?,
                namespace: r.get(1)?,
                after_row: unsigned(r, 2)?,
            })
        })
        .map_err(err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(err)
    }
    pub fn unfinished_index(&self) -> Result<Option<String>> {
        self.db()?
            .query_row(
                "SELECT id FROM index_jobs WHERE status NOT IN ('completed','cancelled') LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)
    }
    pub fn create_index_job(
        &self,
        request: &VisualIndexRequest,
        input_version: u64,
    ) -> Result<VisualIndexJob> {
        let id = request.request_id.clone().unwrap_or_else(new_id);
        let progress = VisualIndexProgress {
            phase: "queued".into(),
            ..Default::default()
        };
        self.db()?.execute("INSERT INTO index_jobs(id,status,request,input_version,progress) VALUES (?1,'queued',?2,?3,?4)",params![id,json(request)?,signed(input_version)?,json(&progress)?]).map_err(err)?;
        self.index_job(&id)
    }
    pub fn index_job(&self, id: &str) -> Result<VisualIndexJob> {
        let db = self.db()?;
        let mut q=db.prepare("SELECT ordinal,status,request,input_version,progress,result,error FROM index_jobs WHERE id=?1").map_err(err)?;
        let mut rows = q.query([id]).map_err(err)?;
        let r = rows
            .next()
            .map_err(err)?
            .ok_or_else(|| Error::new("NOT_FOUND", "索引任务不存在"))?;
        let result: Option<String> = r.get(5).map_err(err)?;
        Ok(VisualIndexJob {
            id: id.into(),
            ordinal: unsigned(r, 0).map_err(err)?,
            status: r.get(1).map_err(err)?,
            request: parse(&r.get::<_, String>(2).map_err(err)?)?,
            input_version: unsigned(r, 3).map_err(err)?,
            progress: parse(&r.get::<_, String>(4).map_err(err)?)?,
            result: result.map(|v| parse(&v)).transpose()?,
            error: r.get(6).map_err(err)?,
            worker_running: false,
        })
    }
    pub fn index_jobs(&self, after: u64, limit: u32) -> Result<Vec<VisualIndexJob>> {
        let ids = {
            let db = self.db()?;
            let mut q = db
                .prepare("SELECT id FROM index_jobs WHERE ordinal>?1 ORDER BY ordinal LIMIT ?2")
                .map_err(err)?;
            q.query_map(params![signed(after)?, limit.clamp(1, 101)], |r| {
                r.get::<_, String>(0)
            })
            .map_err(err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)?
        };
        ids.iter().map(|id| self.index_job(id)).collect()
    }
    pub fn index_status(
        &self,
        id: &str,
        status: &str,
        error: Option<&str>,
        progress: Option<&VisualIndexProgress>,
    ) -> Result<()> {
        self.db()?.execute("UPDATE index_jobs SET status=?2,error=?3,progress=coalesce(?4,progress) WHERE id=?1",params![id,status,error,progress.map(json).transpose()?]).map_err(err)?;
        Ok(())
    }
    pub fn publish_index_job(&self, id: &str, result: &serde_json::Value) -> Result<()> {
        let job = self.index_job(id)?;
        if job.status == "completed" {
            return Ok(());
        }
        let version = result["version"]
            .as_u64()
            .filter(|v| *v >= job.input_version)
            .ok_or_else(|| err("invalid index publication version"))?;
        let mut db = self.db()?;
        let tx = db.transaction().map_err(err)?;
        let current: u64 = tx
            .query_row(
                "SELECT version FROM versions WHERE recipe=?1",
                [&job.request.recipe_id],
                |r| unsigned(r, 0),
            )
            .map_err(err)?;
        if current != job.input_version {
            return Err(err("index input was superseded by another writer"));
        }
        tx.execute(
            "UPDATE versions SET version=?2 WHERE recipe=?1",
            params![job.request.recipe_id, signed(version)?],
        )
        .map_err(err)?;
        let progress = VisualIndexProgress {
            phase: "completed".into(),
            completed_units: result["rows"].as_u64(),
            total_units: result["rows"].as_u64(),
            unit: "vectors".into(),
            updated_at_ms: 0,
        };
        tx.execute(
            "UPDATE index_jobs SET status='completed',result=?2,progress=?3,error=NULL WHERE id=?1",
            params![id, json(result)?, json(&progress)?],
        )
        .map_err(err)?;
        tx.commit().map_err(err)
    }
    pub fn retention_policy(&self) -> Result<VisualRetentionPolicy> {
        let value: Option<String> = self
            .db()?
            .query_row("SELECT json FROM settings WHERE key='retention'", [], |r| {
                r.get(0)
            })
            .optional()
            .map_err(err)?;
        Ok(value.map(|v| parse(&v)).transpose()?.unwrap_or_default())
    }
    pub fn set_retention_policy(&self, policy: &VisualRetentionPolicy) -> Result<()> {
        self.db()?.execute("INSERT INTO settings VALUES ('retention',?1) ON CONFLICT(key) DO UPDATE SET json=excluded.json",[json(policy)?]).map_err(err)?;
        Ok(())
    }
    pub fn protected_versions(&self, recipe: &str) -> Result<Vec<u64>> {
        let db = self.db()?;
        let mut q=db.prepare("SELECT DISTINCT CAST(j.value AS INTEGER) FROM snapshots s,json_each(s.versions) j WHERE j.key=?1").map_err(err)?;
        q.query_map([recipe], |r| unsigned(r, 0))
            .map_err(err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)
    }
}
