use crate::{SqliteStore, db_error, lock_error};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};
use studio_application::llm::LlmRepository;
use studio_domain::{Error, Result, llm::*, validate_id};
mod catalog;
mod models;

fn read<T: DeserializeOwned>(db: &Connection, table: &str, id: &str) -> Result<T> {
    validate_id(id)?;
    let json: Option<String> = db
        .query_row(
            &format!("SELECT json FROM {table} WHERE id=?1"),
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_error)?;
    serde_json::from_str(&json.ok_or_else(|| Error::new("NOT_FOUND", "LLM 配置不存在"))?)
        .map_err(Error::io)
}
fn list<T: DeserializeOwned>(db: &Connection, table: &str) -> Result<Vec<T>> {
    let mut statement = db
        .prepare(&format!("SELECT json FROM {table} ORDER BY id"))
        .map_err(db_error)?;
    statement
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(db_error)?
        .map(|r| serde_json::from_str(&r.map_err(db_error)?).map_err(Error::io))
        .collect()
}
fn revision(db: &Connection, table: &str, id: &str, expected: u64) -> Result<()> {
    validate_id(id)?;
    if expected >= i64::MAX as u64 {
        return Err(Error::invalid("配置版本超出范围"));
    }
    let actual: Option<u64> = db
        .query_row(
            &format!("SELECT revision FROM {table} WHERE id=?1"),
            [id],
            |r| crate::unsigned(r, 0),
        )
        .optional()
        .map_err(db_error)?;
    if actual.unwrap_or(0) != expected {
        return Err(Error::new("REVISION_CONFLICT", "配置已变化，请重新载入"));
    }
    Ok(())
}
fn save<T: Serialize>(
    db: &Connection,
    table: &str,
    id: &str,
    expected: u64,
    value: &T,
    maximum: u32,
) -> Result<()> {
    revision(db, table, id, expected)?;
    if expected == 0 {
        let count: u32 = db
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .map_err(db_error)?;
        if count >= maximum {
            return Err(Error::invalid("已达到本机配置数量上限"));
        }
    }
    db.execute(&format!("INSERT INTO {table}(id,revision,json) VALUES (?1,?2,?3) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,json=excluded.json"),
        params![id, (expected+1) as i64, serde_json::to_string(value).map_err(Error::io)?]).map_err(db_error)?;
    Ok(())
}
fn remove(db: &Connection, table: &str, id: &str, expected: u64) -> Result<()> {
    if expected == 0 {
        return Err(Error::invalid("删除配置需要当前版本"));
    }
    revision(db, table, id, expected)?;
    db.execute(&format!("DELETE FROM {table} WHERE id=?1"), [id])
        .map_err(db_error)?;
    Ok(())
}
impl LlmRepository for SqliteStore {
    fn providers(&self) -> Result<Vec<LlmProvider>> {
        list(&*self.registry.lock().map_err(lock_error)?, "llm_providers")
    }
    fn provider(&self, id: &str) -> Result<LlmProvider> {
        read(
            &*self.registry.lock().map_err(lock_error)?,
            "llm_providers",
            id,
        )
    }
    fn save_provider(&self, mut value: LlmProvider, expected: u64) -> Result<LlmProvider> {
        value.revision = expected
            .checked_add(1)
            .ok_or_else(|| Error::invalid("版本无效"))?;
        save(
            &*self.registry.lock().map_err(lock_error)?,
            "llm_providers",
            &value.id,
            expected,
            &value,
            128,
        )?;
        Ok(value)
    }
    fn remove_provider(&self, id: &str, expected: u64) -> Result<()> {
        let db = self.registry.lock().map_err(lock_error)?;
        let count: u32 = db
            .query_row(
                "SELECT COUNT(*) FROM llm_models WHERE provider_id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        if count > 0 {
            return Err(Error::new("OBJECT_IN_USE", "请先移除该连接保存的模型配置"));
        }
        remove(&db, "llm_providers", id, expected)
    }
    fn models(&self, provider_id: &str) -> Result<Vec<LlmModel>> {
        models::list(&*self.registry.lock().map_err(lock_error)?, provider_id)
    }
    fn model(&self, id: &str) -> Result<LlmModel> {
        read(
            &*self.registry.lock().map_err(lock_error)?,
            "llm_models",
            id,
        )
    }
    fn save_model(&self, value: LlmModel, expected: u64) -> Result<LlmModel> {
        models::save(&*self.registry.lock().map_err(lock_error)?, value, expected)
    }
    fn remove_model(&self, id: &str, expected: u64) -> Result<()> {
        remove(
            &*self.registry.lock().map_err(lock_error)?,
            "llm_models",
            id,
            expected,
        )
    }
    fn presets(&self) -> Result<Vec<LlmPreset>> {
        list(&*self.registry.lock().map_err(lock_error)?, "llm_presets")
    }
    fn preset(&self, id: &str) -> Result<LlmPreset> {
        read(
            &*self.registry.lock().map_err(lock_error)?,
            "llm_presets",
            id,
        )
    }
    fn save_preset(&self, mut value: LlmPreset, expected: u64) -> Result<LlmPreset> {
        value.revision = expected
            .checked_add(1)
            .ok_or_else(|| Error::invalid("版本无效"))?;
        save(
            &*self.registry.lock().map_err(lock_error)?,
            "llm_presets",
            &value.id,
            expected,
            &value,
            256,
        )?;
        Ok(value)
    }
    fn remove_preset(&self, id: &str, expected: u64) -> Result<()> {
        remove(
            &*self.registry.lock().map_err(lock_error)?,
            "llm_presets",
            id,
            expected,
        )
    }
    fn catalog(&self, provider_id: &str) -> Result<Option<LlmCatalog>> {
        catalog::read(&*self.registry.lock().map_err(lock_error)?, provider_id)
    }
    fn save_catalog(&self, value: &LlmCatalog) -> Result<()> {
        catalog::save(&*self.registry.lock().map_err(lock_error)?, value)
    }
}
