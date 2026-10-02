use serde::{Deserialize, Serialize};
use std::path::PathBuf;
pub mod aesthetic;
pub mod aesthetic_analysis;
pub mod lake_updates;
pub mod llm;
pub mod source_collections;
mod sources;
pub use sources::*;
mod metadata;
pub use metadata::*;
mod lifecycle;
pub use lifecycle::*;
mod query;
pub use query::*;
mod scope;
pub use scope::*;
mod operators;
pub use operators::*;
mod artifacts;
pub use artifacts::*;
mod drafts;
pub use drafts::*;
mod resources;
pub use resources::*;
mod ranking;
pub use ranking::*;
mod ranking_v2;
pub use ranking_v2::*;
mod ranking_browse;
pub use ranking_browse::*;
mod management;
pub use management::*;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, thiserror::Error)]
#[error("{code}: {message}")]
pub struct Error {
    pub code: &'static str,
    pub message: String,
}

impl Error {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("INVALID_INPUT", message)
    }
    pub fn io(error: impl std::fmt::Display) -> Self {
        Self::new("IO_ERROR", error.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct AssetKey {
    pub source_id: String,
    pub asset_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub index_root: Option<PathBuf>,
    pub media_root: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub key: AssetKey,
    pub name: String,
    pub bytes: u64,
    pub extension: String,
    pub source_name: String,
}

#[derive(Debug, Clone)]
pub struct AssetPage {
    pub items: Vec<Asset>,
    pub next: Option<String>,
    pub revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub directory: PathBuf,
    pub created_at: String,
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Collection {
    pub id: String,
    pub name: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Selection {
    pub revision: u64,
    pub count: u64,
    pub base_result: Option<String>,
    pub excluded_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(default)]
    pub input_scope: Option<ScopeRef>,
    #[serde(default)]
    pub input_members_frozen: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<JobStage>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobStage {
    pub name: String,
    pub completed: u64,
    pub total: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<JobTelemetry>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobTelemetry {
    pub started_at: String,
    pub updated_at: String,
    pub heartbeat_at: String,
    pub finished_at: Option<String>,
    pub phases: Vec<JobPhaseTiming>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobPhaseTiming {
    pub name: String,
    pub rating: Option<String>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectEvent {
    pub sequence: u64,
    pub project_id: String,
    pub kind: String,
    pub resource_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrozenInput {
    pub asset: Asset,
    pub source_revision: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FrozenField>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerPlan {
    pub version: u32,
    pub job_id: String,
    pub input_path: PathBuf,
    pub input_sha256: String,
    pub output_path: PathBuf,
    pub checkpoint_path: PathBuf,
    pub total: u64,
    pub delay_ms: u64,
    #[serde(default)]
    pub run: OperatorRun,
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}
pub fn validate_id(id: &str) -> Result<()> {
    uuid::Uuid::parse_str(id)
        .map(|_| ())
        .map_err(|_| Error::invalid("无效的对象标识"))
}
pub fn validate_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 120 || name.chars().any(char::is_control) {
        return Err(Error::invalid("名称需要为 1–120 个可显示字符"));
    }
    Ok(name.to_owned())
}
