use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Recent entries are cached descriptions, never evidence that a project is open.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    pub directory: PathBuf,
    pub opened_at: String,
    pub state: ProjectState,
    pub issue: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectState {
    Closed,
    Open,
    Background,
    Draining,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectClose {
    pub project_id: String,
    pub state: ProjectState,
}
#[derive(Debug, Clone)]
pub struct MemberWriteProgress {
    pub state: String,
    pub completed: u64,
    pub total: Option<u64>,
    pub error: Option<String>,
}
