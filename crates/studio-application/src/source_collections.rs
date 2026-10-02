use studio_domain::{Result, source_collections::CollectionOperation};

/// Shares the owned runtime with lake updates; this port has no transport DTOs.
pub trait CollectionBackend: Send + Sync {
    fn execute_collection(
        &self,
        operation: CollectionOperation,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value>;
}
