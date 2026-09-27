use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Serialize, Deserialize)]
pub struct LakeInputPreparation {
    pub id: String,
    pub project_id: String,
    #[serde(default)]
    pub project_name: String,
    #[serde(default)]
    pub label: String,
    pub scope: crate::ScopeRef,
    pub state: String,
    pub result_id: Option<String>,
    pub after: Option<crate::AssetKey>,
    pub processed: u64,
    pub total: Option<u64>,
    pub inputs: Vec<LakePreparedInput>,
    pub error: Option<String>,
    #[serde(default)]
    pub versions: Vec<crate::QuerySourceVersion>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct LakePreparedInput {
    pub library_id: String,
    pub input_id: String,
    pub count: u64,
    pub sealed: bool,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LakeUpdateRuntime {
    pub python: PathBuf,
    pub state_root: PathBuf,
}

/// Commands are independent of HTTP and the worker transport. Frozen definitions
/// are validated by the archive owner's versioned update service.
#[derive(Clone, Copy)]
pub enum LakeUpdateOperation {
    InputCreate,
    Input,
    InputAppend,
    InputAppendBatch,
    InputSeal,
    Status,
    Capabilities,
    PipelineGet,
    PipelineSet,
    Register,
    Lakes,
    CredentialSet,
    CredentialDelete,
    Probe,
    Preview,
    Create,
    Jobs,
    Job,
    Items,
    Action,
    Coverage,
    ScheduleSet,
    Schedules,
    ScheduleDelete,
}
impl LakeUpdateOperation {
    pub fn name(self) -> &'static str {
        match self {
            Self::InputCreate => "input_create",
            Self::Input => "input",
            Self::InputAppend => "input_append",
            Self::InputAppendBatch => "input_append_batch",
            Self::InputSeal => "input_seal",
            Self::Status => "status",
            Self::Capabilities => "capabilities",
            Self::PipelineGet => "pipeline_get",
            Self::PipelineSet => "pipeline_set",
            Self::Register => "register",
            Self::Lakes => "lakes",
            Self::CredentialSet => "credential_set",
            Self::CredentialDelete => "credential_delete",
            Self::Probe => "probe",
            Self::Preview => "preview",
            Self::Create => "create",
            Self::Jobs => "jobs",
            Self::Job => "job",
            Self::Items => "items",
            Self::Action => "action",
            Self::Coverage => "coverage",
            Self::ScheduleSet => "schedule_set",
            Self::Schedules => "schedules",
            Self::ScheduleDelete => "schedule_delete",
        }
    }
}
