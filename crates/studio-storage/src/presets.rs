use crate::*;

fn read(db: &Connection, id: &str) -> Result<ToolPreset> {
    let value=db.query_row("SELECT name,notes,revision,run_json,created_at,updated_at FROM tool_presets WHERE id=?1",[id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,unsigned(r,2)?,r.get::<_,String>(3)?,r.get(4)?,r.get(5)?))).optional().map_err(db_error)?.ok_or_else(||Error::new("NOT_FOUND","参数预设不存在或已删除"))?;
    Ok(ToolPreset {
        id: id.into(),
        name: value.0,
        notes: value.1,
        revision: value.2,
        run: serde_json::from_str(&value.3).map_err(Error::io)?,
        created_at: value.4,
        updated_at: value.5,
    })
}
impl SqliteStore {
    pub fn tool_presets(
        &self,
        pid: &str,
        operator: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<PresetPage> {
        if operator.len() > 120 {
            return Err(Error::invalid("算子标识过长"));
        }
        let p = self.handle(pid)?;
        let db = p.db.lock().map_err(lock_error)?;
        let before = after
            .map(|id| -> Result<ToolPreset> {
                validate_id(id)?;
                let old = read(&db, id)?;
                if old.run.operator_id != operator {
                    return Err(Error::invalid("参数预设游标不属于当前工具"));
                }
                Ok(old)
            })
            .transpose()?;
        let limit = limit.clamp(1, 128);
        let mut stmt=db.prepare("SELECT id FROM tool_presets WHERE json_extract(run_json,'$.operator_id')=?1 AND (?2 IS NULL OR (name,id)>(?2,?3)) ORDER BY name,id LIMIT ?4").map_err(db_error)?;
        let mut ids = stmt
            .query_map(
                params![
                    operator,
                    before.as_ref().map(|p| &p.name),
                    before.as_ref().map(|p| &p.id),
                    limit as i64 + 1
                ],
                |r| r.get::<_, String>(0),
            )
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        let more = ids.len() > limit;
        ids.truncate(limit);
        let next_cursor = if more { ids.last().cloned() } else { None };
        Ok(PresetPage {
            items: ids
                .iter()
                .map(|id| read(&db, id))
                .collect::<Result<Vec<_>>>()?,
            next_cursor,
        })
    }
    pub fn save_tool_preset(
        &self,
        pid: &str,
        id: Option<&str>,
        name: &str,
        notes: &str,
        expected: u64,
        run: OperatorRun,
    ) -> Result<ToolPreset> {
        let name = validate_name(name)?;
        if notes.chars().count() > 4000 {
            return Err(Error::invalid("预设备注最多 4000 个字符"));
        }
        let json = serde_json::to_string(&run).map_err(Error::io)?;
        if json.len() > 65536 {
            return Err(Error::invalid("参数预设最多 64 KiB"));
        }
        let p = self.handle(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.transaction().map_err(db_error)?;
        let id = if let Some(id) = id {
            validate_id(id)?;
            let previous = read(&tx, id)?;
            if previous.revision != expected {
                return Err(Error::new("REVISION_CONFLICT", "参数预设已被其他操作修改"));
            }
            if previous.run.operator_id != run.operator_id {
                return Err(Error::invalid("不能用其他工具覆盖参数预设"));
            }
            tx.execute("UPDATE tool_presets SET name=?2,notes=?3,revision=revision+1,run_json=?4,updated_at=?5 WHERE id=?1",params![id,name,notes.trim(),json,now()]).map_err(db_error)?;
            id.to_owned()
        } else {
            if expected != 0 {
                return Err(Error::new(
                    "REVISION_CONFLICT",
                    "新参数预设的初始版本必须为零",
                ));
            }
            let id = new_id();
            tx.execute(
                "INSERT INTO tool_presets VALUES (?1,?2,?3,1,?4,?5,?5)",
                params![id, name, notes.trim(), json, now()],
            )
            .map_err(db_error)?;
            id
        };
        event(&tx, "preset.changed", &id)?;
        let saved = read(&tx, &id)?;
        tx.commit().map_err(db_error)?;
        Ok(saved)
    }
    pub fn delete_tool_preset(&self, pid: &str, id: &str, expected: u64) -> Result<()> {
        validate_id(id)?;
        let p = self.handle(pid)?;
        let mut db = p.db.lock().map_err(lock_error)?;
        let tx = db.transaction().map_err(db_error)?;
        if read(&tx, id)?.revision != expected {
            return Err(Error::new("REVISION_CONFLICT", "参数预设已被其他操作修改"));
        }
        tx.execute("DELETE FROM tool_presets WHERE id=?1", [id])
            .map_err(db_error)?;
        event(&tx, "preset.removed", id)?;
        tx.commit().map_err(db_error)
    }
}
