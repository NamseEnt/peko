use serde::{Deserialize, Serialize};

use super::resource_summary::ResourceSummary;

/// A collection of resource summaries returned from a search query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceSummaryCollection {
    /// The list of resource summaries.
    pub items: Vec<ResourceSummary>,
}
