use crate::*;

fn same_path(a: &Option<PathBuf>, b: &Option<PathBuf>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            a.canonicalize().unwrap_or_else(|_| a.clone())
                == b.canonicalize().unwrap_or_else(|_| b.clone())
        }
        (None, None) => true,
        _ => false,
    }
}
impl SqliteStore {
    pub(super) fn attach_location(&self, source: &Source) -> Result<()> {
        let old: Option<String> = self
            .registry
            .lock()
            .map_err(lock_error)?
            .query_row(
                "SELECT json FROM source_locations WHERE id=?1",
                [&source.id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?;
        if let Some(json) = &old {
            let saved: Source = serde_json::from_str(json).map_err(Error::io)?;
            if saved.kind != source.kind
                || !same_path(&saved.index_root, &source.index_root)
                || !same_path(&saved.media_root, &source.media_root)
            {
                return Err(Error::new(
                    "SOURCE_LOCATION_CONFLICT",
                    "该数据湖已登记其他位置，请使用重新关联入口调整共享位置",
                ));
            }
            return Ok(());
        }
        let changed = self
            .registry
            .lock()
            .map_err(lock_error)?
            .execute(
                "INSERT OR IGNORE INTO source_locations VALUES (?1,?2)",
                params![source.id, serde_json::to_string(source).map_err(Error::io)?],
            )
            .map_err(db_error)?;
        if changed == 0 {
            return Err(Error::new(
                "SOURCE_LOCATION_CONFLICT",
                "来源位置刚被另一个请求登记，请重试",
            ));
        }
        Ok(())
    }
    pub fn relink_source(&self, pid: &str, source: Source) -> Result<()> {
        let current = self.source(pid, &source.id)?;
        if current.kind != source.kind {
            return Err(Error::new("SOURCE_ID_MISMATCH", "重新关联不能更换来源类型"));
        }
        self.registry
            .lock()
            .map_err(lock_error)?
            .execute(
                "UPDATE source_locations SET json=?2 WHERE id=?1",
                params![
                    source.id,
                    serde_json::to_string(&source).map_err(Error::io)?
                ],
            )
            .map_err(db_error)?;
        // Notify already-open references; do not open or migrate other recent projects.
        for id in self.owned_projects()? {
            let notified = (|| -> Result<()> {
                let p = self.handle(&id)?;
                let mut db = p.db.lock().map_err(lock_error)?;
                let tx = db.transaction().map_err(db_error)?;
                if tx
                    .prepare("SELECT 1 FROM sources WHERE id=?1")
                    .map_err(db_error)?
                    .exists([&source.id])
                    .map_err(db_error)?
                {
                    event(&tx, "source.relinked", &source.id)?;
                }
                tx.commit().map_err(db_error)
            })();
            if let Err(error) = notified
                && error.code != "PROJECT_CLOSED"
            {
                self.registry
                    .lock()
                    .map_err(lock_error)?
                    .execute(
                        "UPDATE projects SET issue=?2 WHERE id=?1",
                        params![id, error.to_string()],
                    )
                    .map_err(db_error)?;
            }
        }
        Ok(())
    }
}
