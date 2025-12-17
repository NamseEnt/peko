use oci_rust_sdk::{
    container_instances::{
        self, ContainerInstanceLifecycleState, CreateContainerDetails,
        CreateContainerInstanceDetails, CreateContainerInstanceRequest,
        CreateContainerInstanceRequestRequiredFields, CreateContainerInstanceShapeConfigDetails,
        CreateContainerVnicDetails, DeleteContainerInstanceRequest,
        DeleteContainerInstanceRequestRequiredFields, ListContainerInstancesRequest,
        ListContainerInstancesRequestRequiredFields,
    },
    core::{auth::ConfigFileAuthProvider, region::Region, ClientConfig, RetryConfig},
};
use std::collections::HashMap;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== OCI Container Instance Lifecycle Example ===\n");

    let auth = ConfigFileAuthProvider::from_default()?;

    let client = container_instances::client(ClientConfig {
        auth_provider: auth,
        region: Region::ApSeoul1,
        timeout: Duration::from_secs(60),
        retry: RetryConfig::no_retry(),
    })?;

    let compartment_id = std::env::var("OCI_COMPARTMENT_ID")
        .expect("OCI_COMPARTMENT_ID environment variable must be set");
    let subnet_id = std::env::var("OCI_SUBNET_ID")
        .expect("OCI_SUBNET_ID environment variable must be set");
    let availability_domain = std::env::var("OCI_AVAILABILITY_DOMAIN")
        .expect("OCI_AVAILABILITY_DOMAIN environment variable must be set");

    println!("Configuration:");
    println!("  Compartment ID: {}", compartment_id);
    println!("  Subnet ID: {}", subnet_id);
    println!("  Availability Domain: {}\n", availability_domain);

    println!("=== Step 1: Create Container Instance ===");

    let mut env_vars = HashMap::new();
    env_vars.insert("EXAMPLE_VAR".to_string(), "example_value".to_string());
    env_vars.insert("ENVIRONMENT".to_string(), "test".to_string());

    let create_details = CreateContainerInstanceDetails {
        compartment_id: compartment_id.clone(),
        availability_domain: availability_domain.clone(),
        shape: "CI.Standard.E4.Flex".to_string(),
        shape_config: CreateContainerInstanceShapeConfigDetails {
            ocpus: 1.0,
            memory_in_gbs: 4.0,
        },
        containers: vec![CreateContainerDetails {
            image_url: "nginx:latest".to_string(),
            display_name: Some("nginx-container".to_string()),
            command: None,
            arguments: None,
            environment_variables: Some(env_vars),
            resource_config: None,
        }],
        vnics: vec![CreateContainerVnicDetails {
            subnet_id: subnet_id.clone(),
            display_name: Some("primary-vnic".to_string()),
            hostname_label: Some("nginx-host".to_string()),
            is_public_ip_assigned: Some(true),
            skip_source_dest_check: None,
            nsg_ids: None,
            private_ip: None,
            freeform_tags: None,
            defined_tags: None,
        }],
        display_name: Some("example-container-instance".to_string()),
        fault_domain: None,
        graceful_shutdown_timeout_in_seconds: Some(30),
        container_restart_policy: Some("ALWAYS".to_string()),
        freeform_tags: None,
        defined_tags: None,
    };

    let create_request = CreateContainerInstanceRequest::builder(
        CreateContainerInstanceRequestRequiredFields {
            create_container_instance_details: create_details,
        },
    )
    .build();

    let container_instance_id = match client.create_container_instance(create_request).await {
        Ok(response) => {
            println!("✓ Container instance created successfully!");
            println!("  ID: {}", response.container_instance.id);
            println!("  Display Name: {}", response.container_instance.display_name);
            println!(
                "  State: {:?}",
                response.container_instance.lifecycle_state
            );

            if let Some(work_request_id) = response.opc_work_request_id {
                println!("  Work Request ID: {}", work_request_id);
            }

            response.container_instance.id
        }
        Err(e) => {
            eprintln!("✗ Error creating container instance: {}", e);
            return Err(e.into());
        }
    };

    println!("\n=== Step 2: Wait for Container Instance to Become Active ===");
    println!("Polling container instance status...");

    let mut poll_count = 0;
    let max_polls = 60;

    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        poll_count += 1;

        let list_request = ListContainerInstancesRequest::builder(
            ListContainerInstancesRequestRequiredFields {
                compartment_id: compartment_id.clone(),
            },
        )
        .display_name("example-container-instance")
        .build();

        match client.list_container_instances(list_request).await {
            Ok(response) => {
                if let Some(instance) = response.items.first() {
                    println!(
                        "  Poll {}: State = {:?}",
                        poll_count, instance.lifecycle_state
                    );

                    match instance.lifecycle_state {
                        ContainerInstanceLifecycleState::Active => {
                            println!("\n✓ Container instance is now ACTIVE!");
                            if let Some(ref vnics) = instance.vnics {
                                println!("  VNICs: {}", vnics.len());
                            }
                            break;
                        }
                        ContainerInstanceLifecycleState::Failed => {
                            eprintln!("\n✗ Container instance creation FAILED!");
                            if let Some(ref details) = instance.lifecycle_details {
                                eprintln!("  Details: {}", details);
                            }
                            return Err("Container instance creation failed".into());
                        }
                        _ => {
                            if poll_count >= max_polls {
                                eprintln!("\n✗ Timeout waiting for container instance to become active");
                                return Err("Timeout".into());
                            }
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("Error polling status: {}", e);
            }
        }
    }

    println!("\n=== Step 3: List All Container Instances ===");
    let list_request = ListContainerInstancesRequest::builder(
        ListContainerInstancesRequestRequiredFields {
            compartment_id: compartment_id.clone(),
        },
    )
    .build();

    match client.list_container_instances(list_request).await {
        Ok(response) => {
            println!("Total container instances: {}", response.items.len());
            for instance in &response.items {
                println!("  - {} ({})", instance.display_name, instance.id);
                println!("    State: {:?}", instance.lifecycle_state);
            }
        }
        Err(e) => {
            eprintln!("Error listing: {}", e);
        }
    }

    println!("\n=== Step 4: Delete Container Instance ===");
    println!(
        "Deleting container instance: {}",
        container_instance_id
    );

    let delete_request = DeleteContainerInstanceRequest::builder(
        DeleteContainerInstanceRequestRequiredFields {
            container_instance_id: container_instance_id.clone(),
        },
    )
    .build();

    match client.delete_container_instance(delete_request).await {
        Ok(response) => {
            println!("✓ Delete request sent successfully!");
            if let Some(work_request_id) = response.opc_work_request_id {
                println!("  Work Request ID: {}", work_request_id);
            }
        }
        Err(e) => {
            eprintln!("✗ Error deleting container instance: {}", e);
            return Err(e.into());
        }
    }

    println!("\n=== Step 5: Wait for Container Instance to be Deleted ===");
    poll_count = 0;

    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        poll_count += 1;

        let list_request = ListContainerInstancesRequest::builder(
            ListContainerInstancesRequestRequiredFields {
                compartment_id: compartment_id.clone(),
            },
        )
        .display_name("example-container-instance")
        .build();

        match client.list_container_instances(list_request).await {
            Ok(response) => {
                if let Some(instance) = response
                    .items
                    .iter()
                    .find(|i| i.id == container_instance_id)
                {
                    println!(
                        "  Poll {}: State = {:?}",
                        poll_count, instance.lifecycle_state
                    );

                    if instance.lifecycle_state == ContainerInstanceLifecycleState::Deleted {
                        println!("\n✓ Container instance has been DELETED!");
                        break;
                    }
                } else {
                    println!("\n✓ Container instance no longer appears in list (deleted)!");
                    break;
                }

                if poll_count >= max_polls {
                    eprintln!("\n✗ Timeout waiting for deletion");
                    return Err("Timeout".into());
                }
            }
            Err(e) => {
                eprintln!("Error polling status: {}", e);
            }
        }
    }

    println!("\n=== Lifecycle Complete ===");
    println!("✓ Successfully created and deleted a container instance!");

    Ok(())
}
