use super::*;
pub(super) fn read(db: &Connection, provider_id: &str) -> Result<Option<LlmCatalog>> {
    validate_id(provider_id)?;
    let json: Option<String> = db
        .query_row(
            "SELECT json FROM llm_catalogs WHERE provider_id=?1",
            [provider_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_error)?;
    json.map(|j| serde_json::from_str(&j).map_err(Error::io))
        .transpose()
}
pub(super) fn save(db: &Connection, value: &LlmCatalog) -> Result<()> {
    if value.models.len() > 10000 {
        return Err(Error::invalid("模型目录最多 10000 项"));
    }
    revision(
        db,
        "llm_providers",
        &value.provider_id,
        value.provider_revision,
    )?;
    db.execute("INSERT INTO llm_catalogs(provider_id,json) VALUES (?1,?2) ON CONFLICT(provider_id) DO UPDATE SET json=excluded.json", params![value.provider_id, serde_json::to_string(value).map_err(Error::io)?]).map_err(db_error)?;
    Ok(())
}
