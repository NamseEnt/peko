use super::*;
use base64::Engine;
use oci_rust_sdk::compute::*;
use oci_rust_sdk::core::{
    RetryConfig,
    auth::{SimpleAuthProvider, SimpleAuthProviderRequiredFields},
    region::Region,
};
use std::collections::BTreeSet;
use std::{env, future::Future, net::IpAddr, pin::Pin, str::FromStr, sync::Arc};

const DEFAULT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

pub struct OciHostInfra {
    compute: Arc<dyn oci_rust_sdk::compute::Compute>,
    compartment_id: String,
    instance_configuration_id: String,
    availability_domain: String,
}

impl OciHostInfra {
    pub fn new() -> Self {
        let private_key_base64 =
            env::var("OCI_PRIVATE_KEY_BASE64").expect("env var OCI_PRIVATE_KEY_BASE64 is not set");
        let user_id = env::var("OCI_USER_ID").expect("env var OCI_USER_ID is not set");
        let fingerprint = env::var("OCI_FINGERPRINT").expect("env var OCI_FINGERPRINT is not set");
        let tenancy_id = env::var("OCI_TENANCY_ID").expect("env var OCI_TENANCY_ID is not set");
        let region = env::var("OCI_REGION").expect("env var OCI_REGION is not set");

        let compartment_id =
            env::var("OCI_COMPARTMENT_ID").expect("env var OCI_COMPARTMENT_ID is not set");
        let instance_configuration_id = env::var("OCI_INSTANCE_CONFIGURATION_ID")
            .expect("env var OCI_INSTANCE_CONFIGURATION_ID is not set");
        let availability_domain = env::var("OCI_AVAILABILITY_DOMAIN")
            .expect("env var OCI_AVAILABILITY_DOMAIN is not set");

        let private_key = String::from_utf8_lossy(
            &base64::engine::general_purpose::STANDARD
                .decode(private_key_base64)
                .unwrap(),
        )
        .to_string();

        let region = Region::from_str(&region).unwrap_or_else(|_| {
            panic!("invalid region {region}");
        });

        let auth_provider = SimpleAuthProvider::builder(SimpleAuthProviderRequiredFields {
            tenancy: tenancy_id,
            user: user_id,
            fingerprint,
            private_key,
        })
        .region(region)
        .build();

        let compute = oci_rust_sdk::compute::client(oci_rust_sdk::core::ClientConfig {
            auth_provider,
            region,
            timeout: DEFAULT_TIMEOUT,
            retry: RetryConfig::no_retry(),
        })
        .unwrap();

        Self {
            compute,
            compartment_id,
            instance_configuration_id,
            availability_domain,
        }
    }
}

impl HostInfra for OciHostInfra {
    fn sync_host_info_map<'a>(
        &'a self,
        host_info_map: HostInfoMap,
    ) -> Pin<Box<dyn Future<Output = color_eyre::Result<()>> + 'a + Send>> {
        Box::pin(async move {
            let mut page = None;
            let mut listed_host_ids = BTreeSet::new();

            loop {
                let response = self
                    .compute
                    .list_instances(ListInstancesRequest {
                        compartment_id: self.compartment_id.clone(),
                        limit: None,
                        page,
                        availability_domain: None,
                        capacity_reservation_id: None,
                        compute_cluster_id: None,
                        display_name: None,
                        sort_by: None,
                        sort_order: None,
                        lifecycle_state: None,
                    })
                    .await?;

                listed_host_ids.extend(
                    response
                        .items
                        .iter()
                        .map(|instance| HostId::new(instance.id.clone())),
                );

                response.items.into_iter().for_each(|instance| {
                    let host_info = HostInfo {
                        id: HostId::new(instance.id),
                        ip: instance.freeform_tags.and_then(|tags| {
                            let ip = tags.get("public_ip")?;
                            let Ok(ip) = IpAddr::from_str(ip) else {
                                panic!("Failed to parse IP address: {ip}");
                            };
                            Some(ip)
                        }),
                        instance_state: match instance.lifecycle_state {
                            LifecycleState::Provisioning | LifecycleState::Starting => {
                                HostInstanceState::Starting
                            }
                            LifecycleState::Running => HostInstanceState::Running,
                            LifecycleState::Stopping
                            | LifecycleState::Stopped
                            | LifecycleState::Terminating
                            | LifecycleState::Terminated => HostInstanceState::Terminating,
                            LifecycleState::Moving | LifecycleState::CreatingImage => {
                                unreachable!()
                            }
                        },
                        instance_created: instance.time_created,
                    };
                    host_info_map.insert(host_info.id.clone(), host_info);
                });

                if let Some(next_page) = response.opc_next_page {
                    page = Some(next_page);
                } else {
                    break;
                }
            }

            host_info_map.retain(|id, _| listed_host_ids.contains(id));
            Ok(())
        })
    }

    fn terminate<'a>(
        &'a self,
        host_id: &'a crate::HostId,
    ) -> Pin<Box<dyn Future<Output = color_eyre::Result<()>> + 'a + Send>> {
        Box::pin(async move {
            self.compute
                .terminate_instance(TerminateInstanceRequest {
                    instance_id: host_id.to_string(),
                    if_match: None,
                    preserve_boot_volume: Some(false),
                    preserve_data_volumes_created_at_launch: Some(false),
                })
                .await?;
            Ok(())
        })
    }

    fn launch_instance<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = color_eyre::Result<()>> + 'a + Send>> {
        Box::pin(async move {
            self.compute
                .launch_instance_configuration(LaunchInstanceConfigurationRequest {
                    instance_configuration_id: self.instance_configuration_id.clone(),
                    instance_configuration: InstanceConfigurationInstanceDetails::Compute(
                        ComputeInstanceDetails {
                            launch_details: Some(InstanceConfigurationLaunchInstanceDetails {
                                availability_domain: Some(self.availability_domain.clone()),
                                compartment_id: Some(self.compartment_id.clone()),
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                    ),
                    opc_retry_token: None,
                })
                .await?;

            Ok(())
        })
    }
}
