use oci_rust_sdk::{
    container_instances::{
        self, CreateContainerDetails, CreateContainerInstanceDetails,
        CreateContainerInstanceRequest, CreateContainerInstanceRequestRequiredFields,
        CreateContainerInstanceShapeConfigDetails, CreateContainerVnicDetails,
    },
    core::{auth::ConfigFileAuthProvider, region::Region, ClientConfig, RetryConfig},
};
use std::collections::HashMap;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let auth = ConfigFileAuthProvider::from_default()?;

    let client = container_instances::client(ClientConfig {
        auth_provider: auth,
        region: Region::ApSeoul1,
        timeout: Duration::from_secs(60),
        retry: RetryConfig::no_retry(),
    })?;

    let compartment_id =
        std::env::var("OCI_COMPARTMENT_ID").expect("OCI_COMPARTMENT_ID must be set");
    let subnet_id = std::env::var("OCI_SUBNET_ID").expect("OCI_SUBNET_ID must be set");
    let availability_domain =
        std::env::var("OCI_AVAILABILITY_DOMAIN").expect("OCI_AVAILABILITY_DOMAIN must be set");

    println!("Creating a simple nginx container instance...\n");

    let mut env_vars = HashMap::new();
    env_vars.insert("APP_NAME".to_string(), "my-app".to_string());

    let create_details = CreateContainerInstanceDetails {
        compartment_id,
        availability_domain,
        shape: "CI.Standard.E4.Flex".to_string(),
        shape_config: CreateContainerInstanceShapeConfigDetails {
            ocpus: 1.0,
            memory_in_gbs: 4.0,
        },
        containers: vec![CreateContainerDetails {
            image_url: "nginx:alpine".to_string(),
            display_name: Some("nginx".to_string()),
            command: None,
            arguments: None,
            environment_variables: Some(env_vars),
            resource_config: None,
        }],
        vnics: vec![CreateContainerVnicDetails {
            subnet_id,
            display_name: Some("main-vnic".to_string()),
            hostname_label: None,
            is_public_ip_assigned: Some(true),
            skip_source_dest_check: None,
            nsg_ids: None,
            private_ip: None,
            freeform_tags: None,
            defined_tags: None,
        }],
        display_name: Some("my-nginx-instance".to_string()),
        fault_domain: None,
        graceful_shutdown_timeout_in_seconds: Some(30),
        container_restart_policy: Some("ALWAYS".to_string()),
        freeform_tags: None,
        defined_tags: None,
    };

    let request = CreateContainerInstanceRequest::builder(
        CreateContainerInstanceRequestRequiredFields {
            create_container_instance_details: create_details,
        },
    )
    .build();

    match client.create_container_instance(request).await {
        Ok(response) => {
            println!("✓ Container instance created successfully!");
            println!("\nDetails:");
            println!("  ID: {}", response.container_instance.id);
            println!(
                "  Display Name: {}",
                response.container_instance.display_name
            );
            println!(
                "  State: {:?}",
                response.container_instance.lifecycle_state
            );
            println!("  Shape: {}", response.container_instance.shape);
            println!(
                "  OCPUs: {}",
                response.container_instance.shape_config.ocpus
            );
            println!(
                "  Memory: {} GB",
                response.container_instance.shape_config.memory_in_gbs
            );
            println!(
                "  Containers: {}",
                response.container_instance.container_count
            );

            if let Some(work_request_id) = response.opc_work_request_id {
                println!("\n  Work Request ID: {}", work_request_id);
                println!("  You can monitor the work request to track the creation progress.");
            }

            println!("\n✓ Done! The container instance is being created.");
            println!("  Use the OCI Console or CLI to monitor its progress.");
        }
        Err(e) => {
            eprintln!("✗ Error creating container instance: {}", e);
            return Err(e.into());
        }
    }

    Ok(())
}
