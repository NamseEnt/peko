use oci_rust_sdk::{
    container_instances::{
        self, ContainerInstanceLifecycleState, ListContainerInstancesRequest,
        ListContainerInstancesRequestRequiredFields, SortBy, SortOrder,
    },
    core::{auth::ConfigFileAuthProvider, region::Region, ClientConfig, RetryConfig},
};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let auth = ConfigFileAuthProvider::from_default()?;

    let client = container_instances::client(ClientConfig {
        auth_provider: auth,
        region: Region::ApSeoul1,
        timeout: Duration::from_secs(30),
        retry: RetryConfig::no_retry(),
    })?;

    let compartment_id = std::env::var("OCI_COMPARTMENT_ID")
        .unwrap_or_else(|_| "ocid1.compartment.oc1..aaaaaaaxxxxx".to_string());

    println!("=== Example 1: List All Container Instances ===");
    let request = ListContainerInstancesRequest::builder(
        ListContainerInstancesRequestRequiredFields {
            compartment_id: compartment_id.clone(),
        },
    )
    .limit(10)
    .build();

    match client.list_container_instances(request).await {
        Ok(response) => {
            println!("Found {} container instances", response.items.len());
            for instance in &response.items {
                println!("  - {}: {}", instance.display_name, instance.id);
                println!("    Shape: {}", instance.shape);
                println!("    State: {:?}", instance.lifecycle_state);
                println!("    AD: {}", instance.availability_domain);
                println!("    Containers: {}", instance.container_count);
                println!("    Created: {}", instance.time_created);

                if let Some(ref vnics) = instance.vnics {
                    println!("    VNICs: {}", vnics.len());
                }
            }

            if let Some(next_page) = response.opc_next_page {
                println!("\nMore results available. Next page token: {}", next_page);
            }

            if let Some(request_id) = response.opc_request_id {
                println!("\nRequest ID: {}", request_id);
            }
        }
        Err(e) => {
            eprintln!("Error listing container instances: {}", e);
        }
    }

    println!("\n=== Example 2: List Active Container Instances Only ===");
    let request = ListContainerInstancesRequest::builder(
        ListContainerInstancesRequestRequiredFields {
            compartment_id: compartment_id.clone(),
        },
    )
    .lifecycle_state(ContainerInstanceLifecycleState::Active)
    .build();

    match client.list_container_instances(request).await {
        Ok(response) => {
            println!(
                "Found {} active container instances",
                response.items.len()
            );
            for instance in &response.items {
                println!("  - {}: {}", instance.display_name, instance.shape);
                println!(
                    "    OCPUs: {}, Memory: {} GB",
                    instance.shape_config.ocpus, instance.shape_config.memory_in_gbs
                );
            }
        }
        Err(e) => {
            eprintln!("Error: {}", e);
        }
    }

    println!("\n=== Example 3: List Container Instances Sorted by Display Name ===");
    let request = ListContainerInstancesRequest::builder(
        ListContainerInstancesRequestRequiredFields {
            compartment_id: compartment_id.clone(),
        },
    )
    .sort_by(SortBy::DisplayName)
    .sort_order(SortOrder::Asc)
    .limit(5)
    .build();

    match client.list_container_instances(request).await {
        Ok(response) => {
            println!(
                "Found {} container instances (sorted by name)",
                response.items.len()
            );
            for instance in &response.items {
                println!("  - {}", instance.display_name);
            }
        }
        Err(e) => {
            eprintln!("Error: {}", e);
        }
    }

    println!("\n=== Example 4: Filter by Display Name ===");
    let request = ListContainerInstancesRequest::builder(
        ListContainerInstancesRequestRequiredFields {
            compartment_id: compartment_id.clone(),
        },
    )
    .display_name("my-container-instance")
    .build();

    match client.list_container_instances(request).await {
        Ok(response) => {
            println!(
                "Found {} container instances matching 'my-container-instance'",
                response.items.len()
            );
            for instance in &response.items {
                println!("  - {} ({})", instance.id, instance.lifecycle_state);
            }
        }
        Err(e) => {
            eprintln!("Error: {}", e);
        }
    }

    println!("\n=== Example 5: Check Different Lifecycle States ===");
    let states_to_check = vec![
        ContainerInstanceLifecycleState::Creating,
        ContainerInstanceLifecycleState::Active,
        ContainerInstanceLifecycleState::Inactive,
        ContainerInstanceLifecycleState::Deleting,
    ];

    for state in states_to_check {
        let request = ListContainerInstancesRequest::builder(
            ListContainerInstancesRequestRequiredFields {
                compartment_id: compartment_id.clone(),
            },
        )
        .lifecycle_state(state)
        .limit(5)
        .build();

        match client.list_container_instances(request).await {
            Ok(response) => {
                println!("  {:?} state: {} instances", state, response.items.len());
            }
            Err(e) => {
                eprintln!("Error checking {:?} state: {}", state, e);
            }
        }
    }

    println!("\n=== Example 6: Pagination Example ===");
    let mut page_token: Option<String> = None;
    let mut page_number = 1;
    let max_pages = 3;
    let mut total_instances = 0;

    loop {
        let mut request_builder = ListContainerInstancesRequest::builder(
            ListContainerInstancesRequestRequiredFields {
                compartment_id: compartment_id.clone(),
            },
        )
        .limit(2);

        if let Some(ref token) = page_token {
            request_builder = request_builder.page(token);
        }

        let request = request_builder.build();

        match client.list_container_instances(request).await {
            Ok(response) => {
                println!(
                    "\nPage {}: Found {} container instances",
                    page_number,
                    response.items.len()
                );
                total_instances += response.items.len();

                for instance in &response.items {
                    println!("  - {}", instance.display_name);
                }

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

    println!("\nTotal container instances found: {}", total_instances);

    Ok(())
}
