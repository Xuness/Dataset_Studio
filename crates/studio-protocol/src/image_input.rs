use serde::Serialize;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct ImageInputInfo {
    pub source_sha256: String,
    pub sha256: String,
    pub source_width: Option<u32>,
    pub source_height: Option<u32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub source_bytes: u64,
    pub bytes: u64,
    pub content_type: String,
    pub max_edge: Option<u32>,
    pub transform_version: String,
}
impl From<studio_domain::ImageInputInfo> for ImageInputInfo {
    fn from(v: studio_domain::ImageInputInfo) -> Self {
        Self {
            source_sha256: v.source_sha256,
            sha256: v.sha256,
            source_width: v.source_width,
            source_height: v.source_height,
            width: v.width,
            height: v.height,
            source_bytes: v.source_bytes,
            bytes: v.bytes,
            content_type: v.content_type,
            max_edge: v.max_edge,
            transform_version: v.transform_version,
        }
    }
}
