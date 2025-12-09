use oci_rust_sdk::{
    core::{auth::ConfigFileAuthProvider, region::Region, ClientConfig},
    virtual_network::{self, Lifetime, ListPublicIpsRequest, ListPublicIpsRequestRequiredFields, Scope},
};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let auth = ConfigFileAuthProvider::from_default()?;

    let client = virtual_network::client(ClientConfig {
        auth_provider: auth,
        region: Region::ApSeoul1,
        timeout: Duration::from_secs(30),
    })?;

    // IMPORTANT: Replace this with your actual compartment OCID
    let compartment_id = std::env::var("OCI_COMPARTMENT_ID")
        .unwrap_or_else(|_| "ocid1.compartment.oc1..aaaaaaaxxxxx".to_string());

    println!("=== Example 1: List Regional Public IPs ===");
    let request = ListPublicIpsRequest::builder(ListPublicIpsRequestRequiredFields {
        scope: Scope::Region,
        compartment_id: compartment_id.clone(),
    })
        .limit(10)
        .build();

    match client.list_public_ips(request).await {
        Ok(response) => {
            println!("Found {} regional public IPs", response.items.len());
            for ip in &response.items {
                println!(
                    "  - {}: {}",
                    ip.display_name.as_deref().unwrap_or("(unnamed)"),
                    ip.ip_address
                );
                println!("    ID: {}", ip.id);
                println!("    State: {:?}", ip.lifecycle_state);
                println!("    Lifetime: {:?}", ip.lifetime);
                if let Some(ref entity_id) = ip.assigned_entity_id {
                    println!("    Assigned to: {}", entity_id);
                    println!("    Entity type: {:?}", ip.assigned_entity_type);
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
            eprintln!("Error listing public IPs: {}", e);
        }
    }

    println!("\n=== Example 2: List Reserved Public IPs Only ===");
    let request = ListPublicIpsRequest::builder(ListPublicIpsRequestRequiredFields {
        scope: Scope::Region,
        compartment_id: compartment_id.clone(),
    })
        .lifetime(Lifetime::Reserved)
        .build();

    match client.list_public_ips(request).await {
        Ok(response) => {
            println!("Found {} reserved public IPs", response.items.len());
            for ip in &response.items {
                println!("  - {}", ip.ip_address);
                println!(
                    "    Name: {}",
                    ip.display_name.as_deref().unwrap_or("(unnamed)")
                );
                if let Some(ref entity_id) = ip.assigned_entity_id {
                    println!("    Assigned to: {}", entity_id);
                } else {
                    println!("    Not assigned");
                }
            }
        }
        Err(e) => {
            eprintln!("Error: {}", e);
        }
    }

    println!("\n=== Example 3: List Ephemeral Public IPs ===");
    let request = ListPublicIpsRequest::builder(ListPublicIpsRequestRequiredFields {
        scope: Scope::Region,
        compartment_id: compartment_id.clone(),
    })
        .lifetime(Lifetime::Ephemeral)
        .limit(5)
        .build();

    match client.list_public_ips(request).await {
        Ok(response) => {
            println!("Found {} ephemeral public IPs", response.items.len());
            for ip in &response.items {
                println!("  - {}", ip.ip_address);
                println!("    Created: {}", ip.time_created);
                if let Some(ref pool_id) = ip.public_ip_pool_id {
                    println!("    Pool: {}", pool_id);
                }
            }
        }
        Err(e) => {
            eprintln!("Error: {}", e);
        }
    }

    println!("\n=== Example 4: Pagination Example ===");
    let mut page_token: Option<String> = None;
    let mut page_number = 1;
    let max_pages = 3;

    loop {
        let mut request_builder =
            ListPublicIpsRequest::builder(ListPublicIpsRequestRequiredFields {
                scope: Scope::Region,
                compartment_id: compartment_id.clone(),
            }).limit(2);

        if let Some(ref token) = page_token {
            request_builder = request_builder.page(token);
        }

        let request = request_builder.build();

        match client.list_public_ips(request).await {
            Ok(response) => {
                println!(
                    "\nPage {}: Found {} public IPs",
                    page_number,
                    response.items.len()
                );
                for ip in &response.items {
                    println!("  - {}", ip.ip_address);
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

    Ok(())
}
