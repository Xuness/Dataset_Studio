use crate::*;
use studio_application::DraftRepository;

fn key(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 120
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err(Error::invalid("草稿及偏好标识无效"));
    }
    Ok(())
}
fn payload(request: &SaveDraft) -> Result<String> {
    if request.schema_version == 0
        || request.schema_version > 65535
        || request.expected_revision >= i64::MAX as u64
    {
        return Err(Error::invalid("草稿版本无效"));
    }
    let json = serde_json::to_string(&request.value).map_err(Error::io)?;
    if json.len() > 65536 {
        return Err(Error::invalid("单份草稿或偏好最多 64 KiB"));
    }
    Ok(json)
}
fn read_draft(db: &Connection, pid: &str, module: &str, instance: &str) -> Result<Option<Draft>> {
    let row = db.query_row("SELECT schema_version,revision,updated_at,value_json FROM tool_drafts WHERE module_id=?1 AND instance_id=?2",params![module,instance], |r| Ok((r.get::<_,u32>(0)?,unsigned(r,1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?))).optional().map_err(db_error)?;
    if let Some((schema_version, revision, updated_at, json)) = row {
        return Ok(Some(Draft {
            project_id: pid.into(),
            module_id: module.into(),
            instance_id: instance.into(),
            schema_version,
            revision,
            updated_at,
            value: serde_json::from_str(&json).map_err(Error::io)?,
        }));
    }
    if instance == "default" {
        let old: Option<String> = db
            .query_row("SELECT json FROM drafts WHERE tool_id=?1", [module], |r| {
                r.get(0)
            })
            .optional()
            .map_err(db_error)?;
        if let Some(old) = old {
            return Ok(Some(Draft {
                project_id: pid.into(),
                module_id: module.into(),
                instance_id: instance.into(),
                schema_version: 0,
                revision: 0,
                updated_at: "unknown".into(),
                value: serde_json::from_str(&old).map_err(Error::io)?,
            }));
        }
    }
    Ok(None)
}
impl DraftRepository for SqliteStore {
    fn draft(&self, pid: &str, module: &str, instance: &str) -> Result<Option<Draft>> {
        key(module)?;
        key(instance)?;
        let p = self.handle(pid)?;
        let started = std::time::Instant::now();
        let db = p.read()?;
        let outcome = read_draft(&db, pid, module, instance);
        tracing::debug!(target: "studio_storage::drafts", project_id = pid,
            elapsed_us = started.elapsed().as_micros() as u64,
            success = outcome.is_ok(), "draft snapshot read");
        outcome
    }
    fn save_draft(
        &self,
        pid: &str,
        module: &str,
        instance: &str,
        request: SaveDraft,
    ) -> Result<Draft> {
        key(module)?;
        key(instance)?;
        let json = payload(&request)?;
        let p = self.handle(pid)?;
        let waiting = std::time::Instant::now();
        let mut db = p.db.lock().map_err(lock_error)?;
        let wait_us = waiting.elapsed().as_micros() as u64;
        let writing = std::time::Instant::now();
        let tx = db.project_transaction().map_err(db_error)?;
        let old = read_draft(&tx, pid, module, instance)?;
        if old.as_ref().map_or(0, |d| d.revision) != request.expected_revision {
            return Err(Error::new("REVISION_CONFLICT", "草稿已被另一次编辑修改"));
        }
        if old
            .as_ref()
            .is_some_and(|d| d.schema_version > request.schema_version)
        {
            return Err(Error::new(
                "DRAFT_VERSION_UNSUPPORTED",
                "不能用较旧格式覆盖已有草稿",
            ));
        }
        tx.execute("INSERT INTO tool_drafts VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(module_id,instance_id) DO UPDATE SET schema_version=excluded.schema_version,revision=excluded.revision,updated_at=excluded.updated_at,value_json=excluded.value_json",params![module,instance,request.schema_version,request.expected_revision as i64+1,now(),json]).map_err(db_error)?;
        event(&tx, "draft.changed", &format!("{module}/{instance}"))?;
        let draft = read_draft(&tx, pid, module, instance)?
            .ok_or_else(|| Error::new("DATABASE_ERROR", "草稿保存后不可读"))?;
        tx.commit().map_err(db_error)?;
        let transaction_us = writing.elapsed().as_micros() as u64;
        drop(db);
        tracing::debug!(target: "studio_storage::drafts", project_id = pid,
            wait_us, transaction_us, "draft committed");
        Ok(draft)
    }
    fn preference(&self, name: &str) -> Result<Option<Preference>> {
        key(name)?;
        read_preference(&*self.registry.lock().map_err(lock_error)?, name)
    }
    fn save_preference(&self, name: &str, request: SaveDraft) -> Result<Preference> {
        key(name)?;
        let json = payload(&request)?;
        let mut db = self.registry.lock().map_err(lock_error)?;
        let tx = db.project_transaction().map_err(db_error)?;
        let old = read_preference(&tx, name)?;
        if old.as_ref().map_or(0, |p| p.revision) != request.expected_revision {
            return Err(Error::new("REVISION_CONFLICT", "偏好已被其他窗口修改"));
        }
        if old
            .as_ref()
            .is_some_and(|p| p.schema_version > request.schema_version)
        {
            return Err(Error::new(
                "PREFERENCE_VERSION_UNSUPPORTED",
                "不能用较旧格式覆盖偏好",
            ));
        }
        tx.execute("INSERT INTO preferences VALUES (?1,?2,?3,?4) ON CONFLICT(key) DO UPDATE SET schema_version=excluded.schema_version,revision=excluded.revision,value_json=excluded.value_json",params![name,request.schema_version,request.expected_revision as i64+1,json]).map_err(db_error)?;
        let preference = read_preference(&tx, name)?
            .ok_or_else(|| Error::new("DATABASE_ERROR", "偏好保存后不可读"))?;
        tx.commit().map_err(db_error)?;
        Ok(preference)
    }
}
fn read_preference(db: &Connection, name: &str) -> Result<Option<Preference>> {
    let row = db
        .query_row(
            "SELECT schema_version,revision,value_json FROM preferences WHERE key=?1",
            [name],
            |r| Ok((r.get::<_, u32>(0)?, unsigned(r, 1)?, r.get::<_, String>(2)?)),
        )
        .optional()
        .map_err(db_error)?;
    row.map(|(schema_version, revision, json)| {
        Ok(Preference {
            key: name.into(),
            schema_version,
            revision,
            value: serde_json::from_str(&json).map_err(Error::io)?,
        })
    })
    .transpose()
}
