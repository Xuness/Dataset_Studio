use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LakeUpdateRuntime {
    pub python: PathBuf,
    pub store_root: PathBuf,
    pub state_root: PathBuf,
}

/// Commands are independent of HTTP and the worker transport. Frozen definitions
/// are validated by the archive owner's versioned update service.
#[derive(Clone, Copy)]
pub enum LakeUpdateOperation {
    InputCreate,
    Input,
    InputAppend,
    InputSeal,
    Status,
    Capabilities,
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
            Self::InputSeal => "input_seal",
            Self::Status => "status",
            Self::Capabilities => "capabilities",
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
