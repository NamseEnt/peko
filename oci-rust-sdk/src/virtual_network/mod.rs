pub mod models;
pub mod requests;

pub use models::*;
pub use requests::*;

use crate::core::Result;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Trait defining operations for Virtual Network (Core) service
pub trait VirtualNetwork: Send + Sync {
    /// List public IPs in a compartment
    fn list_public_ips(
        &self,
        request: ListPublicIpsRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ListPublicIpsResponse>> + Send + '_>>;
}

pub fn client<A: crate::core::auth::AuthProvider + 'static>(
    config: crate::core::ClientConfig<A>,
) -> Result<Arc<dyn VirtualNetwork>> {
    let endpoint = config.region.endpoint("iaas");
    let oci_client = crate::core::OciClient::new(
        Arc::new(config.auth_provider),
        endpoint,
        config.timeout,
    )?;
    Ok(Arc::new(oci_client))
}

/// Implementation of VirtualNetwork trait for OciClient
impl VirtualNetwork for crate::core::OciClient {
    fn list_public_ips(
        &self,
        request: ListPublicIpsRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ListPublicIpsResponse>> + Send + '_>> {
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
            let path = format!("/20160918/publicIps{}", query_string);

            // Make GET request - API returns Vec<PublicIp> directly
            let oci_response = self.get::<Vec<PublicIp>>(&path).await?;

            // Extract pagination and request tracking headers
            let opc_request_id = oci_response.get_header("opc-request-id");
            let opc_next_page = oci_response.get_header("opc-next-page");

            Ok(ListPublicIpsResponse {
                items: oci_response.body,
                opc_request_id,
                opc_next_page,
            })
        })
    }
}
