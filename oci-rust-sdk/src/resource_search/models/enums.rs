use serde::{Deserialize, Serialize};

/// The type of matching context returned in the response.
///
/// If you specify HIGHLIGHTS, then the service will highlight fragments in its response.
/// The default setting is NONE.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MatchingContextType {
    /// No matching context
    None,
    /// Include highlighted fragments showing what matched
    Highlights,
}
