use crate::Media;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use studio_domain::*;

pub type ReadCancellation = Arc<AtomicBool>;
pub fn read_cancelled(cancelled: &AtomicBool) -> Result<()> {
    if cancelled.load(Ordering::Acquire) {
        Err(Error::new("CANCELLED", "读取请求已取消"))
    } else {
        Ok(())
    }
}
pub trait ReadLease: Send {}
/// Blocking application port. Infrastructure calls it from a bounded worker pool.
pub trait ReadResources: Send + Sync {
    fn acquire(&self, request: ReadRequest, cancelled: &AtomicBool) -> Result<Box<dyn ReadLease>>;
    fn metrics(&self) -> Vec<ReadMetrics>;
}
pub struct MediaInput {
    pub asset_id: String,
    pub cancelled: ReadCancellation,
    /// Upper bound admitted before opening the source payload. Adapters must
    /// recheck the current index length against it before allocation or I/O.
    pub byte_limit: u64,
}
pub struct MediaBatch {
    /// Same identity and order as the caller's bounded input, including errors.
    pub items: Vec<Result<Media>>,
    pub stats: PhysicalReadStats,
}
#[derive(Debug, Clone)]
pub struct MediaIdentity {
    pub content_version: String,
    pub source_revision: String,
    pub bytes: u64,
}
pub trait MediaSource: Send + Sync {
    /// Derive a stable candidate without I/O; an offline hit still needs a previously
    /// verified cache record. Online use must also call verify_media_identity.
    fn content_version(&self, source: &Source, asset_id: &str) -> Result<String>;
    fn verify_media_identity(&self, source: &Source, asset_id: &str) -> Result<MediaIdentity>;
    fn read_many(&self, source: &Source, inputs: &[MediaInput]) -> Result<MediaBatch>;
}
