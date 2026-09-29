//! Bridge the archive-owned relocation journal and the application's read registry.
use serde_json::{Value, json};
use std::sync::{LazyLock, Mutex};
use studio_application::lake_updates::LakeUpdateBackend;
use studio_domain::{Error, Result, lake_updates::LakeUpdateOperation as Op};
use studio_storage::SqliteStore;

pub static COMMAND: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn acknowledge(
    store: &SqliteStore,
    backend: &dyn LakeUpdateBackend,
    value: Value,
) -> Result<Value> {
    if value["phase"] != "writer_committed" {
        return Ok(value);
    }
    let field = |name: &str| -> Result<&str> {
        value[name]
            .as_str()
            .ok_or_else(|| Error::new("UPDATE_PROTOCOL", "迁移记录格式无效"))
    };
    store.relink_location_roots(
        field("lake_id")?,
        field("media_root")?.into(),
        field("index_root")?.into(),
    )?;
    #[cfg(feature = "test-faults")]
    if std::env::var("STUDIO_TEST_RELOCATION_FAULT").as_deref() == Ok("after_reader") {
        return Err(Error::new(
            "UPDATE_UNAVAILABLE",
            "Injected relocation interruption after reader commit",
        ));
    }
    backend.execute(Op::RelocationFinish, json!({"identity": field("id")?}))
}
pub fn apply(
    store: &SqliteStore,
    backend: &dyn LakeUpdateBackend,
    id: &str,
    media: &str,
    index: &str,
) -> Result<Value> {
    let _guard = COMMAND
        .lock()
        .map_err(|_| Error::new("UPDATE_UNAVAILABLE", "迁移控制器不可用"))?;
    let value = backend.execute(
        Op::RelocationApply,
        json!({"identity":id,"media_root":media,"index_root":index}),
    )?;
    acknowledge(store, backend, value)
}
pub fn recover(store: &SqliteStore, backend: &dyn LakeUpdateBackend) -> Result<()> {
    let _guard = COMMAND
        .lock()
        .map_err(|_| Error::new("UPDATE_UNAVAILABLE", "迁移控制器不可用"))?;
    let values = backend.execute(Op::RelocationList, json!({}))?;
    for value in values["items"]
        .as_array()
        .ok_or_else(|| Error::new("UPDATE_PROTOCOL", "迁移列表格式无效"))?
    {
        if value["phase"] == "writer_committed" {
            acknowledge(store, backend, value.clone())?;
        }
    }
    Ok(())
}
