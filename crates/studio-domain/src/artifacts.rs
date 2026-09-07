use crate::{AssetKey, OperatorRun, ScalarValue, ScopeRef};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactState {
    Legacy,
    Publishing,
    Ready,
    Released,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactFile {
    pub path: String,
    pub bytes: Option<u64>,
    pub sha256: Option<String>,
    pub media_type: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactProvenance {
    pub run: Option<OperatorRun>,
    pub input_scope: Option<ScopeRef>,
    pub input_sha256: Option<String>,
    pub attempt: Option<u32>,
    pub input_artifacts: Vec<String>,
    pub fields_frozen: bool,
    pub evidence: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub project_id: String,
    pub job_id: String,
    pub output_id: String,
    pub name: String,
    pub kind: String,
    pub schema_version: u32,
    pub state: ArtifactState,
    pub count: Option<u64>,
    pub created_at: String,
    pub files: Vec<ArtifactFile>,
    pub provenance: ArtifactProvenance,
    pub issue: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactRow {
    pub key: AssetKey,
    pub ordinal: u64,
    pub scalar: Option<ScalarValue>,
    pub data: serde_json::Value,
}
#[derive(Debug, Clone)]
pub struct ArtifactPage {
    pub items: Vec<ArtifactRow>,
    pub next: Option<AssetKey>,
}
