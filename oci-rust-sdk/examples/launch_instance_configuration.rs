use oci_rust_sdk::compute::{self, models::*, requests::*};
use oci_rust_sdk::core::{auth::ConfigFileAuthProvider, region::Region, ClientConfig};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let auth = ConfigFileAuthProvider::from_default()?;

    let client = compute::client(ClientConfig {
        auth_provider: auth,
        region: Region::ApSeoul1,
        timeout: Duration::from_secs(30),
    })?;

    // Set your instance configuration OCID
    let instance_configuration_id = "ocid1.instanceconfiguration.oc1..xxxxx".to_string();

    // Create launch instance details
    let launch_details = InstanceConfigurationLaunchInstanceDetails::new()
        .with_availability_domain("AD-1")
        .with_compartment_id("ocid1.compartment.oc1..xxxxx")
        .with_shape("VM.Standard.E4.Flex")
        .with_display_name("my-instance-from-config");

    // Create compute instance details with launch configuration
    let compute_details = ComputeInstanceDetails::new()
        .with_launch_details(launch_details);

    // Wrap in instance configuration details enum
    let instance_configuration = InstanceConfigurationInstanceDetails::compute(compute_details);

    // Build the request
    let request = LaunchInstanceConfigurationRequest::builder(
        LaunchInstanceConfigurationRequestRequiredFields {
            instance_configuration_id,
            instance_configuration,
        },
    )
    .build();

    // Launch the instance
    println!("Launching instance from configuration...");
    let response = client.launch_instance_configuration(request).await?;

    println!("Instance launched successfully!");
    println!("Instance ID: {}", response.instance.id);
    println!("Display Name: {:?}", response.instance.display_name);
    println!("Lifecycle State: {:?}", response.instance.lifecycle_state);

    if let Some(work_request_id) = response.opc_work_request_id {
        println!("Work Request ID: {}", work_request_id);
        println!("Use GetWorkRequest to track the status of this operation.");
    }

    Ok(())
}
