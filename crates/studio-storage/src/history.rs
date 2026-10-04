//! Transactional selection deltas. A query selection retains its base reference;
//! recording a point edit never copies or enumerates that base's members.
use crate::*;
use studio_application::DraftRepository;

const SETTINGS_KEY: &str = "core.editing";

fn discard(db: &Connection, condition: &str, limit: u32) -> Result<()> {
    // condition is an internal SQL fragment, never supplied by a client.
    for table in ["result_references", "artifact_references"] {
        db.execute(&format!("DELETE FROM {table} WHERE owner_kind='selection_history' AND owner_id IN (SELECT CAST(id AS TEXT) FROM selection_history WHERE {condition})"), [limit]).map_err(db_error)?;
    }
    db.execute(
        &format!("DELETE FROM selection_history WHERE {condition}"),
        [limit],
    )
    .map_err(db_error)?;
    Ok(())
}
pub(super) fn prune(db: &Connection, limit: u32) -> Result<()> {
    discard(
        db,
        "id NOT IN (SELECT id FROM selection_history ORDER BY applied DESC,CASE WHEN applied=1 THEN -id ELSE id END LIMIT ?1)",
        limit,
    )?;
    crate::cache_cleanup::queue_unreferenced_inputs(db)
}
fn save_refs(db: &Connection, id: i64, phase: i32) -> Result<()> {
    for (kind, table, column) in [
        ("result", "result_references", "result_id"),
        ("artifact", "artifact_references", "artifact_id"),
    ] {
        db.execute(&format!("INSERT OR IGNORE INTO selection_history_refs SELECT ?1,?2,?3,{column} FROM {table} WHERE owner_kind='selection' AND owner_id='selection'"), params![id,phase,kind]).map_err(db_error)?;
    }
    db.execute("INSERT OR IGNORE INTO selection_history_refs SELECT ?1,?2,'result',result_id FROM selection_base WHERE singleton=1",params![id,phase]).map_err(db_error)?;
    db.execute("INSERT OR IGNORE INTO result_references SELECT 'selection_history',CAST(step_id AS TEXT),target_id FROM selection_history_refs WHERE step_id=?1 AND kind='result'",[id]).map_err(db_error)?;
    db.execute("INSERT OR IGNORE INTO artifact_references SELECT 'selection_history',CAST(step_id AS TEXT),target_id FROM selection_history_refs WHERE step_id=?1 AND kind='artifact'",[id]).map_err(db_error)?;
    Ok(())
}
pub(super) fn begin(db: &Connection, limit: u32, label: &str) -> Result<Option<i64>> {
    discard(db, "applied=0 AND ?1>=0", limit)?;
    prune(db, limit)?;
    if limit == 0 {
        return Ok(None);
    }
    let selection = selection::read(db)?;
    db.execute("INSERT INTO selection_history(label,created_at,before_base,before_count,after_count) VALUES (?1,?2,?3,?4,?4)",params![label,now(),selection.base_result,selection.count as i64]).map_err(db_error)?;
    let id = db.last_insert_rowid();
    save_refs(db, id, 0)?;
    db.execute(
        "UPDATE meta SET value=?1 WHERE key='selection_history_current'",
        [id.to_string()],
    )
    .map_err(db_error)?;
    Ok(Some(id))
}
pub(super) fn finish(db: &Connection, id: Option<i64>, limit: u32) -> Result<()> {
    db.execute(
        "UPDATE meta SET value='0' WHERE key='selection_history_current'",
        [],
    )
    .map_err(db_error)?;
    if let Some(id) = id {
        let current = selection::read(db)?;
        db.execute(
            "UPDATE selection_history SET after_base=?2,after_count=?3 WHERE id=?1",
            params![id, current.base_result, current.count as i64],
        )
        .map_err(db_error)?;
        db.execute("DELETE FROM selection_history_changes WHERE step_id=?1 AND before_present=after_present",[id]).map_err(db_error)?;
        save_refs(db, id, 1)?;
        let unchanged: bool = db.query_row("SELECT before_base IS after_base AND before_count=after_count AND NOT EXISTS(SELECT 1 FROM selection_history_changes WHERE step_id=?1) AND NOT EXISTS(SELECT kind,target_id FROM selection_history_refs WHERE step_id=?1 AND phase=0 EXCEPT SELECT kind,target_id FROM selection_history_refs WHERE step_id=?1 AND phase=1) AND NOT EXISTS(SELECT kind,target_id FROM selection_history_refs WHERE step_id=?1 AND phase=1 EXCEPT SELECT kind,target_id FROM selection_history_refs WHERE step_id=?1 AND phase=0) FROM selection_history WHERE id=?1",[id],|r|r.get(0)).map_err(db_error)?;
        if unchanged {
            for table in ["result_references", "artifact_references"] {
                db.execute(
                    &format!(
                        "DELETE FROM {table} WHERE owner_kind='selection_history' AND owner_id=?1"
                    ),
                    [id.to_string()],
                )
                .map_err(db_error)?;
            }
            db.execute("DELETE FROM selection_history WHERE id=?1", [id])
                .map_err(db_error)?;
        }
    }
    prune(db, limit)
}
fn status(db: &Connection, limit: u32) -> Result<HistoryStatus> {
    let (undo_steps,redo_steps,undo_label,redo_label) = db.query_row("SELECT (SELECT COUNT(*) FROM selection_history WHERE applied=1),(SELECT COUNT(*) FROM selection_history WHERE applied=0),(SELECT label FROM selection_history WHERE applied=1 ORDER BY id DESC LIMIT 1),(SELECT label FROM selection_history WHERE applied=0 ORDER BY id LIMIT 1)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(db_error)?;
    Ok(HistoryStatus {
        selection: selection::read(db)?,
        undo_steps,
        redo_steps,
        undo_label,
        redo_label,
        limit,
    })
}
impl SqliteStore {
    pub fn editing_settings(&self) -> Result<EditingSettings> {
        let value = self.preference(SETTINGS_KEY)?;
        match value {
            None => Ok(EditingSettings {
                undo_limit: 50,
                revision: 0,
            }),
            Some(p) => {
                if p.schema_version != 1 {
                    return Err(Error::new(
                        "PREFERENCE_VERSION_UNSUPPORTED",
                        "选择撤销设置版本不兼容",
                    ));
                }
                let undo_limit = p
                    .value
                    .get("undo_limit")
                    .and_then(|v| v.as_u64())
                    .filter(|v| *v <= 200)
                    .ok_or_else(|| Error::invalid("撤销步数必须是 0–200 的整数"))?
                    as u32;
                Ok(EditingSettings {
                    undo_limit,
                    revision: p.revision,
                })
            }
        }
    }
    pub fn configure_editing(&self, undo_limit: u32, revision: u64) -> Result<EditingSettings> {
        if undo_limit > 200 {
            return Err(Error::invalid("撤销步数必须是 0–200 的整数"));
        }
        let saved = self.save_preference(
            SETTINGS_KEY,
            SaveDraft {
                schema_version: 1,
                expected_revision: revision,
                value: serde_json::json!({"undo_limit":undo_limit}),
            },
        )?;
        for pid in self.owned_projects()? {
            let p = self.handle(&pid)?;
            let mut db = p.db.lock().map_err(lock_error)?;
            let tx = db.project_transaction().map_err(db_error)?;
            prune(&tx, undo_limit)?;
            event(&tx, "selection.history.changed", "selection")?;
            tx.commit().map_err(db_error)?;
        }
        Ok(EditingSettings {
            undo_limit,
            revision: saved.revision,
        })
    }
    pub fn selection_history(&self, pid: &str) -> Result<HistoryStatus> {
        let limit = self.editing_settings()?.undo_limit;
        let p = self.handle(pid)?;
        status(&*p.db.lock().map_err(lock_error)?, limit)
    }
    pub fn clear_selection_history(&self, pid: &str, expected: u64) -> Result<HistoryStatus> {
        let limit = self.editing_settings()?.undo_limit;
        let p = self.handle(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.project_transaction().map_err(db_error)?;
        selection::check_revision(&tx, expected)?;
        prune(&tx, 0)?;
        event(&tx, "selection.history.changed", "selection")?;
        let result = status(&tx, limit)?;
        tx.commit().map_err(db_error)?;
        Ok(result)
    }
    pub fn restore_selection(&self, pid: &str, expected: u64, redo: bool) -> Result<HistoryStatus> {
        let limit = self.editing_settings()?.undo_limit;
        let p = self.handle(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.project_transaction().map_err(db_error)?;
        selection::check_revision(&tx, expected)?;
        let sql = if redo {
            "SELECT id,after_base,after_count FROM selection_history WHERE applied=0 ORDER BY id LIMIT 1"
        } else {
            "SELECT id,before_base,before_count FROM selection_history WHERE applied=1 ORDER BY id DESC LIMIT 1"
        };
        let step: Option<(i64, Option<String>, u64)> = tx
            .query_row(sql, [], |r| Ok((r.get(0)?, r.get(1)?, unsigned(r, 2)?)))
            .optional()
            .map_err(db_error)?;
        let (id, base, count) = step.ok_or_else(|| {
            Error::new(
                "HISTORY_EMPTY",
                if redo {
                    "没有可重做的选择操作"
                } else {
                    "没有可撤销的选择操作"
                },
            )
        })?;
        // Triggers are idle throughout restoration; only this transaction can mutate selection.
        tx.execute(
            "UPDATE meta SET value='0' WHERE key='selection_history_current'",
            [],
        )
        .map_err(db_error)?;
        let side = if redo {
            "after_present"
        } else {
            "before_present"
        };
        for (table, bucket) in [
            ("selection", "selection"),
            ("selection_exclusions", "exclusion"),
        ] {
            tx.execute(&format!("DELETE FROM {table} WHERE (source_id,asset_id) IN (SELECT source_id,asset_id FROM selection_history_changes WHERE step_id=?1 AND bucket=?2 AND {side}=0)"),params![id,bucket]).map_err(db_error)?;
            tx.execute(&format!("INSERT OR IGNORE INTO {table} SELECT source_id,asset_id FROM selection_history_changes WHERE step_id=?1 AND bucket=?2 AND {side}=1"),params![id,bucket]).map_err(db_error)?;
        }
        tx.execute("DELETE FROM selection_base", [])
            .map_err(db_error)?;
        if let Some(base) = base {
            tx.execute("INSERT INTO selection_base VALUES (1,?1)", [base])
                .map_err(db_error)?;
        }
        for (kind, table) in [
            ("result", "result_references"),
            ("artifact", "artifact_references"),
        ] {
            tx.execute(
                &format!(
                    "DELETE FROM {table} WHERE owner_kind='selection' AND owner_id='selection'"
                ),
                [],
            )
            .map_err(db_error)?;
            tx.execute(&format!("INSERT INTO {table} SELECT 'selection','selection',target_id FROM selection_history_refs WHERE step_id=?1 AND phase=?2 AND kind=?3"),params![id,i32::from(redo),kind]).map_err(db_error)?;
        }
        tx.execute(
            "UPDATE selection_history SET applied=?2 WHERE id=?1",
            params![id, i32::from(redo)],
        )
        .map_err(db_error)?;
        let selected = selection::publish(&tx)?;
        if selected.count != count {
            return Err(Error::new(
                "HISTORY_INVALID",
                "选择历史与成员不一致，未修改当前选择",
            ));
        }
        let result = status(&tx, limit)?;
        tx.commit().map_err(db_error)?;
        Ok(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn point_history_on_a_large_reference_has_bounded_work_and_one_delta() {
        let mut db = Connection::open_in_memory().unwrap();
        migrations::initialize(&mut db).unwrap();
        db.execute_batch("INSERT INTO sources VALUES ('source','{}');
            INSERT INTO query_results(id,spec_json,versions_json,status,count,created_at) VALUES ('result','{}','[]','ready',100000,'1');
            WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<100000)
            INSERT INTO result_members SELECT 'result','source',printf('%08d',x) FROM n;
            INSERT INTO selection_base VALUES (1,'result');
            INSERT INTO result_references VALUES ('selection','selection','result');
            UPDATE meta SET value='100000' WHERE key='selection_count';").unwrap();
        let ticks = Arc::new(AtomicUsize::new(0));
        let observed = ticks.clone();
        db.progress_handler(
            100,
            Some(move || observed.fetch_add(1, Ordering::Relaxed) > 400),
        )
        .unwrap();
        let tx = db.project_transaction().unwrap();
        let id = begin(&tx, 50, "排除一张图片").unwrap();
        tx.execute(
            "INSERT INTO selection_exclusions VALUES ('source','00050000')",
            [],
        )
        .unwrap();
        assert_eq!(selection::publish(&tx).unwrap().count, 99999);
        finish(&tx, id, 50).unwrap();
        tx.commit().unwrap();
        db.progress_handler(0, None::<fn() -> bool>).unwrap();
        assert!(ticks.load(Ordering::Relaxed) <= 400);
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM selection_history_changes", [], |r| {
                r.get::<_, u32>(0)
            })
            .unwrap(),
            1
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM result_members WHERE result_id='result'",
                [],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
            100000
        );
    }
}
