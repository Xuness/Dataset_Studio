//! Source-owned Pinterest commands; no HTTP or process transport types.
#[derive(Clone, Copy)]
pub enum PinterestOperation {
    Status,
    Capabilities,
    Lakes,
    LakeCreate,
    LakeRegister,
    Preview,
    Create,
    Jobs,
    Job,
    Items,
    Streams,
    Action,
    Schedules,
    ScheduleSave,
    ScheduleRemove,
}
impl PinterestOperation {
    pub fn name(self) -> &'static str {
        match self {
            Self::Status => "pinterest_status",
            Self::Capabilities => "pinterest_capabilities",
            Self::Lakes => "pinterest_lakes",
            Self::LakeCreate => "pinterest_lake_create",
            Self::LakeRegister => "pinterest_lake_register",
            Self::Preview => "pinterest_preview",
            Self::Create => "pinterest_create",
            Self::Jobs => "pinterest_jobs",
            Self::Job => "pinterest_job",
            Self::Items => "pinterest_items",
            Self::Streams => "pinterest_streams",
            Self::Action => "pinterest_action",
            Self::Schedules => "pinterest_schedules",
            Self::ScheduleSave => "pinterest_schedule_save",
            Self::ScheduleRemove => "pinterest_schedule_remove",
        }
    }
}
