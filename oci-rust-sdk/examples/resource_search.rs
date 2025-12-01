use oci_rust_sdk::{
    core::{auth::ConfigFileAuthProvider, region::Region, OciClient},
    resource_search::{
        MatchingContextType, ResourceSearch, SearchDetails, SearchResourcesRequest,
        StructuredSearchDetails,
    },
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up authentication using default config file
    let auth = Arc::new(ConfigFileAuthProvider::from_default()?);

    // Resource Search uses a different endpoint pattern (query.region)
    let endpoint = Region::ApSeoul1.query_endpoint();
    let client = OciClient::new(auth, endpoint)?;

    // Example 1: Structured search query
    println!("=== Example 1: Structured Search ===");
    let search_details = SearchDetails::Structured(StructuredSearchDetails {
        query: "query instance resources".to_string(),
        matching_context_type: Some(MatchingContextType::Highlights),
    });

    let request = SearchResourcesRequest::builder(search_details)
        .limit(10)
        .build();

    match client.search_resources(request).await {
        Ok(response) => {
            println!(
                "Found {} resources",
                response.resource_summary_collection.items.len()
            );
            for resource in &response.resource_summary_collection.items {
                println!("  - {}: {}", resource.resource_type, resource.identifier);
                if let Some(display_name) = &resource.display_name {
                    println!("    Name: {}", display_name);
                }
                if let Some(state) = &resource.lifecycle_state {
                    println!("    State: {}", state);
                }
                if let Some(tags) = &resource.freeform_tags {
                    println!("    Tags: {:?}", tags);
                }
            }

            if let Some(next_page) = response.opc_next_page {
                println!("Next page token: {}", next_page);
            }
        }
        Err(e) => {
            eprintln!("Error searching resources: {}", e);
        }
    }

    // Example 2: Free text search
    println!("\n=== Example 2: Free Text Search ===");
    let free_text_search = SearchDetails::FreeText(
        oci_rust_sdk::resource_search::FreeTextSearchDetails {
            text: "production".to_string(),
            matching_context_type: Some(MatchingContextType::None),
        },
    );

    let request = SearchResourcesRequest::builder(free_text_search)
        .limit(5)
        .build();

    match client.search_resources(request).await {
        Ok(response) => {
            println!(
                "Found {} resources with 'production'",
                response.resource_summary_collection.items.len()
            );
            for resource in &response.resource_summary_collection.items {
                println!("  - {} ({})", resource.identifier, resource.resource_type);
            }
        }
        Err(e) => {
            eprintln!("Error searching resources: {}", e);
        }
    }

    Ok(())
}
