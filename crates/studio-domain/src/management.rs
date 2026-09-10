use crate::{OperatorRun, Result, Selection};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    Project,
    Source,
    Workset,
    Artifact,
    Query,
    Job,
    QueryResult,
    Selection,
    SelectionHistory,
}
impl ObjectKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Source => "source",
            Self::Workset => "workset",
            Self::Artifact => "artifact",
            Self::Query => "query",
            Self::Job => "job",
            Self::QueryResult => "query_result",
            Self::Selection => "selection",
            Self::SelectionHistory => "selection_history",
        }
    }
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "project" => Ok(Self::Project),
            "source" => Ok(Self::Source),
            "workset" | "collection" => Ok(Self::Workset),
            "artifact" => Ok(Self::Artifact),
            "query" | "definition" => Ok(Self::Query),
            "job" => Ok(Self::Job),
            "query_result" | "result" => Ok(Self::QueryResult),
            "selection" => Ok(Self::Selection),
            "selection_history" => Ok(Self::SelectionHistory),
            _ => Err(crate::Error::invalid("未知的项目对象类型")),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedObject {
    pub kind: ObjectKind,
    pub id: String,
    pub name: String,
    pub notes: String,
    pub revision: u64,
    pub state: String,
    pub subtype: Option<String>,
    pub archived: bool,
    pub count: Option<u64>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub bytes: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectLink {
    pub kind: ObjectKind,
    pub id: String,
    pub name: String,
    pub relation: String,
    pub blocking: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectDetails {
    pub object: ManagedObject,
    pub incoming: Vec<ObjectLink>,
    pub outgoing: Vec<ObjectLink>,
    pub incoming_total: u64,
    pub outgoing_total: u64,
    pub incoming_cursor: Option<String>,
    pub outgoing_cursor: Option<String>,
    pub provenance: Value,
    pub paths: Vec<String>,
    pub can_remove: bool,
    pub remove_reason: Option<String>,
    pub run: Option<OperatorRun>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditObject {
    pub expected_revision: u64,
    pub name: String,
    pub notes: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectListing {
    pub kind: ObjectKind,
    pub search: String,
    pub order: String,
    pub state: String,
    pub subtype: Option<String>,
    pub include_archived: bool,
    pub after: Option<String>,
    pub limit: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectPage {
    pub items: Vec<ManagedObject>,
    pub next_cursor: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectLinkPage {
    pub items: Vec<ObjectLink>,
    pub next_cursor: Option<String>,
    pub total: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryStatus {
    pub selection: Selection,
    pub undo_steps: u32,
    pub redo_steps: u32,
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
    pub limit: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditingSettings {
    pub undo_limit: u32,
    pub revision: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPreset {
    pub id: String,
    pub name: String,
    pub notes: String,
    pub revision: u64,
    pub run: OperatorRun,
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresetPage {
    pub items: Vec<ToolPreset>,
    pub next_cursor: Option<String>,
}
