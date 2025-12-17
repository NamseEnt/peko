use serde::{Deserialize, Serialize};

pub struct DeleteContainerInstanceRequestRequiredFields {
    pub container_instance_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteContainerInstanceRequest {
    pub container_instance_id: String,

    pub if_match: Option<String>,

    pub opc_request_id: Option<String>,
}

impl DeleteContainerInstanceRequest {
    pub fn builder(
        required: DeleteContainerInstanceRequestRequiredFields,
    ) -> DeleteContainerInstanceRequestBuilder {
        DeleteContainerInstanceRequestBuilder {
            request: DeleteContainerInstanceRequest {
                container_instance_id: required.container_instance_id,
                if_match: None,
                opc_request_id: None,
            },
        }
    }
}

#[derive(Debug)]
pub struct DeleteContainerInstanceRequestBuilder {
    request: DeleteContainerInstanceRequest,
}

impl DeleteContainerInstanceRequestBuilder {
    pub fn if_match(mut self, if_match: impl Into<String>) -> Self {
        self.request.if_match = Some(if_match.into());
        self
    }

    pub fn opc_request_id(mut self, opc_request_id: impl Into<String>) -> Self {
        self.request.opc_request_id = Some(opc_request_id.into());
        self
    }

    pub fn build(self) -> DeleteContainerInstanceRequest {
        self.request
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteContainerInstanceResponse {
    pub opc_work_request_id: Option<String>,

    pub opc_request_id: Option<String>,
}
