use crate::{RankingFilter, RankingOrder};
use serde::{Deserialize, Serialize};

/// A fixed member scope that can read ordering and scores from an immutable result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankedScope {
    /// Stable member revision, independent of user-visible query aliases.
    pub index_scope: crate::ScopeRef,
    /// Stable presentation identity across refreshes of the same filter.
    pub view_key: String,
    pub schema_version: u32,
    pub workset_id: String,
    pub artifact_id: String,
    pub artifact_name: String,
    pub count: u64,
    pub saved_filter: RankingFilter,
}

impl RankedScope {
    pub fn order(&self, requested: Option<RankingOrder>) -> RankingOrder {
        requested.unwrap_or(self.saved_filter.order)
    }
}
