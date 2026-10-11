//! Ports shared by the engine and local visual infrastructure.
use studio_domain::{Result, visual::*};

pub trait VisualRepository: Send + Sync {
    fn recipes(&self) -> Result<Vec<VisualRecipe>>;
    fn run(&self, id: &str) -> Result<VisualRun>;
    fn coverage(&self) -> Result<Vec<VisualCoverage>>;
}
/// Small control messages and bounded receipts; vectors remain in storage.
pub trait VisualExecutor: Send + Sync {
    fn request(&self, operation: &str, args: serde_json::Value) -> Result<serde_json::Value>;
}
