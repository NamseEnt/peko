//! Example demonstrating trait-based service usage
//!
//! This example shows how to use the OsManagementHub trait for
//! dependency injection and testing.

use oci_rust_sdk::core::{auth::ConfigFileAuthProvider, region::Region, ClientConfig, Result};
use oci_rust_sdk::os_management_hub::{
    self, ListManagedInstancesRequest, ListManagedInstancesResponse, ManagedInstanceCollection,
    OsManagementHub,
};
use std::time::Duration;

/// Business logic that depends on OS Management Hub service
///
/// By accepting a trait instead of concrete client, this function
/// can be tested with mocks and used with alternative implementations.
async fn count_running_instances<T: OsManagementHub>(
    service: &T,
    compartment_id: &str,
) -> Result<usize> {
    let request = ListManagedInstancesRequest::builder()
        .compartment_id(compartment_id)
        .build();

    let response = service.list_managed_instances(request).await?;

    Ok(response.managed_instance_collection.items.len())
}

/// Example of a custom mock implementation
///
/// Users can create their own mock implementations for testing
struct MockOsManagementHub {
    instance_count: usize,
}

impl MockOsManagementHub {
    fn new(instance_count: usize) -> Self {
        Self { instance_count }
    }
}

impl OsManagementHub for MockOsManagementHub {
    fn list_managed_instances(
        &self,
        _request: ListManagedInstancesRequest,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ListManagedInstancesResponse>> + Send + '_>,
    > {
        let instance_count = self.instance_count;
        Box::pin(async move {
            // Return mock data with configured instance count
            Ok(ListManagedInstancesResponse {
                managed_instance_collection: ManagedInstanceCollection {
                    items: vec![/* would contain instance_count items */],
                },
                opc_request_id: Some("mock-request-id".to_string()),
                opc_next_page: None,
                opc_total_items: Some(instance_count as u32),
            })
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    println!("=== OCI Rust SDK Trait Usage Example ===\n");

    // Example 1: Using the real client
    println!("Example 1: Real Client (requires OCI credentials)");
    println!("--------------------------------------------------");

    match ConfigFileAuthProvider::from_default() {
        Ok(auth) => {
            let client = os_management_hub::client(ClientConfig {
                auth_provider: auth,
                region: Region::ApSeoul1,
                timeout: Duration::from_secs(30),
            })?;

            // Already returns Arc<dyn OsManagementHub> - ready for dependency injection
            let _service: &dyn OsManagementHub = &*client;

            println!("Client created successfully");
            println!("Note: To actually fetch instances, provide a valid compartment_id\n");

            // Uncomment to run with valid credentials:
            // let count = count_running_instances(service, "ocid1.compartment.oc1..xxxxx").await?;
            // println!("Found {} running instances", count);
        }
        Err(e) => {
            println!(
                "Could not load OCI config (this is expected in CI/CD): {}",
                e
            );
            println!("Skipping real client example\n");
        }
    }

    // Example 2: Using a mock implementation
    println!("Example 2: Mock Implementation");
    println!("--------------------------------");

    let mock = MockOsManagementHub::new(5);
    let count = count_running_instances(&mock, "mock-compartment").await?;
    println!("Mock returned {} instances", count);
    println!("This demonstrates how users can create custom mocks for testing\n");

    // Example 3: Using trait objects for dynamic dispatch
    println!("Example 3: Trait Objects (Dynamic Dispatch)");
    println!("--------------------------------------------");

    let mock = MockOsManagementHub::new(10);
    let service: &dyn OsManagementHub = &mock;

    let request = ListManagedInstancesRequest::builder()
        .compartment_id("test-compartment")
        .limit(5)
        .build();

    let response = service.list_managed_instances(request).await?;
    println!("Response headers:");
    println!("  opc-request-id: {:?}", response.opc_request_id);
    println!("  opc-total-items: {:?}", response.opc_total_items);

    println!("\n=== Examples Complete ===");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test showing how to use mocks in unit tests
    #[tokio::test]
    async fn test_count_instances_with_mock() {
        let mock = MockOsManagementHub::new(3);
        let count = count_running_instances(&mock, "test-compartment")
            .await
            .unwrap();
        assert_eq!(count, 0); // MockOsManagementHub returns empty items vec
    }

    /// Test showing multiple mock configurations
    #[tokio::test]
    async fn test_different_mock_scenarios() {
        // Scenario 1: Empty response
        let empty_mock = MockOsManagementHub::new(0);
        let response = empty_mock
            .list_managed_instances(ListManagedInstancesRequest::builder().build())
            .await
            .unwrap();
        assert_eq!(response.opc_total_items, Some(0));

        // Scenario 2: Multiple instances
        let populated_mock = MockOsManagementHub::new(100);
        let response = populated_mock
            .list_managed_instances(ListManagedInstancesRequest::builder().build())
            .await
            .unwrap();
        assert_eq!(response.opc_total_items, Some(100));
    }
}
