use serde::{Deserialize, Serialize};
use studio_domain as domain;
mod metadata;
pub use metadata::*;
mod query;
pub use query::*;
mod scope;
pub use scope::*;
mod tools;
pub use tools::*;
mod resources;
pub use resources::*;
mod settings;
pub use settings::*;
mod ranking;
pub use ranking::*;
mod ranking_browse;
pub use ranking_browse::*;
mod management;
pub use management::*;
use utoipa::ToSchema;
pub const API_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EngineConnection {
    pub api_version: u32,
    pub instance_id: String,
    pub pid: u32,
    pub endpoint: String,
    pub token: String,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct Health {
    pub api_version: u32,
    pub version: String,
    pub instance_id: String,
}
#[derive(Serialize, ToSchema)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    pub request_id: String,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct AssetKey {
    pub source_id: String,
    pub asset_id: String,
}
impl From<AssetKey> for domain::AssetKey {
    fn from(k: AssetKey) -> Self {
        Self {
            source_id: k.source_id,
            asset_id: k.asset_id,
        }
    }
}
impl From<domain::AssetKey> for AssetKey {
    fn from(k: domain::AssetKey) -> Self {
        Self {
            source_id: k.source_id,
            asset_id: k.asset_id,
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct Asset {
    pub key: AssetKey,
    pub name: String,
    pub bytes: String,
    pub extension: String,
    pub source_name: String,
    pub selected: bool,
    pub summary: Option<AssetSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ranking: Option<AssetRanking>,
}
impl Asset {
    pub fn from_domain(a: domain::Asset, selected: bool) -> Self {
        Self {
            key: a.key.into(),
            name: a.name,
            bytes: a.bytes.to_string(),
            extension: a.extension,
            source_name: a.source_name,
            selected,
            summary: None,
            ranking: None,
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct AssetPage {
    pub items: Vec<Asset>,
    pub next_cursor: Option<String>,
    pub revision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preparing: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scan: Option<BrowseScan>,
    /// Reusable first-page cursor after a Danbooru ID has been located.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_cursor: Option<String>,
}
#[derive(Serialize, ToSchema)]
pub struct BrowseScan {
    pub scanned: u64,
    pub total: u64,
}
#[derive(Serialize, ToSchema)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub directory: String,
    pub created_at: String,
    pub revision: u64,
}
impl From<domain::Project> for Project {
    fn from(p: domain::Project) -> Self {
        Self {
            id: p.id,
            name: p.name,
            directory: display_path(&p.directory),
            created_at: p.created_at,
            revision: p.revision,
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct Projects {
    pub items: Vec<ProjectSummary>,
}
#[derive(Serialize, ToSchema)]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    pub directory: String,
    pub opened_at: String,
    pub state: ProjectState,
    pub issue: Option<String>,
}
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProjectState {
    Closed,
    Open,
    Background,
    Draining,
    Unavailable,
}
impl From<domain::ProjectState> for ProjectState {
    fn from(value: domain::ProjectState) -> Self {
        match value {
            domain::ProjectState::Closed => Self::Closed,
            domain::ProjectState::Open => Self::Open,
            domain::ProjectState::Background => Self::Background,
            domain::ProjectState::Draining => Self::Draining,
            domain::ProjectState::Unavailable => Self::Unavailable,
        }
    }
}
impl From<domain::ProjectSummary> for ProjectSummary {
    fn from(p: domain::ProjectSummary) -> Self {
        Self {
            id: p.id,
            name: p.name,
            directory: display_path(&p.directory),
            opened_at: p.opened_at,
            state: p.state.into(),
            issue: p.issue,
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct ProjectClose {
    pub project_id: String,
    pub state: ProjectState,
}
impl From<domain::ProjectClose> for ProjectClose {
    fn from(p: domain::ProjectClose) -> Self {
        Self {
            project_id: p.project_id,
            state: p.state.into(),
        }
    }
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateProject {
    pub name: String,
    pub parent_directory: Option<String>,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenProject {
    pub directory: String,
}
#[derive(Serialize, ToSchema)]
pub struct Source {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub revision: Option<String>,
    pub enumeration: String,
    pub count: Option<u64>,
    pub available: bool,
    pub issue: Option<String>,
}
#[derive(Serialize, ToSchema)]
pub struct Sources {
    pub items: Vec<Source>,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AttachSource {
    pub kind: String,
    pub name: String,
    pub index_root: Option<String>,
    pub media_root: Option<String>,
}
#[derive(Deserialize, ToSchema)]
pub struct BrowseQuery {
    pub order: Option<QueryOrder>,
    pub source_id: Option<String>,
    pub collection_id: Option<String>,
    pub selection: Option<bool>,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}
#[derive(Serialize, ToSchema)]
pub struct Selection {
    pub revision: u64,
    pub count: u64,
    pub base_result: Option<String>,
    pub excluded_count: u64,
}
impl From<domain::Selection> for Selection {
    fn from(s: domain::Selection) -> Self {
        Self {
            revision: s.revision,
            count: s.count,
            base_result: s.base_result,
            excluded_count: s.excluded_count,
        }
    }
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeSelection {
    pub expected_revision: u64,
    pub add: Vec<AssetKey>,
    pub remove: Vec<AssetKey>,
    pub clear: bool,
}
#[derive(Serialize, ToSchema)]
pub struct Collection {
    pub id: String,
    pub name: String,
    pub count: u64,
}
impl From<domain::Collection> for Collection {
    fn from(c: domain::Collection) -> Self {
        Self {
            id: c.id,
            name: c.name,
            count: c.count,
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct Collections {
    pub items: Vec<Collection>,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateCollection {
    pub name: String,
    pub scope: Option<ScopeRef>,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SubmitJob {
    pub idempotency_key: String,
    pub selection_revision: Option<u64>,
    pub scope: Option<ScopeRef>,
    #[serde(default)]
    pub delay_ms: u64,
}
#[derive(Serialize, ToSchema)]
pub struct Job {
    pub id: String,
    pub project_id: String,
    pub operator: String,
    pub status: String,
    pub total: u64,
    pub completed: u64,
    pub attempt: u32,
    pub created_at: String,
    pub error: Option<String>,
    pub artifact: Option<String>,
    pub input_scope: Option<ScopeRef>,
    pub input_members_frozen: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<JobStage>,
}
#[derive(Serialize, ToSchema)]
pub struct JobStage {
    pub name: String,
    pub completed: u64,
    pub total: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<JobTelemetry>,
}
#[derive(Serialize, ToSchema)]
pub struct JobTelemetry {
    pub started_at: String,
    pub updated_at: String,
    pub heartbeat_at: String,
    pub finished_at: Option<String>,
    pub phases: Vec<JobPhaseTiming>,
}
#[derive(Serialize, ToSchema)]
pub struct JobPhaseTiming {
    pub name: String,
    pub rating: Option<String>,
    pub elapsed_ms: u64,
}
impl From<domain::Job> for Job {
    fn from(j: domain::Job) -> Self {
        Self {
            id: j.id,
            project_id: j.project_id,
            operator: j.operator,
            status: j.status,
            total: j.total,
            completed: j.completed,
            attempt: j.attempt,
            created_at: j.created_at,
            error: j.error,
            artifact: j.artifact,
            input_scope: j.input_scope.map(Into::into),
            input_members_frozen: j.input_members_frozen,
            stage: j.stage.map(|s| JobStage {
                name: s.name,
                completed: s.completed,
                total: s.total,
                rating: s.rating,
                telemetry: s.telemetry.map(|t| JobTelemetry {
                    started_at: t.started_at,
                    updated_at: t.updated_at,
                    heartbeat_at: t.heartbeat_at,
                    finished_at: t.finished_at,
                    phases: t
                        .phases
                        .into_iter()
                        .map(|p| JobPhaseTiming {
                            name: p.name,
                            rating: p.rating,
                            elapsed_ms: p.elapsed_ms,
                        })
                        .collect(),
                }),
            }),
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct Jobs {
    pub items: Vec<Job>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct ProjectEvent {
    pub sequence: u64,
    pub project_id: String,
    pub kind: String,
    pub resource_id: String,
}
impl From<domain::ProjectEvent> for ProjectEvent {
    fn from(e: domain::ProjectEvent) -> Self {
        Self {
            sequence: e.sequence,
            project_id: e.project_id,
            kind: e.kind,
            resource_id: e.resource_id,
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct OkResponse {
    pub ok: bool,
}

pub fn display_path(path: &std::path::Path) -> String {
    let text = path.to_string_lossy();
    if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{}", unc)
    } else {
        text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
    }
}
