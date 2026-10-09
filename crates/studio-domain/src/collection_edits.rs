use crate::{AssetKey, Collection, ScopeRef};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CollectionMemberInput {
    Scope { scope: ScopeRef },
    Keys { keys: Vec<AssetKey> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CollectionChange {
    Add { input: CollectionMemberInput },
    Remove { input: CollectionMemberInput },
    Restore { revision: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CollectionEdit {
    pub request_id: String,
    pub expected_revision: u64,
    pub change: CollectionChange,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionEditResult {
    pub collection: Collection,
    pub previous_revision: u64,
    pub requested: u64,
    pub changed: u64,
}
