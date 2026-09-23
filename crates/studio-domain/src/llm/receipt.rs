use super::*;
use serde::{Deserialize, Serialize};

/// Bounded transport evidence, independent of provider and business parsing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRawReceipt {
    pub http_status: u16,
    pub headers: std::collections::BTreeMap<String, String>,
    pub provider_request_id: Option<String>,
    pub protocol: LlmProtocol,
    pub adapter_version: String,
    pub complete: bool,
    pub failure: Option<LlmFailure>,
    /// The blob is stored separately; its digest covers exactly these retained bytes.
    #[serde(skip)]
    pub body: Vec<u8>,
}
pub const LLM_RECEIPT_LIMIT: usize = 16 * 1024 * 1024;
