use crate::*;

impl SqliteStore {
    pub fn browse_scope_count(&self, pid: &str, scope: &ScopeRef) -> Result<u64> {
        let p = self.handle(pid)?;
        let db = p.db.lock().map_err(lock_error)?;
        Ok(scopes::resolve(&db, pid, scope)?.count)
    }

    pub fn browse_scope_keys(
        &self,
        pid: &str,
        scope: &ScopeRef,
        after: Option<&AssetKey>,
        limit: usize,
        descending: bool,
    ) -> Result<Vec<AssetKey>> {
        let p = self.handle(pid)?;
        let db = p.db.lock().map_err(lock_error)?;
        let resolved = scopes::resolve(&db, pid, scope)?;
        if matches!(scope.target, ScopeTarget::Selection { .. }) {
            return selection::keys_ordered(&db, after, limit, descending);
        }
        let end = if descending { "\u{10ffff}" } else { "" };
        let (source, asset) = after
            .map(|k| (k.source_id.as_str(), k.asset_id.as_str()))
            .unwrap_or((end, end));
        let op = if descending { "<" } else { ">" };
        let direction = if descending { "DESC" } else { "ASC" };
        let mut stmt = db.prepare(&format!("SELECT source_id,asset_id FROM ({}) WHERE (source_id,asset_id){op}(?1,?2) ORDER BY source_id {direction},asset_id {direction} LIMIT ?3", resolved.sql)).map_err(db_error)?;
        stmt.query_map(params![source, asset, limit.clamp(1, 4097) as u32], |row| {
            Ok(AssetKey {
                source_id: row.get(0)?,
                asset_id: row.get(1)?,
            })
        })
        .map_err(db_error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_error)
    }

    pub fn filter_browse_scope(
        &self,
        pid: &str,
        scope: &ScopeRef,
        keys: &[AssetKey],
    ) -> Result<Vec<bool>> {
        if keys.len() > 512 {
            return Err(Error::invalid("范围检查批次超过 512 项"));
        }
        let p = self.handle(pid)?;
        let db = p.db.lock().map_err(lock_error)?;
        let resolved = scopes::resolve(&db, pid, scope)?;
        if matches!(scope.target, ScopeTarget::Selection { .. }) {
            return keys
                .iter()
                .map(|key| selection::contains(&db, key))
                .collect();
        }
        let mut stmt = db
            .prepare(&format!(
                "SELECT 1 FROM ({}) WHERE source_id=?1 AND asset_id=?2 LIMIT 1",
                resolved.sql
            ))
            .map_err(db_error)?;
        keys.iter()
            .map(|key| {
                stmt.exists(params![key.source_id, key.asset_id])
                    .map_err(db_error)
            })
            .collect()
    }
}
