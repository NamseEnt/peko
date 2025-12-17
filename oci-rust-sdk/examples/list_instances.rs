use oci_rust_sdk::{
    compute::{self, LifecycleState, ListInstancesRequest, ListInstancesRequestRequiredFields, SortBy, SortOrder},
    core::{auth::ConfigFileAuthProvider, region::Region, ClientConfig, RetryConfig},
};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let auth = ConfigFileAuthProvider::from_default()?;

    let client = compute::client(ClientConfig {
        auth_provider: auth,
        region: Region::ApSeoul1,
        timeout: Duration::from_secs(30),
        retry: RetryConfig::no_retry(),
    })?;

    // IMPORTANT: Replace this with your actual compartment OCID
    let compartment_id = std::env::var("OCI_COMPARTMENT_ID")
        .unwrap_or_else(|_| "ocid1.compartment.oc1..aaaaaaaxxxxx".to_string());

    println!("=== Example 1: List All Instances ===");
    let request = ListInstancesRequest::builder(ListInstancesRequestRequiredFields {
        compartment_id: compartment_id.clone(),
    })
        .limit(10)
        .build();

    match client.list_instances(request).await {
        Ok(response) => {
            println!("Found {} instances", response.items.len());
            for instance in &response.items {
                println!(
                    "  - {}: {}",
                    instance.display_name.as_deref().unwrap_or("(unnamed)"),
                    instance.id
                );
                println!("    Shape: {}", instance.shape);
                println!("    State: {:?}", instance.lifecycle_state);
                println!("    AD: {}", instance.availability_domain);
                println!("    Created: {}", instance.time_created);
            }

            if let Some(next_page) = response.opc_next_page {
                println!("\nMore results available. Next page token: {}", next_page);
            }

            if let Some(request_id) = response.opc_request_id {
                println!("\nRequest ID: {}", request_id);
            }
        }
        Err(e) => {
            eprintln!("Error listing instances: {}", e);
        }
    }

    println!("\n=== Example 2: List Running Instances Only ===");
    let request = ListInstancesRequest::builder(ListInstancesRequestRequiredFields {
        compartment_id: compartment_id.clone(),
    })
        .lifecycle_state(LifecycleState::Running)
        .build();

    match client.list_instances(request).await {
        Ok(response) => {
            println!("Found {} running instances", response.items.len());
            for instance in &response.items {
                println!(
                    "  - {}: {}",
                    instance.display_name.as_deref().unwrap_or("(unnamed)"),
                    instance.shape
                );
            }
        }
        Err(e) => {
            eprintln!("Error: {}", e);
        }
    }

    println!("\n=== Example 3: List Instances Sorted by Display Name ===");
    let request = ListInstancesRequest::builder(ListInstancesRequestRequiredFields {
        compartment_id: compartment_id.clone(),
    })
        .sort_by(SortBy::DisplayName)
        .sort_order(SortOrder::Asc)
        .limit(5)
        .build();

    match client.list_instances(request).await {
        Ok(response) => {
            println!("Found {} instances (sorted by name)", response.items.len());
            for instance in &response.items {
                println!(
                    "  - {}",
                    instance.display_name.as_deref().unwrap_or("(unnamed)")
                );
            }
        }
        Err(e) => {
            eprintln!("Error: {}", e);
        }
    }

    println!("\n=== Example 4: Filter by Display Name ===");
    let request = ListInstancesRequest::builder(ListInstancesRequestRequiredFields {
        compartment_id: compartment_id.clone(),
    })
        .display_name("my-instance")
        .build();

    match client.list_instances(request).await {
        Ok(response) => {
            println!("Found {} instances matching 'my-instance'", response.items.len());
            for instance in &response.items {
                println!("  - {} ({})", instance.id, instance.lifecycle_state);
            }
        }
        Err(e) => {
            eprintln!("Error: {}", e);
        }
    }

    println!("\n=== Example 4.5: Filter by Different Lifecycle States ===");
    let states_to_check = vec![
        LifecycleState::Running,
        LifecycleState::Stopped,
        LifecycleState::Terminating,
    ];

    for state in states_to_check {
        let request = ListInstancesRequest::builder(ListInstancesRequestRequiredFields {
            compartment_id: compartment_id.clone(),
        })
            .lifecycle_state(state)
            .limit(5)
            .build();

        match client.list_instances(request).await {
            Ok(response) => {
                println!(
                    "  {} state: {} instances",
                    state,
                    response.items.len()
                );
            }
            Err(e) => {
                eprintln!("Error checking {} state: {}", state, e);
            }
        }
    }

    println!("\n=== Example 5: Pagination Example ===");
    let mut page_token: Option<String> = None;
    let mut page_number = 1;
    let max_pages = 3;
    let mut total_instances = 0;

    loop {
        let mut request_builder = ListInstancesRequest::builder(ListInstancesRequestRequiredFields {
            compartment_id: compartment_id.clone(),
        }).limit(2);

        if let Some(ref token) = page_token {
            request_builder = request_builder.page(token);
        }

        let request = request_builder.build();

        match client.list_instances(request).await {
            Ok(response) => {
                println!(
                    "\nPage {}: Found {} instances",
                    page_number,
                    response.items.len()
                );
                total_instances += response.items.len();

                for instance in &response.items {
                    println!(
                        "  - {}",
                        instance.display_name.as_deref().unwrap_or("(unnamed)")
                    );
                }

                // Check if there are more pages
                if let Some(next) = response.opc_next_page {
                    page_token = Some(next);
                    page_number += 1;

                    if page_number > max_pages {
                        println!(
                            "\nReached maximum pages ({}), stopping pagination demo",
                            max_pages
                        );
                        break;
                    }
                } else {
                    println!("\nNo more pages available");
                    break;
                }
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                break;
            }
        }
    }

    println!("\nTotal instances found: {}", total_instances);

    Ok(())
}
