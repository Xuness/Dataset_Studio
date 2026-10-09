//! Source-owned Pinterest commands; no HTTP or process transport types.
#[derive(Clone, Copy)]
pub enum PinterestOperation {
    Status,
    Capabilities,
    Lakes,
    LakeCreate,
    Preview,
    Create,
    Jobs,
    Job,
    Items,
    Action,
}
impl PinterestOperation {
    pub fn name(self) -> &'static str {
        match self {
            Self::Status => "pinterest_status",
            Self::Capabilities => "pinterest_capabilities",
            Self::Lakes => "pinterest_lakes",
            Self::LakeCreate => "pinterest_lake_create",
            Self::Preview => "pinterest_preview",
            Self::Create => "pinterest_create",
            Self::Jobs => "pinterest_jobs",
            Self::Job => "pinterest_job",
            Self::Items => "pinterest_items",
            Self::Action => "pinterest_action",
        }
    }
}
