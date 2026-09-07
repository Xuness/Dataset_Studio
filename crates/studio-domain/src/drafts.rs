use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Draft {
    pub project_id: String,
    pub module_id: String,
    pub instance_id: String,
    pub schema_version: u32,
    pub revision: u64,
    pub updated_at: String,
    pub value: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveDraft {
    pub schema_version: u32,
    pub expected_revision: u64,
    pub value: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preference {
    pub key: String,
    pub schema_version: u32,
    pub revision: u64,
    pub value: Value,
}
