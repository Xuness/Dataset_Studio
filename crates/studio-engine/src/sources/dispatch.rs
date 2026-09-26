use std::sync::Arc;
use studio_application::{ReadLease, ReadResources, SourceReadContext};
use studio_domain::*;

/// Admission is shared with previews, decoding and project work. Backends receive
/// an existing reservation and never recursively acquire the same resource.
pub(super) struct SourceDispatcher {
    resources: Arc<dyn ReadResources>,
}
impl SourceDispatcher {
    pub fn new(resources: Arc<dyn ReadResources>) -> Self {
        Self { resources }
    }
    pub fn admit(
        &self,
        class: ReadClass,
        bytes: u64,
        context: &SourceReadContext,
    ) -> Result<Box<dyn ReadLease>> {
        context.check()?;
        let lease = self.resources.acquire_until(
            ReadRequest {
                class,
                priority: context.priority,
                bytes,
            },
            &context.cancelled,
            context.deadline,
        )?;
        context.check()?;
        Ok(lease)
    }
}
