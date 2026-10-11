//! Shared identities, physical plans, sparse coverage and atomic publication.
use fs2::FileExt;
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};
use studio_application::visual::VisualRepository;
use studio_domain::{AssetKey, Error, Result, new_id, visual::*};

pub struct VisualStore {
    root: PathBuf,
    db: Mutex<Connection>,
    _lease: File,
}
fn err(error: impl std::fmt::Display) -> Error {
    Error::new("VISUAL_STORAGE", error.to_string())
}
fn json(value: &impl serde::Serialize) -> Result<String> {
    serde_json::to_string(value).map_err(err)
}
fn parse<T: serde::de::DeserializeOwned>(value: &str) -> Result<T> {
    serde_json::from_str(value).map_err(err)
}
fn signed(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(err)
}
fn unsigned(row: &rusqlite::Row<'_>, column: usize) -> rusqlite::Result<u64> {
    u64::try_from(row.get::<_, i64>(column)?).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Integer,
            Box::new(e),
        )
    })
}
fn outcome(tx: &rusqlite::Transaction<'_>, item: &VisualOutcome, batch: &str) -> Result<()> {
    let old: Option<String> = tx
        .query_row(
            "SELECT status FROM outcomes WHERE object=?1 AND recipe=?2",
            params![item.object_id, item.recipe_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(err)?;
    if old.as_deref() == Some("success") {
        return Ok(());
    }
    tx.execute("INSERT INTO outcomes VALUES (?1,?2,?3,?4,?5) ON CONFLICT(object,recipe) DO UPDATE SET status=excluded.status,batch=excluded.batch,error=excluded.error",params![item.object_id,item.recipe_id,item.status,batch,item.error]).map_err(err)?;
    let success = i64::from(item.status == "success");
    let failed = i64::from(item.status == "failed") - i64::from(old.as_deref() == Some("failed"));
    tx.execute("INSERT INTO coverage_counts(source,recipe,succeeded,failed) SELECT source,?2,?3,?4 FROM bindings WHERE object=?1 ON CONFLICT(source,recipe) DO UPDATE SET succeeded=succeeded+excluded.succeeded,failed=failed+excluded.failed",params![item.object_id,item.recipe_id,success,failed]).map_err(err)?;
    Ok(())
}

impl VisualStore {
    pub fn open(root: &Path) -> Result<Self> {
        fs::create_dir_all(root).map_err(err)?;
        let root = root.canonicalize().map_err(err)?;
        let lease = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join("writer.lock"))
            .map_err(err)?;
        lease
            .try_lock_exclusive()
            .map_err(|_| Error::new("VISUAL_BUSY", "已有引擎管理此视觉材料目录"))?;
        let db = Connection::open(root.join("visual.sqlite")).map_err(err)?;
        db.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(err)?;
        let version: u32 = db
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(err)?;
        if version > 1 {
            return Err(Error::new("VISUAL_FORMAT", "视觉目录版本高于当前引擎"));
        }
        db.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA cache_size=-32768;",
        )
        .map_err(err)?;
        db.execute_batch(include_str!("schema.sql")).map_err(err)?;
        db.execute("UPDATE runs SET status='paused',error='引擎重启，可恢复未确认批次' WHERE status IN ('queued','planning','running','pausing','cancelling')", []).map_err(err)?;
        Ok(Self {
            root,
            db: Mutex::new(db),
            _lease: lease,
        })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    fn db(&self) -> Result<MutexGuard<'_, Connection>> {
        self.db.lock().map_err(err)
    }
    pub fn register_recipes(&self, recipes: &[VisualRecipe]) -> Result<()> {
        let mut db = self.db()?;
        let tx = db.transaction().map_err(err)?;
        for recipe in recipes {
            if recipe.id.len() != 64
                || !recipe.id.bytes().all(|b| b.is_ascii_hexdigit())
                || !matches!(
                    (recipe.model.as_str(), recipe.dimension),
                    ("siglip2_naflex", 1152) | ("embeddinggemma2", 768) | ("kaloscope3", 256)
                )
            {
                return Err(Error::invalid("视觉模型描述不兼容"));
            }
            // Locations can change; the immutable spec and its content identity cannot.
            let old: Option<String> = tx
                .query_row("SELECT json FROM recipes WHERE id=?1", [&recipe.id], |r| {
                    r.get(0)
                })
                .optional()
                .map_err(err)?;
            if let Some(old) = old {
                let saved: VisualRecipe = parse(&old)?;
                if saved.spec != recipe.spec {
                    return Err(Error::new("VISUAL_FORMAT", "配方身份与内容不一致"));
                }
            }
            tx.execute("INSERT INTO recipes VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET json=excluded.json", params![recipe.id,json(recipe)?]).map_err(err)?;
        }
        tx.commit().map_err(err)
    }
    pub fn recipe(&self, id: &str) -> Result<VisualRecipe> {
        let value: Option<String> = self
            .db()?
            .query_row("SELECT json FROM recipes WHERE id=?1", [id], |r| r.get(0))
            .optional()
            .map_err(err)?;
        parse(&value.ok_or_else(|| Error::new("NOT_FOUND", "视觉配方不存在"))?)
    }
    pub fn create_run(
        &self,
        request: &VisualRunRequest,
        sources: &[VisualSourceSnapshot],
    ) -> Result<VisualRun> {
        let id = new_id();
        self.db()?
            .execute(
                "INSERT INTO runs(id,status,request,sources) VALUES (?1,'queued',?2,?3)",
                params![id, json(request)?, json(&sources)?],
            )
            .map_err(err)?;
        self.run(&id)
    }
    pub fn runs(&self, after: u64, limit: u32) -> Result<Vec<VisualRun>> {
        let ids = {
            let db = self.db()?;
            let mut query = db
                .prepare("SELECT id FROM runs WHERE ordinal>?1 ORDER BY ordinal LIMIT ?2")
                .map_err(err)?;
            query
                .query_map(params![signed(after)?, limit.clamp(1, 100)], |r| {
                    r.get::<_, String>(0)
                })
                .map_err(err)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(err)?
        };
        ids.iter().map(|id| self.run(id)).collect()
    }
    pub fn status(&self, id: &str, status: &str, error: Option<&str>) -> Result<()> {
        if self
            .db()?
            .execute(
                "UPDATE runs SET status=?2,error=?3 WHERE id=?1",
                params![id, status, error],
            )
            .map_err(err)?
            == 0
        {
            return Err(Error::new("NOT_FOUND", "视觉任务不存在"));
        }
        Ok(())
    }
    pub fn refresh_sources(&self, id: &str, sources: &[VisualSourceSnapshot]) -> Result<()> {
        self.db()?
            .execute(
                "UPDATE runs SET sources=?2 WHERE id=?1",
                params![id, json(&sources)?],
            )
            .map_err(err)?;
        Ok(())
    }
    pub fn failures(&self, run: &str, after: i64, limit: u32) -> Result<Vec<VisualOutcome>> {
        let db = self.db()?;
        // Page by object, so all failed heads of that object travel together.
        let mut statement=db.prepare("SELECT f.object,f.recipe,'failed',f.error FROM run_failures f JOIN (SELECT DISTINCT object FROM run_failures WHERE run=?1 AND object>?2 ORDER BY object LIMIT ?3) selected ON selected.object=f.object WHERE f.run=?1 ORDER BY f.object,f.recipe").map_err(err)?;
        statement
            .query_map(params![run, after, limit.clamp(1, 100)], |r| {
                Ok(VisualOutcome {
                    object_id: r.get(0)?,
                    recipe_id: r.get(1)?,
                    status: r.get(2)?,
                    hash: None,
                    error: r.get(3)?,
                })
            })
            .map_err(err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)
    }
    pub fn discover(&self, run: &VisualRun, index: usize, page: &VisualMediaPage) -> Result<()> {
        let mut snapshots = run.sources.clone();
        let snapshot = snapshots
            .get_mut(index)
            .ok_or_else(|| Error::invalid("来源游标无效"))?;
        let mut db = self.db()?;
        let tx = db.transaction().map_err(err)?;
        tx.execute(
            "INSERT OR IGNORE INTO sources(source_id) VALUES (?1)",
            [&snapshot.source.id],
        )
        .map_err(err)?;
        let sid: i64 = tx
            .query_row(
                "SELECT id FROM sources WHERE source_id=?1",
                [&snapshot.source.id],
                |r| r.get(0),
            )
            .map_err(err)?;
        let mut added = 0;
        let mut reused = 0i64;
        let required: Vec<_> = std::iter::once(HASH_RECIPE)
            .chain(run.request.recipe_ids.iter().map(String::as_str))
            .collect();
        let required_json = json(&required)?;
        for item in &page.items {
            tx.execute(
                "INSERT OR IGNORE INTO objects(sha256) VALUES (?1)",
                [&item.sha256],
            )
            .map_err(err)?;
            let oid: i64 = tx
                .query_row(
                    "SELECT id FROM objects WHERE sha256=?1",
                    [&item.sha256],
                    |r| r.get(0),
                )
                .map_err(err)?;
            if tx
                .execute(
                    "INSERT OR IGNORE INTO bindings VALUES (?1,?2)",
                    params![sid, oid],
                )
                .map_err(err)?
                == 1
            {
                tx.execute(
                    "UPDATE sources SET discovered=discovered+1 WHERE id=?1",
                    [sid],
                )
                .map_err(err)?;
                tx.execute("INSERT INTO coverage_counts(source,recipe,succeeded,failed) SELECT ?1,recipe,status='success',status='failed' FROM outcomes WHERE object=?2 ON CONFLICT(source,recipe) DO UPDATE SET succeeded=succeeded+excluded.succeeded,failed=failed+excluded.failed",params![sid,oid]).map_err(err)?;
                tx.execute("INSERT OR IGNORE INTO projection_pending SELECT recipe,object FROM outcomes WHERE object=?1 AND status='success' AND recipe!=?2",params![oid,HASH_RECIPE]).map_err(err)?;
            }
            let covered:u32=tx.prepare_cached("SELECT count(*) FROM outcomes WHERE object=?1 AND status='success' AND recipe IN (SELECT value FROM json_each(?2))").map_err(err)?.query_row(params![oid,required_json],|r|r.get(0)).map_err(err)?;
            let projection_pending: bool = tx
                .prepare_cached("SELECT EXISTS(SELECT 1 FROM projection_pending WHERE object=?1)")
                .map_err(err)?
                .query_row([oid], |r| r.get(0))
                .map_err(err)?;
            if covered as usize == required.len() && !projection_pending {
                reused += 1;
            } else {
                added += tx.prepare_cached("INSERT OR IGNORE INTO plan(run,object,source,pack,offset,length) VALUES (?1,?2,?3,?4,?5,?6)").map_err(err)?.execute(params![run.id,oid,sid,item.pack,signed(item.offset)?,signed(item.length)?]).map_err(err)?;
            }
            snapshot.after_row = item.row_id;
        }
        snapshot.discovered += page.items.len() as u64;
        snapshot.complete = page.complete
            || run
                .request
                .max_objects_per_source
                .is_some_and(|n| snapshot.discovered >= n);
        tx.execute(
            "UPDATE runs SET sources=?2,planned=planned+?3,reused=reused+?4 WHERE id=?1",
            params![run.id, json(&snapshots)?, added as i64, reused],
        )
        .map_err(err)?;
        tx.commit().map_err(err)
    }
    pub fn pending_batch(&self, run: &str) -> Result<Option<VisualBatchRequest>> {
        let value: Option<String> = self
            .db()?
            .query_row(
                "SELECT request FROM batches WHERE run=?1 AND state='pending' LIMIT 1",
                [run],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        value.map(|s| parse(&s)).transpose()
    }
    pub fn pending_any(&self) -> Result<Option<VisualBatchRequest>> {
        let value: Option<String> = self
            .db()?
            .query_row(
                "SELECT request FROM batches WHERE state='pending' LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        value.map(|s| parse(&s)).transpose()
    }
    pub fn next_batch(&self, run: &VisualRun) -> Result<Option<VisualBatchRequest>> {
        self.next_batch_with_cancel(run, &std::sync::atomic::AtomicBool::new(false))
    }
    pub fn next_batch_with_cancel(
        &self,
        run: &VisualRun,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<Option<VisualBatchRequest>> {
        studio_application::read_cancelled(cancelled)?;
        if let Some(batch) = self.pending_batch(&run.id)? {
            return Ok(Some(batch));
        }
        loop {
            studio_application::read_cancelled(cancelled)?;
            let batch = self.select_batch(run)?;
            if batch.is_some() {
                return Ok(batch);
            }
            let remaining: bool = self
                .db()?
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM plan WHERE run=?1 AND state=0)",
                    [&run.id],
                    |r| r.get(0),
                )
                .map_err(err)?;
            if !remaining {
                return Ok(None);
            }
        }
    }
    pub fn has_unfinished(&self) -> Result<bool> {
        self.db()?.query_row("SELECT EXISTS(SELECT 1 FROM runs WHERE status NOT IN ('completed','completed_with_errors','cancelled'))",[],|r|r.get(0)).map_err(err)
    }
    fn select_batch(&self, run: &VisualRun) -> Result<Option<VisualBatchRequest>> {
        let all_recipes = self.recipes()?;
        let recipes = run
            .request
            .recipe_ids
            .iter()
            .map(|id| self.recipe(id))
            .collect::<Result<Vec<_>>>()?;
        let mut db = self.db()?;
        let tx = db.transaction().map_err(err)?;
        let mut items = Vec::new();
        let mut total = 0;
        let mut query = tx.prepare("SELECT p.object,o.sha256,s.source_id,p.pack,p.offset,p.length,EXISTS(SELECT 1 FROM hashes h WHERE h.object=p.object) FROM plan p JOIN objects o ON o.id=p.object JOIN sources s ON s.id=p.source WHERE p.run=?1 AND p.state=0 ORDER BY p.source,p.pack,p.offset,p.object LIMIT ?2").map_err(err)?;
        let mut rows = query
            .query(params![run.id, run.request.batch_size])
            .map_err(err)?;
        let mut oversize = Vec::new();
        while let Some(row) = rows.next().map_err(err)? {
            let length = unsigned(row, 5).map_err(err)?;
            let oid: i64 = row.get(0).map_err(err)?;
            if length == 0 || length > MEDIA_BATCH_BYTES {
                oversize.push(oid);
                continue;
            }
            if total + length > MEDIA_BATCH_BYTES {
                break;
            }
            let source_id: String = row.get(2).map_err(err)?;
            let source = run
                .sources
                .iter()
                .find(|s| s.source.id == source_id)
                .ok_or_else(|| err("plan source absent"))?;
            let mut associations = tx
                .prepare("SELECT source FROM bindings WHERE object=?1 ORDER BY source")
                .map_err(err)?;
            let source_ids = associations
                .query_map([oid], |r| r.get(0))
                .map_err(err)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(err)?;
            items.push(VisualPlanItem {
                object_id: oid,
                sha256: row.get(1).map_err(err)?,
                source_ids,
                source_id,
                media_root: source
                    .source
                    .media_root
                    .clone()
                    .ok_or_else(|| err("media root missing"))?,
                pack: row.get(3).map_err(err)?,
                offset: unsigned(row, 4).map_err(err)?,
                length,
                hash_done: row.get(6).map_err(err)?,
            });
            total += length;
        }
        drop(rows);
        drop(query);
        for oid in &oversize {
            for rid in std::iter::once(HASH_RECIPE)
                .chain(run.request.recipe_ids.iter().map(String::as_str))
            {
                outcome(
                    &tx,
                    &VisualOutcome {
                        object_id: *oid,
                        recipe_id: rid.into(),
                        status: "failed".into(),
                        hash: None,
                        error: Some("compressed_byte_limit".into()),
                    },
                    "discovery",
                )?;
                tx.execute(
                    "INSERT OR REPLACE INTO run_failures VALUES (?1,?2,?3,'compressed_byte_limit')",
                    params![run.id, oid, rid],
                )
                .map_err(err)?;
            }
            tx.execute(
                "UPDATE plan SET state=1 WHERE run=?1 AND object=?2",
                params![run.id, oid],
            )
            .map_err(err)?;
        }
        if !oversize.is_empty() {
            tx.execute(
                "UPDATE runs SET completed=completed+?2,failed=failed+?3 WHERE id=?1",
                params![
                    run.id,
                    oversize.len() as i64,
                    (oversize.len() * (recipes.len() + 1)) as i64
                ],
            )
            .map_err(err)?;
        }
        if items.is_empty() {
            tx.commit().map_err(err)?;
            return Ok(None);
        }
        let batch = VisualBatchRequest {
            batch_id: new_id(),
            items,
            recipes,
            projection_recipes: all_recipes,
        };
        tx.execute(
            "INSERT INTO batches VALUES (?1,?2,'pending',?3,NULL)",
            params![batch.batch_id, run.id, json(&batch)?],
        )
        .map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(Some(batch))
    }
    pub fn publish(&self, receipt: &VisualBatchReceipt) -> Result<()> {
        let mut db = self.db()?;
        let tx = db.transaction().map_err(err)?;
        let (run, state, request): (String, String, String) = tx
            .query_row(
                "SELECT run,state,request FROM batches WHERE id=?1",
                [&receipt.batch_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(err)?;
        if state == "published" {
            return Ok(());
        }
        let request: VisualBatchRequest = parse(&request)?;
        let allowed: BTreeSet<_> = request.items.iter().map(|i| i.object_id).collect();
        let processed: BTreeSet<_> = receipt.object_ids.iter().copied().collect();
        let recipes: BTreeSet<_> = std::iter::once(HASH_RECIPE)
            .chain(request.recipes.iter().map(|r| r.id.as_str()))
            .collect();
        if processed.is_empty()
            || processed.len() != receipt.object_ids.len()
            || !processed.is_subset(&allowed)
            || receipt.bytes_read > MEDIA_BATCH_BYTES
        {
            return Err(Error::new("VISUAL_PROTOCOL", "视觉批次回执的范围无效"));
        }
        let mut seen = BTreeSet::new();
        let mut failed = 0;
        for item in &receipt.results {
            if !processed.contains(&item.object_id)
                || !recipes.contains(item.recipe_id.as_str())
                || !seen.insert((item.object_id, item.recipe_id.clone()))
                || !matches!(item.status.as_str(), "success" | "failed")
            {
                return Err(Error::new("VISUAL_PROTOCOL", "视觉批次结果与意图不一致"));
            }
            if item.status == "success" {
                if item.recipe_id == HASH_RECIPE {
                    let hash = item.hash.as_deref().ok_or_else(|| err("hash missing"))?;
                    if hash.len() != 16 {
                        return Err(err("invalid pHash"));
                    }
                    let bits = u64::from_str_radix(hash, 16).map_err(err)?;
                    let old: Option<String> = tx
                        .query_row(
                            "SELECT value FROM hashes WHERE object=?1",
                            [item.object_id],
                            |r| r.get(0),
                        )
                        .optional()
                        .map_err(err)?;
                    if old.as_deref().is_some_and(|old| old != hash) {
                        return Err(Error::new(
                            "VISUAL_INTEGRITY",
                            "同一对象的固定 pHash 配方产生不同结果",
                        ));
                    }
                    tx.execute("INSERT INTO hashes VALUES (?1,?2) ON CONFLICT(object) DO UPDATE SET value=excluded.value", params![item.object_id,hash]).map_err(err)?;
                    for band in 0..8 {
                        tx.execute(
                            "INSERT OR IGNORE INTO hash_bands VALUES (?1,?2,?3)",
                            params![band, ((bits >> (band * 8)) & 255) as i64, item.object_id],
                        )
                        .map_err(err)?;
                    }
                } else if !receipt.versions.contains_key(&item.recipe_id) {
                    return Err(Error::new("VISUAL_PROTOCOL", "成功向量缺少存储版本"));
                }
                tx.execute(
                    "UPDATE objects SET verified=1 WHERE id=?1",
                    [item.object_id],
                )
                .map_err(err)?;
            } else {
                failed += 1;
                tx.execute(
                    "INSERT OR REPLACE INTO run_failures VALUES (?1,?2,?3,?4)",
                    params![run, item.object_id, item.recipe_id, item.error],
                )
                .map_err(err)?;
            }
            outcome(&tx, item, &receipt.batch_id)?;
        }
        for oid in &processed {
            for rid in &recipes {
                let exists: bool = tx
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM outcomes WHERE object=?1 AND recipe=?2)",
                        params![oid, rid],
                        |r| r.get(0),
                    )
                    .map_err(err)?;
                if !exists {
                    return Err(Error::new("VISUAL_PROTOCOL", "回执遗漏对象配方结果"));
                }
            }
            tx.execute(
                "UPDATE plan SET state=1 WHERE run=?1 AND object=?2",
                params![run, oid],
            )
            .map_err(err)?;
        }
        for (recipe, version) in &receipt.versions {
            if !request
                .projection_recipes
                .iter()
                .chain(request.recipes.iter())
                .any(|r| r.id == *recipe)
            {
                return Err(err("unexpected version"));
            }
            tx.execute("INSERT INTO versions VALUES (?1,?2) ON CONFLICT(recipe) DO UPDATE SET version=max(version,excluded.version)", params![recipe,signed(*version)?]).map_err(err)?;
            for oid in &processed {
                tx.execute(
                    "DELETE FROM projection_pending WHERE recipe=?1 AND object=?2",
                    params![recipe, oid],
                )
                .map_err(err)?;
            }
        }
        tx.execute(
            "UPDATE batches SET state='published',receipt=?2 WHERE id=?1",
            params![receipt.batch_id, json(receipt)?],
        )
        .map_err(err)?;
        tx.execute("UPDATE runs SET completed=completed+?2,failed=failed+?3,bytes_read=bytes_read+?4 WHERE id=?1", params![run,processed.len() as i64,failed,signed(receipt.bytes_read)?]).map_err(err)?;
        tx.commit().map_err(err)
    }
    pub fn version(&self, recipe: &str) -> Result<u64> {
        self.db()?
            .query_row(
                "SELECT version FROM versions WHERE recipe=?1",
                [recipe],
                |r| unsigned(r, 0),
            )
            .optional()
            .map_err(err)?
            .ok_or_else(|| Error::new("RESULT_NOT_READY", "该配方尚无已发布向量"))
    }
    pub fn projection_ready(&self, recipe: &str) -> Result<bool> {
        self.db()?
            .query_row(
                "SELECT NOT EXISTS(SELECT 1 FROM projection_pending WHERE recipe=?1)",
                [recipe],
                |r| r.get(0),
            )
            .map_err(err)
    }
    pub fn publish_index(&self, recipe: &str, version: u64) -> Result<()> {
        self.db()?
            .execute(
                "UPDATE versions SET version=max(version,?2) WHERE recipe=?1",
                params![recipe, signed(version)?],
            )
            .map_err(err)?;
        Ok(())
    }
    pub fn object(&self, key: &AssetKey) -> Result<i64> {
        self.db()?.query_row("SELECT o.id FROM objects o JOIN bindings b ON b.object=o.id JOIN sources s ON s.id=b.source WHERE o.sha256=?1 AND s.source_id=?2", params![key.asset_id,key.source_id], |r| r.get(0)).optional().map_err(err)?.ok_or_else(|| Error::new("NOT_FOUND", "图片尚未进入视觉目录"))
    }
    pub fn source_numbers(&self, sources: &[String]) -> Result<Vec<i64>> {
        let db = self.db()?;
        sources
            .iter()
            .map(|id| {
                db.query_row("SELECT id FROM sources WHERE source_id=?1", [id], |r| {
                    r.get(0)
                })
                .optional()
                .map_err(err)?
                .ok_or_else(|| Error::new("NOT_FOUND", "视觉来源不存在"))
            })
            .collect()
    }
    pub fn matches(
        &self,
        ids: &[(i64, Option<f64>)],
        source_filter: &[String],
    ) -> Result<Vec<VisualMatch>> {
        let db = self.db()?;
        ids.iter().map(|(id,distance)| {
            let sha256: String = db.query_row("SELECT sha256 FROM objects WHERE id=?1", [id], |r| r.get(0)).map_err(err)?;
            let mut query = db.prepare("SELECT s.source_id FROM bindings b JOIN sources s ON s.id=b.source WHERE b.object=?1 ORDER BY s.source_id").map_err(err)?;
            let sources = query.query_map([id], |r| r.get::<_,String>(0)).map_err(err)?.collect::<std::result::Result<Vec<_>,_>>().map_err(err)?;
            let assets = sources.into_iter().filter(|s| source_filter.is_empty() || source_filter.contains(s)).map(|source_id| AssetKey { source_id, asset_id: sha256.clone() }).collect();
            Ok(VisualMatch { object_id:*id,sha256,assets,distance:*distance })
        }).collect()
    }
    pub fn duplicates(
        &self,
        query: Option<&AssetKey>,
        after: i64,
        limit: u32,
        radius: u32,
    ) -> Result<(Vec<VisualMatch>, Option<i64>)> {
        if limit == 0 || limit > 100 || radius > 7 || after < 0 {
            return Err(Error::invalid("pHash 半径为 0-7，分页大小为 1-100"));
        }
        let oid = query.map(|q| self.object(q)).transpose()?;
        let mut ids = Vec::new();
        let mut cursor = after;
        let mut more = false;
        {
            let db = self.db()?;
            if let Some(oid) = oid {
                let hash: String = db
                    .query_row("SELECT value FROM hashes WHERE object=?1", [oid], |r| {
                        r.get(0)
                    })
                    .optional()
                    .map_err(err)?
                    .ok_or_else(|| Error::new("RESULT_NOT_READY", "查询图片尚无 pHash"))?;
                let bits = u64::from_str_radix(&hash, 16).map_err(err)?;
                // At radius <= 7, one of eight disjoint bytes is unchanged. UNION
                // gives complete candidates, and keyset pages handle crowded buckets.
                let union = (0..8).map(|band| format!("SELECT object FROM hash_bands WHERE band={band} AND bits={} AND object>{after}", (bits >> (band*8)) & 255)).collect::<Vec<_>>().join(" UNION ");
                let sql = format!(
                    "SELECT h.object,h.value FROM hashes h JOIN ({union}) c ON c.object=h.object ORDER BY h.object LIMIT 50001"
                );
                let mut statement = db.prepare(&sql).map_err(err)?;
                let mut rows = statement.query([]).map_err(err)?;
                let mut examined = 0;
                while let Some(row) = rows.next().map_err(err)? {
                    let id: i64 = row.get(0).map_err(err)?;
                    if examined == 50000 || ids.len() == limit as usize {
                        more = true;
                        break;
                    }
                    cursor = id;
                    examined += 1;
                    if id == oid {
                        continue;
                    }
                    let candidate: String = row.get(1).map_err(err)?;
                    let distance =
                        (bits ^ u64::from_str_radix(&candidate, 16).map_err(err)?).count_ones();
                    if distance <= radius {
                        ids.push((id, Some(f64::from(distance))));
                    }
                }
            } else {
                let mut statement = db.prepare("SELECT b.object FROM bindings b JOIN objects o ON o.id=b.object WHERE b.object>?1 GROUP BY b.object HAVING count(*)>1 ORDER BY b.object LIMIT ?2").map_err(err)?;
                let values = statement
                    .query_map(params![after, limit + 1], |r| r.get::<_, i64>(0))
                    .map_err(err)?
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(err)?;
                more = values.len() > limit as usize;
                for id in values.into_iter().take(limit as usize) {
                    cursor = id;
                    ids.push((id, None));
                }
            }
        }
        Ok((self.matches(&ids, &[])?, more.then_some(cursor)))
    }
}

impl VisualRepository for VisualStore {
    fn recipes(&self) -> Result<Vec<VisualRecipe>> {
        let db = self.db()?;
        let mut query = db
            .prepare("SELECT json FROM recipes ORDER BY id")
            .map_err(err)?;
        query
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(err)?
            .map(|r| parse(&r.map_err(err)?))
            .collect()
    }
    fn run(&self, id: &str) -> Result<VisualRun> {
        let db = self.db()?;
        let mut statement = db.prepare("SELECT ordinal,status,request,sources,planned,completed,failed,bytes_read,error,reused FROM runs WHERE id=?1").map_err(err)?;
        let mut rows = statement.query([id]).map_err(err)?;
        let row = rows
            .next()
            .map_err(err)?
            .ok_or_else(|| Error::new("NOT_FOUND", "视觉任务不存在"))?;
        Ok(VisualRun {
            id: id.into(),
            ordinal: unsigned(row, 0).map_err(err)?,
            status: row.get(1).map_err(err)?,
            request: parse(&row.get::<_, String>(2).map_err(err)?)?,
            sources: parse(&row.get::<_, String>(3).map_err(err)?)?,
            planned_objects: unsigned(row, 4).map_err(err)?,
            completed_objects: unsigned(row, 5).map_err(err)?,
            reused_bindings: unsigned(row, 9).map_err(err)?,
            failed_features: unsigned(row, 6).map_err(err)?,
            bytes_read: unsigned(row, 7).map_err(err)?,
            error: row.get(8).map_err(err)?,
        })
    }
    fn coverage(&self) -> Result<Vec<VisualCoverage>> {
        let recipes = self.recipes()?;
        let db = self.db()?;
        let mut totals = BTreeMap::new();
        let mut statement = db
            .prepare("SELECT source_id,discovered FROM sources")
            .map_err(err)?;
        for row in statement
            .query_map([], |r| Ok((r.get::<_, String>(0)?, unsigned(r, 1)?)))
            .map_err(err)?
        {
            let (id, n) = row.map_err(err)?;
            totals.insert(id, n);
        }
        let mut result = Vec::new();
        for (source, discovered) in totals {
            for recipe in std::iter::once(HASH_RECIPE).chain(recipes.iter().map(|r| r.id.as_str()))
            {
                let (succeeded,failed) = db.query_row("SELECT c.succeeded,c.failed FROM coverage_counts c JOIN sources s ON s.id=c.source WHERE s.source_id=?1 AND c.recipe=?2", params![source,recipe], |r| Ok((unsigned(r,0)?,unsigned(r,1)?))).optional().map_err(err)?.unwrap_or((0,0));
                result.push(VisualCoverage {
                    source_id: source.clone(),
                    recipe_id: recipe.into(),
                    discovered,
                    succeeded,
                    failed,
                    pending: discovered.saturating_sub(succeeded + failed),
                });
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
