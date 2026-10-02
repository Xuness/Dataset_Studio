use studio_domain::{Result, lake_updates::*};

/// Archive-owned update control. Arguments and results are a versioned document
/// protocol; transport DTOs and processes stay in the outer adapters.
pub trait LakeUpdateBackend: crate::source_collections::CollectionBackend + Send + Sync {
    fn configure(&self, runtime: LakeUpdateRuntime) -> Result<()>;
    fn configured(&self) -> bool;
    fn health(&self) -> LakeUpdateHealth;
    fn execute(
        &self,
        operation: LakeUpdateOperation,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value>;
    fn shutdown(&self);
}
