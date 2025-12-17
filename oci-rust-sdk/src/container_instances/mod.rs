pub mod models;
pub mod requests;

pub use models::*;
pub use requests::*;

use crate::core::Result;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub trait ContainerInstance: Send + Sync {
    fn list_container_instances(
        &self,
        request: ListContainerInstancesRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ListContainerInstancesResponse>> + Send + '_>>;

    fn create_container_instance(
        &self,
        request: CreateContainerInstanceRequest,
    ) -> Pin<Box<dyn Future<Output = Result<CreateContainerInstanceResponse>> + Send + '_>>;

    fn delete_container_instance(
        &self,
        request: DeleteContainerInstanceRequest,
    ) -> Pin<Box<dyn Future<Output = Result<DeleteContainerInstanceResponse>> + Send + '_>>;
}

pub fn client<A: crate::core::auth::AuthProvider + 'static>(
    config: crate::core::ClientConfig<A>,
) -> Result<Arc<dyn ContainerInstance>> {
    let endpoint = config.region.endpoint("compute-containers");
    let oci_client = crate::core::OciClient::new(
        Arc::new(config.auth_provider),
        endpoint,
        config.timeout,
        config.retry,
    )?;
    Ok(Arc::new(oci_client))
}

impl ContainerInstance for crate::core::OciClient {
    fn list_container_instances(
        &self,
        request: ListContainerInstancesRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ListContainerInstancesResponse>> + Send + '_>> {
        Box::pin(async move {
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

            let path = format!("/20210415/containerInstances{}", query_string);

            let oci_response = self.get::<Vec<models::ContainerInstance>>(&path).await?;

            let opc_request_id = oci_response.get_header("opc-request-id");
            let opc_next_page = oci_response.get_header("opc-next-page");

            Ok(ListContainerInstancesResponse {
                items: oci_response.body,
                opc_request_id,
                opc_next_page,
            })
        })
    }

    fn create_container_instance(
        &self,
        request: CreateContainerInstanceRequest,
    ) -> Pin<Box<dyn Future<Output = Result<CreateContainerInstanceResponse>> + Send + '_>> {
        Box::pin(async move {
            let path = "/20210415/containerInstances";

            let oci_response = self
                .post::<CreateContainerInstanceDetails, models::ContainerInstance>(
                    path,
                    Some(&request.create_container_instance_details),
                )
                .await?;

            let opc_request_id = oci_response.get_header("opc-request-id");
            let etag = oci_response.get_header("etag");
            let opc_work_request_id = oci_response.get_header("opc-work-request-id");

            Ok(CreateContainerInstanceResponse {
                container_instance: oci_response.body,
                opc_request_id,
                etag,
                opc_work_request_id,
            })
        })
    }

    fn delete_container_instance(
        &self,
        request: DeleteContainerInstanceRequest,
    ) -> Pin<Box<dyn Future<Output = Result<DeleteContainerInstanceResponse>> + Send + '_>> {
        Box::pin(async move {
            let path = format!(
                "/20210415/containerInstances/{}",
                request.container_instance_id
            );

            let oci_response = self.delete::<crate::core::EmptyResponse>(&path).await?;

            let opc_request_id = oci_response.get_header("opc-request-id");
            let opc_work_request_id = oci_response.get_header("opc-work-request-id");

            Ok(DeleteContainerInstanceResponse {
                opc_request_id,
                opc_work_request_id,
            })
        })
    }
}
