use crate::{Job, OperatorRun, Selection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use studio_domain as domain;
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
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
impl From<domain::ObjectKind> for ObjectKind {
    fn from(v: domain::ObjectKind) -> Self {
        match v {
            domain::ObjectKind::Project => Self::Project,
            domain::ObjectKind::Source => Self::Source,
            domain::ObjectKind::Workset => Self::Workset,
            domain::ObjectKind::Artifact => Self::Artifact,
            domain::ObjectKind::Query => Self::Query,
            domain::ObjectKind::Job => Self::Job,
            domain::ObjectKind::QueryResult => Self::QueryResult,
            domain::ObjectKind::Selection => Self::Selection,
            domain::ObjectKind::SelectionHistory => Self::SelectionHistory,
        }
    }
}
#[derive(Serialize, ToSchema)]
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
impl From<domain::ManagedObject> for ManagedObject {
    fn from(v: domain::ManagedObject) -> Self {
        Self {
            kind: v.kind.into(),
            id: v.id,
            name: v.name,
            notes: v.notes,
            revision: v.revision,
            state: v.state,
            subtype: v.subtype,
            archived: v.archived,
            count: v.count,
            created_at: v.created_at,
            updated_at: v.updated_at,
            bytes: v.bytes,
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct ObjectPage {
    pub items: Vec<ManagedObject>,
    pub next_cursor: Option<String>,
}
#[derive(Serialize, ToSchema)]
pub struct ManagedJob {
    pub object: ManagedObject,
    pub job: Job,
    pub result_available: bool,
}
#[derive(Serialize, ToSchema)]
pub struct ManagedJobPage {
    pub items: Vec<ManagedJob>,
    pub next_cursor: Option<String>,
}
impl From<domain::ObjectPage> for ObjectPage {
    fn from(v: domain::ObjectPage) -> Self {
        Self {
            items: v.items.into_iter().map(Into::into).collect(),
            next_cursor: v.next_cursor,
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct ObjectLink {
    pub kind: ObjectKind,
    pub id: String,
    pub name: String,
    pub relation: String,
    pub blocking: bool,
}
impl From<domain::ObjectLink> for ObjectLink {
    fn from(v: domain::ObjectLink) -> Self {
        Self {
            kind: v.kind.into(),
            id: v.id,
            name: v.name,
            relation: v.relation,
            blocking: v.blocking,
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct ObjectLinkPage {
    pub items: Vec<ObjectLink>,
    pub next_cursor: Option<String>,
    pub total: u64,
}
impl From<domain::ObjectLinkPage> for ObjectLinkPage {
    fn from(v: domain::ObjectLinkPage) -> Self {
        Self {
            items: v.items.into_iter().map(Into::into).collect(),
            next_cursor: v.next_cursor,
            total: v.total,
        }
    }
}
#[derive(Serialize, ToSchema)]
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
impl From<domain::ObjectDetails> for ObjectDetails {
    fn from(v: domain::ObjectDetails) -> Self {
        Self {
            object: v.object.into(),
            incoming: v.incoming.into_iter().map(Into::into).collect(),
            outgoing: v.outgoing.into_iter().map(Into::into).collect(),
            incoming_total: v.incoming_total,
            outgoing_total: v.outgoing_total,
            incoming_cursor: v.incoming_cursor,
            outgoing_cursor: v.outgoing_cursor,
            provenance: v.provenance,
            paths: v.paths,
            can_remove: v.can_remove,
            remove_reason: v.remove_reason,
            run: v.run.map(Into::into),
        }
    }
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EditObject {
    pub expected_revision: u64,
    pub name: String,
    pub notes: String,
}
impl From<EditObject> for domain::EditObject {
    fn from(v: EditObject) -> Self {
        Self {
            expected_revision: v.expected_revision,
            name: v.name,
            notes: v.notes,
        }
    }
}
#[derive(Clone, Copy, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ObjectActionKind {
    Remove,
    Archive,
    Unarchive,
    Reconnect,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ObjectAction {
    pub action: ObjectActionKind,
    pub expected_revision: u64,
}
#[derive(Serialize, ToSchema)]
pub struct HistoryStatus {
    pub selection: Selection,
    pub undo_steps: u32,
    pub redo_steps: u32,
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
    pub limit: u32,
}
impl From<domain::HistoryStatus> for HistoryStatus {
    fn from(v: domain::HistoryStatus) -> Self {
        Self {
            selection: v.selection.into(),
            undo_steps: v.undo_steps,
            redo_steps: v.redo_steps,
            undo_label: v.undo_label,
            redo_label: v.redo_label,
            limit: v.limit,
        }
    }
}
#[derive(Clone, Copy, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HistoryActionKind {
    Undo,
    Redo,
    Clear,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct HistoryAction {
    pub action: HistoryActionKind,
    pub expected_revision: u64,
}
#[derive(Serialize, ToSchema)]
pub struct EditingSettings {
    pub undo_limit: u32,
    pub revision: u64,
}
impl From<domain::EditingSettings> for EditingSettings {
    fn from(v: domain::EditingSettings) -> Self {
        Self {
            undo_limit: v.undo_limit,
            revision: v.revision,
        }
    }
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfigureEditing {
    pub undo_limit: u32,
    pub expected_revision: u64,
}
#[derive(Serialize, ToSchema)]
pub struct ToolPreset {
    pub id: String,
    pub name: String,
    pub notes: String,
    pub revision: u64,
    pub run: OperatorRun,
    pub created_at: String,
    pub updated_at: String,
}
impl From<domain::ToolPreset> for ToolPreset {
    fn from(v: domain::ToolPreset) -> Self {
        Self {
            id: v.id,
            name: v.name,
            notes: v.notes,
            revision: v.revision,
            run: v.run.into(),
            created_at: v.created_at,
            updated_at: v.updated_at,
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct PresetPage {
    pub items: Vec<ToolPreset>,
    pub next_cursor: Option<String>,
}
impl From<domain::PresetPage> for PresetPage {
    fn from(v: domain::PresetPage) -> Self {
        Self {
            items: v.items.into_iter().map(Into::into).collect(),
            next_cursor: v.next_cursor,
        }
    }
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveToolPreset {
    pub id: Option<String>,
    pub name: String,
    pub notes: String,
    pub expected_revision: u64,
    pub run: OperatorRun,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteToolPreset {
    pub expected_revision: u64,
}
fn yes() -> bool {
    true
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RevealObject {
    #[serde(default)]
    pub file_index: usize,
    #[serde(default = "yes")]
    pub open: bool,
}
#[derive(Serialize, ToSchema)]
pub struct RevealedLocation {
    pub path: String,
    pub opened: bool,
}
