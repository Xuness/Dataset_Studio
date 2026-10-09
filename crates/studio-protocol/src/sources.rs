use crate::domain;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct SourceCapabilities {
    pub browse: bool,
    pub media: bool,
    pub metadata: bool,
    pub query: bool,
    pub post_order: bool,
    pub identity_summaries: bool,
    pub relink: bool,
    pub raw_metadata: bool,
    pub incremental: bool,
    pub stored_dimensions: bool,
    pub work_members: bool,
    pub author_metadata: bool,
    pub literal_tags: bool,
}
#[derive(Serialize, ToSchema)]
pub struct SourceDescriptor {
    pub version: u32,
    pub backend_id: String,
    pub display_name: String,
    pub site_id: Option<String>,
    pub semantics_version: String,
    pub capabilities: SourceCapabilities,
    pub projections: Vec<String>,
}
impl From<domain::SourceDescriptor> for SourceDescriptor {
    fn from(v: domain::SourceDescriptor) -> Self {
        let c = v.capabilities;
        Self {
            version: v.version,
            backend_id: v.backend_id,
            display_name: v.display_name,
            site_id: v.site_id,
            semantics_version: v.semantics_version,
            projections: v.projections,
            capabilities: SourceCapabilities {
                browse: c.browse,
                media: c.media,
                metadata: c.metadata,
                query: c.query,
                post_order: c.post_order,
                identity_summaries: c.identity_summaries,
                relink: c.relink,
                raw_metadata: c.raw_metadata,
                incremental: c.incremental,
                stored_dimensions: c.stored_dimensions,
                work_members: c.work_members,
                author_metadata: c.author_metadata,
                literal_tags: c.literal_tags,
            },
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct SourceRegistration {
    pub kind: String,
    pub name: String,
    pub descriptor: SourceDescriptor,
}
#[derive(Serialize, ToSchema)]
pub struct SourceRegistrations {
    pub items: Vec<SourceRegistration>,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ProbeSource {
    pub kind: String,
    pub index_root: Option<String>,
    pub media_root: Option<String>,
}
#[derive(Serialize, ToSchema)]
pub struct SourcePreflight {
    pub source_id: String,
    pub kind: String,
    pub descriptor: SourceDescriptor,
    pub revision: String,
    pub enumeration: String,
    pub count: Option<String>,
    pub analysis_sequence: Option<String>,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceRequirementsRequest {
    pub scope: crate::ScopeRef,
    pub projections: Vec<String>,
}
#[derive(Serialize, ToSchema)]
pub struct SourceRequirementStatus {
    pub source_id: String,
    pub name: String,
    pub supported: bool,
    pub reason: Option<String>,
}
#[derive(Serialize, ToSchema)]
pub struct SourceRequirementsResult {
    pub supported: bool,
    pub sources: Vec<SourceRequirementStatus>,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveOriginal {
    /// Absolute file path chosen by the user; never inside an attached lake.
    pub path: String,
}
#[derive(Serialize, ToSchema)]
pub struct SavedOriginal {
    pub path: String,
    pub bytes: u64,
}
