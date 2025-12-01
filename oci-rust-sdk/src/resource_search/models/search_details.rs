use serde::{Deserialize, Serialize};

use super::enums::MatchingContextType;

/// Search criteria for querying resources.
///
/// This is a polymorphic type that can be either a structured query or free text search.
/// The type is determined by the `type` field in the JSON representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SearchDetails {
    /// A structured query using Search's query language
    Structured(StructuredSearchDetails),
    /// A free text search
    FreeText(FreeTextSearchDetails),
}

/// A request that uses Search's structured query language to specify filter conditions.
///
/// For more information about writing queries, see the Search Language Syntax documentation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StructuredSearchDetails {
    /// The structured query describing which resources to search for.
    ///
    /// Example: "query instance resources where lifecycleState = 'RUNNING'"
    pub query: String,

    /// The type of matching context to return in the response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matching_context_type: Option<MatchingContextType>,
}

/// A request that uses free text search.
///
/// Searches across resource names, descriptions, and other text fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FreeTextSearchDetails {
    /// The text to search for.
    pub text: String,

    /// The type of matching context to return in the response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matching_context_type: Option<MatchingContextType>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_structured_search_serialization() {
        let details = SearchDetails::Structured(StructuredSearchDetails {
            query: "query instance resources".to_string(),
            matching_context_type: Some(MatchingContextType::Highlights),
        });

        let json = serde_json::to_string(&details).unwrap();
        assert!(json.contains(r#""type":"Structured"#));
        assert!(json.contains(r#""query":"query instance resources"#));
        assert!(json.contains(r#""matchingContextType":"HIGHLIGHTS"#));
    }

    #[test]
    fn test_free_text_search_serialization() {
        let details = SearchDetails::FreeText(FreeTextSearchDetails {
            text: "search text".to_string(),
            matching_context_type: Some(MatchingContextType::None),
        });

        let json = serde_json::to_string(&details).unwrap();
        assert!(json.contains(r#""type":"FreeText"#));
        assert!(json.contains(r#""text":"search text"#));
        assert!(json.contains(r#""matchingContextType":"NONE"#));
    }

    #[test]
    fn test_structured_search_deserialization() {
        let json = r#"{"type":"Structured","query":"query user resources","matchingContextType":"HIGHLIGHTS"}"#;
        let details: SearchDetails = serde_json::from_str(json).unwrap();

        match details {
            SearchDetails::Structured(structured) => {
                assert_eq!(structured.query, "query user resources");
                assert_eq!(
                    structured.matching_context_type,
                    Some(MatchingContextType::Highlights)
                );
            }
            _ => panic!("Expected Structured variant"),
        }
    }

    #[test]
    fn test_free_text_search_deserialization() {
        let json = r#"{"type":"FreeText","text":"test search"}"#;
        let details: SearchDetails = serde_json::from_str(json).unwrap();

        match details {
            SearchDetails::FreeText(free_text) => {
                assert_eq!(free_text.text, "test search");
                assert_eq!(free_text.matching_context_type, None);
            }
            _ => panic!("Expected FreeText variant"),
        }
    }
}
