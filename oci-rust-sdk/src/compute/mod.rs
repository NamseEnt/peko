pub mod models;
pub mod requests;

pub use models::*;
pub use requests::*;

use crate::core::Result;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Trait defining operations for Compute service
pub trait Compute: Send + Sync {
    /// Launch a new compute instance
    fn launch_instance(
        &self,
        request: LaunchInstanceRequest,
    ) -> Pin<Box<dyn Future<Output = Result<LaunchInstanceResponse>> + Send + '_>>;

    /// Launch an instance from an instance configuration
    fn launch_instance_configuration(
        &self,
        request: LaunchInstanceConfigurationRequest,
    ) -> Pin<Box<dyn Future<Output = Result<LaunchInstanceConfigurationResponse>> + Send + '_>>;

    /// Terminate a compute instance
    fn terminate_instance(
        &self,
        request: TerminateInstanceRequest,
    ) -> Pin<Box<dyn Future<Output = Result<TerminateInstanceResponse>> + Send + '_>>;

    /// List compute instances in a compartment
    fn list_instances(
        &self,
        request: ListInstancesRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ListInstancesResponse>> + Send + '_>>;
}

pub fn client<A: crate::core::auth::AuthProvider + 'static>(
    config: crate::core::ClientConfig<A>,
) -> Result<Arc<dyn Compute>> {
    let endpoint = config.region.endpoint("iaas");
    let oci_client = crate::core::OciClient::new(
        Arc::new(config.auth_provider),
        endpoint,
        config.timeout,
        config.retry,
    )?;
    Ok(Arc::new(oci_client))
}

/// Implementation of Compute trait for OciClient
impl Compute for crate::core::OciClient {
    fn launch_instance(
        &self,
        request: LaunchInstanceRequest,
    ) -> Pin<Box<dyn Future<Output = Result<LaunchInstanceResponse>> + Send + '_>> {
        Box::pin(async move {
            // API version 20160918
            let path = "/20160918/instances";

            // Make POST request with LaunchInstanceDetails body
            let oci_response = self
                .post::<LaunchInstanceDetails, Instance>(
                    path,
                    Some(&request.launch_instance_details),
                )
                .await?;

            // Extract request tracking headers
            let opc_request_id = oci_response.get_header("opc-request-id");
            let etag = oci_response.get_header("etag");

            Ok(LaunchInstanceResponse {
                instance: oci_response.body,
                opc_request_id,
                etag,
            })
        })
    }

    fn launch_instance_configuration(
        &self,
        request: LaunchInstanceConfigurationRequest,
    ) -> Pin<Box<dyn Future<Output = Result<LaunchInstanceConfigurationResponse>> + Send + '_>>
    {
        Box::pin(async move {
            // API version 20160918
            let path = format!(
                "/20160918/instanceConfigurations/{}/actions/launch",
                request.instance_configuration_id
            );

            // Make POST request with InstanceConfigurationInstanceDetails body
            let oci_response = self
                .post::<InstanceConfigurationInstanceDetails, Instance>(
                    &path,
                    Some(&request.instance_configuration),
                )
                .await?;

            // Extract request tracking headers
            let opc_request_id = oci_response.get_header("opc-request-id");
            let etag = oci_response.get_header("etag");
            let opc_work_request_id = oci_response.get_header("opc-work-request-id");

            Ok(LaunchInstanceConfigurationResponse {
                instance: oci_response.body,
                opc_request_id,
                etag,
                opc_work_request_id,
            })
        })
    }

    fn terminate_instance(
        &self,
        request: TerminateInstanceRequest,
    ) -> Pin<Box<dyn Future<Output = Result<TerminateInstanceResponse>> + Send + '_>> {
        Box::pin(async move {
            // Build query string from request parameters
            let query_params = request.to_query_params();
            let query_string = if query_params.is_empty() {
                String::new()
            } else {
                format!(
                    "?{}",
                    query_params
                        .iter()
                        .map(|(k, v)| format!(
                            "{}={}",
                            urlencoding::encode(k),
                            urlencoding::encode(v)
                        ))
                        .collect::<Vec<_>>()
                        .join("&")
                )
            };

            // API version 20160918
            let path = format!(
                "/20160918/instances/{}{}",
                request.instance_id, query_string
            );

            // Make DELETE request
            let oci_response = self.delete::<crate::core::EmptyResponse>(&path).await?;

            // Extract request tracking headers
            let opc_request_id = oci_response.get_header("opc-request-id");

            Ok(TerminateInstanceResponse { opc_request_id })
        })
    }

    fn list_instances(
        &self,
        request: ListInstancesRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ListInstancesResponse>> + Send + '_>> {
        Box::pin(async move {
            // Build query string from request parameters
            let query_params = request.to_query_params();
            let query_string = if query_params.is_empty() {
                String::new()
            } else {
                format!(
                    "?{}",
                    query_params
                        .iter()
                        .map(|(k, v)| format!(
                            "{}={}",
                            urlencoding::encode(k),
                            urlencoding::encode(v)
                        ))
                        .collect::<Vec<_>>()
                        .join("&")
                )
            };

            // API version 20160918
            let path = format!("/20160918/instances{}", query_string);

            // Make GET request
            let oci_response = self.get::<Vec<Instance>>(&path).await?;

            // Extract request tracking headers
            let opc_request_id = oci_response.get_header("opc-request-id");
            let opc_next_page = oci_response.get_header("opc-next-page");

            Ok(ListInstancesResponse {
                items: oci_response.body,
                opc_request_id,
                opc_next_page,
            })
        })
    }
}
