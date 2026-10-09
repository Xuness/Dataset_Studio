use serde::{Deserialize, Serialize};
use studio_domain as domain;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ScopeRef {
    pub project_id: String,
    pub target: ScopeTarget,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScopeTarget {
    Source {
        source_id: String,
        revision: String,
    },
    Workset {
        collection_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        revision: Option<u64>,
    },
    QueryResult {
        result_id: String,
    },
    Selection {
        revision: u64,
    },
}
impl From<ScopeRef> for domain::ScopeRef {
    fn from(s: ScopeRef) -> Self {
        Self {
            project_id: s.project_id,
            target: match s.target {
                ScopeTarget::Source {
                    source_id,
                    revision,
                } => domain::ScopeTarget::Source {
                    source_id,
                    revision,
                },
                ScopeTarget::Workset {
                    collection_id,
                    revision,
                } => domain::ScopeTarget::Workset {
                    collection_id,
                    revision,
                },
                ScopeTarget::QueryResult { result_id } => {
                    domain::ScopeTarget::QueryResult { result_id }
                }
                ScopeTarget::Selection { revision } => domain::ScopeTarget::Selection { revision },
            },
        }
    }
}
impl From<domain::ScopeRef> for ScopeRef {
    fn from(s: domain::ScopeRef) -> Self {
        Self {
            project_id: s.project_id,
            target: match s.target {
                domain::ScopeTarget::Source {
                    source_id,
                    revision,
                } => ScopeTarget::Source {
                    source_id,
                    revision,
                },
                domain::ScopeTarget::Workset {
                    collection_id,
                    revision,
                } => ScopeTarget::Workset {
                    collection_id,
                    revision,
                },
                domain::ScopeTarget::QueryResult { result_id } => {
                    ScopeTarget::QueryResult { result_id }
                }
                domain::ScopeTarget::Selection { revision } => ScopeTarget::Selection { revision },
            },
        }
    }
}
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScopeOperation {
    Replace,
    Add,
    Remove,
    Intersect,
}
impl From<ScopeOperation> for domain::ScopeOperation {
    fn from(o: ScopeOperation) -> Self {
        match o {
            ScopeOperation::Replace => Self::Replace,
            ScopeOperation::Add => Self::Add,
            ScopeOperation::Remove => Self::Remove,
            ScopeOperation::Intersect => Self::Intersect,
        }
    }
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeSelectionScope {
    pub expected_revision: u64,
    pub scope: ScopeRef,
    pub operation: ScopeOperation,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptureScope {
    pub scope: ScopeRef,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RelinkSource {
    pub index_root: String,
    pub media_root: String,
}
#[derive(Serialize, ToSchema)]
pub struct SourceRelinked {
    pub source_id: String,
    pub revision: String,
    pub impact: String,
}
