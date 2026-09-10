use crate::{RankingFilter, RankingOrder};
use serde::{Deserialize, Serialize};

/// A fixed member scope that can read ordering and scores from an immutable result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankedScope {
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
