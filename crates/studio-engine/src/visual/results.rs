use super::*;

impl Backend {
    pub fn metrics(&self, id: &str) -> Result<VisualMetrics> {
        self.ensure()?.store.metrics(id)
    }
    pub fn snapshots(&self, after: &str, limit: u32) -> Result<Vec<VisualSnapshot>> {
        self.ensure()?.store.snapshots(after, limit.clamp(1, 101))
    }
    pub fn snapshot(&self, id: &str) -> Result<VisualSnapshot> {
        self.ensure()?.store.snapshot(id)
    }
    pub fn create_snapshot(&self, name: &str, recipe_ids: &[String]) -> Result<VisualSnapshot> {
        if name.len() > 200 || recipe_ids.is_empty() || recipe_ids.len() > 3 {
            return Err(Error::invalid("快照需要 1-3 个配方和不超过 200 字节的名称"));
        }
        let _gate = self.gate.lock().map_err(lock)?;
        let session = self.ensure()?;
        let versions = session.store.version_map(recipe_ids)?;
        let id = studio_domain::new_id();
        for (rid, version) in &versions {
            self.worker(&session)?.request("pin",json!({"recipe":session.store.recipe(rid)?,"version":version,"tag":format!("snapshot_{id}")}))?;
        }
        session.store.save_snapshot(&id, name, &versions)
    }
    /// Cursor covers examined object identities, including objects without a
    /// vector in this snapshot. Empty pages with a cursor must be continued.
    pub fn feature_page(&self, request: VisualFeaturePageRequest, export: bool) -> Result<Value> {
        if request.after < 0
            || request.limit == 0
            || request.limit > if export { 4096 } else { 100 }
        {
            return Err(Error::invalid(
                "结果分页须为 1-100 项，导出分页须为 1-4096 项",
            ));
        }
        if export && request.snapshot_id.is_none() {
            return Err(Error::invalid("分批导出需要固定特征快照"));
        }
        let session = self.ensure()?;
        let recipe = session.store.recipe(&request.recipe_id)?;
        let version = session
            .store
            .selected_version(&recipe.id, request.snapshot_id.as_deref())?;
        let mut ids = session.store.feature_candidates(
            request.run_id.as_deref(),
            request.after,
            request.limit + 1,
        )?;
        let more = ids.len() > request.limit as usize;
        ids.truncate(request.limit as usize);
        let next_cursor = if more { ids.last().copied() } else { None };
        let _lease = self.resources.acquire(
            ReadRequest {
                class: ReadClass::NativeQuery,
                priority: ReadPriority::Interactive,
                bytes: 64 << 20,
            },
            &flag(),
        )?;
        let value = self.worker(&session)?.request(
            if export { "export_page" } else { "entries" },
            json!({"recipe":recipe,"version":version,"ids":ids}),
        )?;
        if export {
            return Ok(
                json!({"recipe_id":recipe.id,"snapshot_id":request.snapshot_id,"version":version,"next_cursor":next_cursor,"rows":value["rows"],"path":value["path"],"sha256":value["sha256"],"bytes":value["bytes"]}),
            );
        }
        let ids: Vec<i64> = decode(value["object_ids"].clone())?;
        let source_ids = if let Some(id) = &request.run_id {
            session.store.run(id)?.request.source_ids
        } else {
            Vec::new()
        };
        let items = session.store.matches(
            &ids.into_iter().map(|id| (id, None)).collect::<Vec<_>>(),
            &source_ids,
        )?;
        Ok(json!({"recipe_id":recipe.id,"version":version,"items":items,"next_cursor":next_cursor}))
    }
}
