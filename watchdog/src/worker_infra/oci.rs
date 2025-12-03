use super::*;
use base64::Engine;
use oci_rust_sdk::compute::*;
use std::{env, net::IpAddr, str::FromStr, sync::Arc};

pub struct OciWorkerInfra {
    compute: Arc<dyn Compute>,
    compartment_id: String,
}

impl OciWorkerInfra {
    pub fn new() -> Self {
        let private_key_base64 =
            env::var("OCI_PRIVATE_KEY_BASE64").expect("env var OCI_PRIVATE_KEY_BASE64 is not set");
        let user_id = env::var("OCI_USER_ID").expect("env var OCI_USER_ID is not set");
        let fingerprint = env::var("OCI_FINGERPRINT").expect("env var OCI_FINGERPRINT is not set");
        let tenancy_id = env::var("OCI_TENANCY_ID").expect("env var OCI_TENANCY_ID is not set");
        let region = env::var("OCI_REGION").expect("env var OCI_REGION is not set");
        let compartment_id =
            env::var("OCI_COMPARTMENT_ID").expect("env var OCI_COMPARTMENT_ID is not set");

        let private_key = std::str::from_utf8(
            &base64::engine::general_purpose::STANDARD_NO_PAD
                .decode(private_key_base64)
                .unwrap(),
        )
        .unwrap()
        .to_string();

        let region = oci_rust_sdk::core::region::Region::from_str(&region).unwrap_or_else(|_| {
            panic!("invalid region {region}");
        });

        let auth_provider = oci_rust_sdk::core::auth::SimpleAuthProvider::builder()
            .user(user_id)
            .fingerprint(fingerprint)
            .private_key(private_key)
            .tenancy(tenancy_id)
            .region(region)
            .build();

        let compute = oci_rust_sdk::compute::client(auth_provider, region).unwrap();
        Self {
            compute,
            compartment_id,
        }
    }
}

impl WorkerInfra for OciWorkerInfra {
    fn get_worker_infos<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<WorkerInfos>> + 'a + Send>> {
        Box::pin(async move {
            let mut infos = vec![];
            let mut page = None;

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

                infos.extend(response.items.into_iter().map(|instance| WorkerInfo {
                    id: WorkerId(instance.id),
                    ip: instance.freeform_tags.and_then(|tags| {
                        let ip = tags.get("public_ip")?;
                        let Ok(ip) = IpAddr::from_str(ip) else {
                            panic!("Failed to parse IP address: {ip}");
                        };
                        Some(ip)
                    }),
                    instance_state: match instance.lifecycle_state {
                        LifecycleState::Provisioning | LifecycleState::Starting => {
                            WorkerInstanceState::Starting
                        }
                        LifecycleState::Running => WorkerInstanceState::Running,
                        LifecycleState::Stopping
                        | LifecycleState::Stopped
                        | LifecycleState::Terminating
                        | LifecycleState::Terminated => WorkerInstanceState::Terminating,
                        LifecycleState::Moving | LifecycleState::CreatingImage => unreachable!(),
                    },
                    instance_created: instance.time_created,
                }));

                if let Some(next_page) = response.opc_next_page {
                    page = Some(next_page);
                } else {
                    break;
                }
            }
            Ok(infos)
        })
    }

    fn terminate<'a>(
        &'a self,
        worker_id: &'a WorkerId,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + 'a + Send>> {
        Box::pin(async move {
            self.compute
                .terminate_instance(TerminateInstanceRequest {
                    instance_id: worker_id.0.clone(),
                    if_match: None,
                    preserve_boot_volume: Some(false),
                    preserve_data_volumes_created_at_launch: Some(false),
                })
                .await?;
            Ok(())
        })
    }
}
