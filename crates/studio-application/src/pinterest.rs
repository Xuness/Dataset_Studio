use studio_domain::{Result, pinterest::PinterestOperation};

pub trait PinterestBackend: Send + Sync {
    fn execute_pinterest(
        &self,
        operation: PinterestOperation,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value>;
}
