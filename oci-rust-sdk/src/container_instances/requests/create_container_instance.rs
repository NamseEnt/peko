use crate::container_instances::models::*;
use serde::{Deserialize, Serialize};

pub struct CreateContainerInstanceRequestRequiredFields {
    pub create_container_instance_details: CreateContainerInstanceDetails,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateContainerInstanceRequest {
    pub create_container_instance_details: CreateContainerInstanceDetails,

    pub opc_retry_token: Option<String>,

    pub opc_request_id: Option<String>,
}

impl CreateContainerInstanceRequest {
    pub fn builder(
        required: CreateContainerInstanceRequestRequiredFields,
    ) -> CreateContainerInstanceRequestBuilder {
        CreateContainerInstanceRequestBuilder {
            request: CreateContainerInstanceRequest {
                create_container_instance_details: required.create_container_instance_details,
                opc_retry_token: None,
                opc_request_id: None,
            },
        }
    }
}

#[derive(Debug)]
pub struct CreateContainerInstanceRequestBuilder {
    request: CreateContainerInstanceRequest,
}

impl CreateContainerInstanceRequestBuilder {
    pub fn opc_retry_token(mut self, opc_retry_token: impl Into<String>) -> Self {
        self.request.opc_retry_token = Some(opc_retry_token.into());
        self
    }

    pub fn opc_request_id(mut self, opc_request_id: impl Into<String>) -> Self {
        self.request.opc_request_id = Some(opc_request_id.into());
        self
    }

    pub fn build(self) -> CreateContainerInstanceRequest {
        self.request
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateContainerInstanceResponse {
    pub container_instance: ContainerInstance,

    pub etag: Option<String>,

    pub opc_work_request_id: Option<String>,

    pub opc_request_id: Option<String>,
}
