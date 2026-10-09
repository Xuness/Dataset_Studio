use crate::{AssetKey, Collection, ScopeRef};
use serde::{Deserialize, Serialize};
use studio_domain as domain;
use utoipa::ToSchema;

#[derive(Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CollectionMemberInput {
    Scope { scope: ScopeRef },
    Keys { keys: Vec<AssetKey> },
}
impl From<CollectionMemberInput> for domain::CollectionMemberInput {
    fn from(input: CollectionMemberInput) -> Self {
        match input {
            CollectionMemberInput::Scope { scope } => Self::Scope {
                scope: scope.into(),
            },
            CollectionMemberInput::Keys { keys } => Self::Keys {
                keys: keys.into_iter().map(Into::into).collect(),
            },
        }
    }
}
#[derive(Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CollectionChange {
    Add { input: CollectionMemberInput },
    Remove { input: CollectionMemberInput },
    Restore { revision: u64 },
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CollectionEdit {
    pub request_id: String,
    pub expected_revision: u64,
    pub change: CollectionChange,
}
impl From<CollectionEdit> for domain::CollectionEdit {
    fn from(edit: CollectionEdit) -> Self {
        Self {
            request_id: edit.request_id,
            expected_revision: edit.expected_revision,
            change: match edit.change {
                CollectionChange::Add { input } => domain::CollectionChange::Add {
                    input: input.into(),
                },
                CollectionChange::Remove { input } => domain::CollectionChange::Remove {
                    input: input.into(),
                },
                CollectionChange::Restore { revision } => {
                    domain::CollectionChange::Restore { revision }
                }
            },
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct CollectionEditResult {
    pub collection: Collection,
    pub previous_revision: u64,
    pub requested: u64,
    pub changed: u64,
}
impl From<domain::CollectionEditResult> for CollectionEditResult {
    fn from(result: domain::CollectionEditResult) -> Self {
        Self {
            collection: result.collection.into(),
            previous_revision: result.previous_revision,
            requested: result.requested,
            changed: result.changed,
        }
    }
}
