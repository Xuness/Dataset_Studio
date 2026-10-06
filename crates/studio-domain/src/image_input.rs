//! Local image preparation settings and evidence, independent of any provider protocol.
use serde::{Deserialize, Serialize};

pub const IMAGE_INPUT_MIN_EDGE: u32 = 128;
pub const IMAGE_INPUT_MAX_EDGE: u32 = 8192;

pub fn validate_image_max_edge(edge: Option<u32>) -> crate::Result<()> {
    if edge.is_some_and(|v| !(IMAGE_INPUT_MIN_EDGE..=IMAGE_INPUT_MAX_EDGE).contains(&v)) {
        return Err(crate::Error::invalid(
            "API 图片最长边须为 128–8192 像素，或选择原尺寸",
        ));
    }
    Ok(())
}

/// Metadata of the bytes actually sent. Original bytes and their identity stay in the lake.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
