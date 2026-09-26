use crate::*;

impl SqliteStore {
    pub fn selection_members(&self, pid: &str, keys: &[AssetKey]) -> Result<(u64, Vec<bool>)> {
        if keys.len() > 128 {
            return Err(Error::invalid("选择检查最多 128 项"));
        }
        let project = self.handle(pid)?;
        let db = project.read()?;
        let revision = read(&db)?.revision;
        let selected = keys
            .iter()
            .map(|key| contains(&db, key))
            .collect::<Result<Vec<_>>>()?;
        Ok((revision, selected))
    }
}

// Explicit additions are disjoint from the base. Exclusions are a subset of it.
// UNION also protects membership if a manually repaired project violates that invariant.
pub(super) const MEMBERS: &str = "SELECT source_id,asset_id FROM selection UNION SELECT m.source_id,m.asset_id FROM result_members m WHERE m.result_id=(SELECT result_id FROM selection_base WHERE singleton=1) AND NOT EXISTS(SELECT 1 FROM selection_exclusions e WHERE e.source_id=m.source_id AND e.asset_id=m.asset_id)";

pub(super) fn keys(
    db: &Connection,
    after: Option<&AssetKey>,
    limit: usize,
) -> Result<Vec<AssetKey>> {
    keys_ordered(db, after, limit, false)
}

pub(super) fn keys_ordered(
    db: &Connection,
    after: Option<&AssetKey>,
    limit: usize,
    descending: bool,
) -> Result<Vec<AssetKey>> {
    let end = if descending { "\u{10ffff}" } else { "" };
    let (source, asset) = after
        .map(|key| (key.source_id.as_str(), key.asset_id.as_str()))
        .unwrap_or((end, end));
    // ORDER BY belongs to the compound query so SQLite can merge two ordered
    // index ranges. Wrapping MEMBERS would sort all remaining members per page.
    let op = if descending { "<" } else { ">" };
    let direction = if descending { "DESC" } else { "ASC" };
    let mut stmt = db.prepare(&format!(
        "SELECT source_id,asset_id FROM selection WHERE (source_id,asset_id){op}(?1,?2)
         UNION
         SELECT m.source_id,m.asset_id FROM result_members m
         WHERE m.result_id=(SELECT result_id FROM selection_base WHERE singleton=1)
         AND (m.source_id,m.asset_id){op}(?1,?2)
         AND NOT EXISTS(SELECT 1 FROM selection_exclusions e WHERE e.source_id=m.source_id AND e.asset_id=m.asset_id)
         ORDER BY source_id {direction},asset_id {direction} LIMIT ?3",
    )).map_err(db_error)?;
    stmt.query_map(params![source, asset, limit.clamp(1, 4097) as u32], |r| {
        Ok(AssetKey {
            source_id: r.get(0)?,
            asset_id: r.get(1)?,
        })
    })
    .map_err(db_error)?
    .collect::<std::result::Result<Vec<_>, _>>()
    .map_err(db_error)
}

pub(super) fn read(db: &Connection) -> Result<Selection> {
    db.query_row("SELECT (SELECT CAST(value AS INTEGER) FROM meta WHERE key='selection_revision'),(SELECT CAST(value AS INTEGER) FROM meta WHERE key='selection_count'),(SELECT result_id FROM selection_base),(SELECT COUNT(*) FROM selection_exclusions)",[],|r|Ok(Selection{revision:unsigned(r,0)?,count:unsigned(r,1)?,base_result:r.get(2)?,excluded_count:unsigned(r,3)?})).map_err(db_error)
}
pub(super) fn check_revision(db: &Connection, expected: u64) -> Result<()> {
    if read(db)?.revision != expected {
        return Err(Error::new("REVISION_CONFLICT", "选择已被其他操作修改"));
    }
    Ok(())
}
pub(super) fn clear(db: &Connection) -> Result<()> {
    db.execute_batch("DELETE FROM selection; DELETE FROM selection_base; DELETE FROM selection_exclusions; DELETE FROM result_references WHERE owner_kind='selection'; DELETE FROM artifact_references WHERE owner_kind='selection';").map_err(db_error)
}
pub(super) fn publish(db: &Connection) -> Result<Selection> {
    let count:u64=db.query_row("SELECT (SELECT COUNT(*) FROM selection)+COALESCE((SELECT r.count FROM selection_base b JOIN query_results r ON r.id=b.result_id),0)-(SELECT COUNT(*) FROM selection_exclusions)",[],|r|unsigned(r,0)).map_err(db_error)?;
    db.execute(
        "UPDATE meta SET value=?1 WHERE key='selection_count'",
        [count.to_string()],
    )
    .map_err(db_error)?;
    db.execute(
        "UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='selection_revision'",
        [],
    )
    .map_err(db_error)?;
    event(db, "selection.changed", "selection")?;
    read(db)
}
pub(super) fn contains(db: &Connection, key: &AssetKey) -> Result<bool> {
    db.query_row("SELECT EXISTS(SELECT 1 FROM selection WHERE source_id=?1 AND asset_id=?2) OR (EXISTS(SELECT 1 FROM result_members m WHERE m.result_id=(SELECT result_id FROM selection_base WHERE singleton=1) AND m.source_id=?1 AND m.asset_id=?2) AND NOT EXISTS(SELECT 1 FROM selection_exclusions WHERE source_id=?1 AND asset_id=?2))",params![key.source_id,key.asset_id],|r|r.get(0)).map_err(db_error)
}
pub(super) fn change(
    store: &SqliteStore,
    pid: &str,
    expected: u64,
    add: &[AssetKey],
    remove: &[AssetKey],
    reset: bool,
) -> Result<Selection> {
    if add.len() + remove.len() > 1000 {
        return Err(Error::invalid("每次选择修改最多提交 1000 项"));
    }
    let history_limit = store.editing_settings()?.undo_limit;
    let p = store.handle(pid)?;
    let mut db = p.db.lock().map_err(lock_error)?;
    let tx = db.project_transaction().map_err(db_error)?;
    check_revision(&tx, expected)?;
    let history_id = history::begin(
        &tx,
        history_limit,
        if reset {
            "清空选择"
        } else {
            "修改图片选择"
        },
    )?;
    if reset {
        clear(&tx)?;
    }
    for key in add {
        tx.execute(
            "DELETE FROM selection_exclusions WHERE source_id=?1 AND asset_id=?2",
            params![key.source_id, key.asset_id],
        )
        .map_err(db_error)?;
        tx.execute("INSERT OR IGNORE INTO selection SELECT ?1,?2 WHERE NOT EXISTS(SELECT 1 FROM result_members m WHERE m.result_id=(SELECT result_id FROM selection_base WHERE singleton=1) AND m.source_id=?1 AND m.asset_id=?2)",params![key.source_id,key.asset_id]).map_err(db_error)?;
    }
    for key in remove {
        tx.execute(
            "DELETE FROM selection WHERE source_id=?1 AND asset_id=?2",
            params![key.source_id, key.asset_id],
        )
        .map_err(db_error)?;
        tx.execute("INSERT OR IGNORE INTO selection_exclusions SELECT ?1,?2 WHERE EXISTS(SELECT 1 FROM result_members m WHERE m.result_id=(SELECT result_id FROM selection_base WHERE singleton=1) AND m.source_id=?1 AND m.asset_id=?2)",params![key.source_id,key.asset_id]).map_err(db_error)?;
    }
    let selection = publish(&tx)?;
    history::finish(&tx, history_id, history_limit)?;
    tx.commit().map_err(db_error)?;
    Ok(selection)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn sparse_selection_pages_do_not_sort_all_remaining_members() {
        let mut db = Connection::open_in_memory().unwrap();
        migrations::initialize(&mut db).unwrap();
        db.execute_batch(
            "INSERT INTO sources VALUES ('source','{}');
             INSERT INTO query_results(id,spec_json,versions_json,status,count,created_at)
             VALUES ('result','{}','[]','ready',100000,'1');
             WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<100000)
             INSERT INTO result_members SELECT 'result','source',printf('%08d',x) FROM n;
             INSERT INTO selection_base VALUES (1,'result');
             INSERT INTO selection_exclusions VALUES ('source','00050010');
             INSERT INTO selection VALUES ('source','00050050'),('source','00100001');",
        )
        .unwrap();
        let ticks = Arc::new(AtomicUsize::new(0));
        let observed = ticks.clone();
        // Bound VM work, not elapsed time. A full-tail sort exceeds this budget
        // by orders of magnitude even when every page and source is cached.
        db.progress_handler(
            100,
            Some(move || observed.fetch_add(1, Ordering::Relaxed) > 100),
        )
        .unwrap();
        let first = keys(
            &db,
            Some(&AssetKey {
                source_id: "source".into(),
                asset_id: "00050000".into(),
            }),
            96,
        )
        .unwrap();
        db.progress_handler(0, None::<fn() -> bool>).unwrap();
        let expected = (50001..=50097)
            .filter(|id| *id != 50010)
            .map(|id| AssetKey {
                source_id: "source".into(),
                asset_id: format!("{id:08}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(first, expected);
        assert!(ticks.load(Ordering::Relaxed) <= 100);
        let tail = keys(
            &db,
            Some(&AssetKey {
                source_id: "source".into(),
                asset_id: "00099999".into(),
            }),
            96,
        )
        .unwrap();
        assert_eq!(
            tail.iter()
                .map(|key| key.asset_id.as_str())
                .collect::<Vec<_>>(),
            vec!["00100000", "00100001"]
        );
    }
}
