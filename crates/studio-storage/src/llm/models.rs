use super::*;
pub(super) fn list(db: &Connection, provider_id: &str) -> Result<Vec<LlmModel>> {
    let _: LlmProvider = read(db, "llm_providers", provider_id)?;
    let mut statement = db
        .prepare("SELECT json FROM llm_models WHERE provider_id=?1 ORDER BY id")
        .map_err(db_error)?;
    statement
        .query_map([provider_id], |r| r.get::<_, String>(0))
        .map_err(db_error)?
        .map(|r| serde_json::from_str(&r.map_err(db_error)?).map_err(Error::io))
        .collect()
}
pub(super) fn save(db: &Connection, mut value: LlmModel, expected: u64) -> Result<LlmModel> {
    revision(db, "llm_models", &value.id, expected)?;
    let _: LlmProvider = read(db, "llm_providers", &value.provider_id)?;
    if expected == 0 {
        let count: u32 = db
            .query_row(
                "SELECT COUNT(*) FROM llm_models WHERE provider_id=?1",
                [&value.provider_id],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        if count >= 1024 {
            return Err(Error::invalid("每条连接最多保存 1024 个模型配置"));
        }
    } else {
        let previous: LlmModel = read(db, "llm_models", &value.id)?;
        if previous.provider_id != value.provider_id {
            return Err(Error::invalid("不能将已有模型配置移到另一条连接"));
        }
    }
    value.revision = expected + 1;
    let protocol = serde_json::to_string(&value.config.protocol).map_err(Error::io)?;
    let duplicate: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM llm_models WHERE provider_id=?1 AND remote_model_id=?2 AND protocol=?3 AND id<>?4)", params![value.provider_id, value.config.remote_model_id, protocol, value.id], |r| r.get(0)).map_err(db_error)?;
    if duplicate {
        return Err(Error::invalid("该连接已经保存同一模型与协议的配置"));
    }
    db.execute("INSERT INTO llm_models(id,provider_id,remote_model_id,protocol,revision,json) VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO UPDATE SET remote_model_id=excluded.remote_model_id,protocol=excluded.protocol,revision=excluded.revision,json=excluded.json", params![value.id, value.provider_id, value.config.remote_model_id, protocol, value.revision as i64, serde_json::to_string(&value).map_err(Error::io)?]).map_err(db_error)?;
    Ok(value)
}
