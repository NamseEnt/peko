use super::*;
use crate::args::OciContainerInstanceHostProviderArgs;
use base64::Engine;
use oci_rust_sdk::container_instances::*;
use oci_rust_sdk::core::{
    RetryConfig,
    auth::{SimpleAuthProvider, SimpleAuthProviderRequiredFields},
    region::Region,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::{future::Future, net::IpAddr, pin::Pin, str::FromStr, sync::Arc};

const DEFAULT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

pub struct OciContainerInstanceHostProvider {
    container_instance_client: Arc<dyn oci_rust_sdk::container_instances::ContainerInstance>,
    compartment_id: String,
    availability_domain: String,
    shape: String,
    ocpus: f32,
    memory_in_gbs: f32,
    subnet_id: String,
    image: String,
    envs: BTreeMap<String, String>,
}

impl OciContainerInstanceHostProvider {
    pub fn new(args: OciContainerInstanceHostProviderArgs) -> Result<Self> {
        let region = Region::from_str(&args.region)?;

        let auth_provider = SimpleAuthProvider::builder(SimpleAuthProviderRequiredFields {
            tenancy: args.tenancy_id,
            user: args.user_id,
            fingerprint: args.fingerprint,
            private_key: String::from_utf8_lossy(
                &base64::engine::general_purpose::STANDARD
                    .decode(args.private_key_base64)
                    .unwrap(),
            )
            .to_string(),
        })
        .region(region)
        .build();

        let container_instance_client =
            oci_rust_sdk::container_instances::client(oci_rust_sdk::core::ClientConfig {
                auth_provider,
                region,
                timeout: DEFAULT_TIMEOUT,
                retry: RetryConfig::no_retry(),
            })
            .unwrap();

        Ok(Self {
            container_instance_client,
            compartment_id: args.compartment_id,
            availability_domain: args.availability_domain,
            shape: args.shape,
            ocpus: args.ocpus,
            memory_in_gbs: args.memory_in_gbs,
            subnet_id: args.subnet_id,
            image: args.image,
            envs: args.envs,
        })
    }
}

impl HostProvider for OciContainerInstanceHostProvider {
    fn sync_host_info_map<'a>(
        &'a self,
        host_info_map: HostInfoMap,
    ) -> Pin<Box<dyn Future<Output = color_eyre::Result<()>> + 'a + Send>> {
        Box::pin(async move {
            let mut page = None;
            let mut listed_host_ids = BTreeSet::new();

            loop {
                let response = self
                    .container_instance_client
                    .list_container_instances(ListContainerInstancesRequest {
                        compartment_id: self.compartment_id.clone(),
                        limit: None,
                        page,
                        availability_domain: None,
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
                    let ip = instance.vnics.and_then(|vnics| {
                        vnics.first().and_then(|vnic| {
                            instance.freeform_tags.as_ref().and_then(|tags| {
                                let ip_str = tags.get("public_ip")?;
                                IpAddr::from_str(ip_str).ok()
                            })
                        })
                    });

                    let host_info = HostInfo {
                        id: HostId::new(instance.id),
                        ip,
                        instance_state: match instance.lifecycle_state {
                            ContainerInstanceLifecycleState::Creating
                            | ContainerInstanceLifecycleState::Updating => {
                                HostInstanceState::Starting
                            }
                            ContainerInstanceLifecycleState::Active => HostInstanceState::Running,
                            ContainerInstanceLifecycleState::Inactive
                            | ContainerInstanceLifecycleState::Deleting
                            | ContainerInstanceLifecycleState::Deleted
                            | ContainerInstanceLifecycleState::Failed => {
                                HostInstanceState::Terminating
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
            self.container_instance_client
                .delete_container_instance(DeleteContainerInstanceRequest {
                    container_instance_id: host_id.to_string(),
                    if_match: None,
                    opc_request_id: None,
                })
                .await?;
            Ok(())
        })
    }

    fn launch_instance<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = color_eyre::Result<()>> + 'a + Send>> {
        Box::pin(async move {
            let environment_variables: HashMap<String, String> = self
                .envs
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();

            let create_details = CreateContainerInstanceDetails {
                compartment_id: self.compartment_id.clone(),
                availability_domain: self.availability_domain.clone(),
                shape: self.shape.clone(),
                shape_config: CreateContainerInstanceShapeConfigDetails {
                    ocpus: self.ocpus,
                    memory_in_gbs: self.memory_in_gbs,
                },
                containers: vec![CreateContainerDetails {
                    image_url: self.image.clone(),
                    display_name: Some("main".to_string()),
                    command: None,
                    arguments: None,
                    environment_variables: Some(environment_variables),
                    resource_config: None,
                }],
                vnics: vec![CreateContainerVnicDetails {
                    subnet_id: self.subnet_id.clone(),
                    display_name: None,
                    hostname_label: None,
                    is_public_ip_assigned: Some(true),
                    skip_source_dest_check: None,
                    nsg_ids: None,
                    private_ip: None,
                    freeform_tags: None,
                    defined_tags: None,
                }],
                display_name: None,
                fault_domain: None,
                graceful_shutdown_timeout_in_seconds: None,
                container_restart_policy: Some("ALWAYS".to_string()),
                freeform_tags: None,
                defined_tags: None,
            };

            self.container_instance_client
                .create_container_instance(CreateContainerInstanceRequest {
                    create_container_instance_details: create_details,
                    opc_retry_token: None,
                    opc_request_id: None,
                })
                .await?;

            Ok(())
        })
    }
}
