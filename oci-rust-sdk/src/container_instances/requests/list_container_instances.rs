use crate::container_instances::models::*;
use serde::{Deserialize, Serialize};

pub struct ListContainerInstancesRequestRequiredFields {
    pub compartment_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListContainerInstancesRequest {
    pub compartment_id: String,

    pub lifecycle_state: Option<ContainerInstanceLifecycleState>,

    pub display_name: Option<String>,

    pub availability_domain: Option<String>,

    pub limit: Option<u32>,

    pub page: Option<String>,

    pub sort_order: Option<SortOrder>,

    pub sort_by: Option<SortBy>,
}

impl ListContainerInstancesRequest {
    pub fn builder(
        required: ListContainerInstancesRequestRequiredFields,
    ) -> ListContainerInstancesRequestBuilder {
        ListContainerInstancesRequestBuilder {
            request: ListContainerInstancesRequest {
                compartment_id: required.compartment_id,
                lifecycle_state: None,
                display_name: None,
                availability_domain: None,
                limit: None,
                page: None,
                sort_order: None,
                sort_by: None,
            },
        }
    }

    pub fn to_query_params(&self) -> Vec<(String, String)> {
        let mut params = Vec::new();

        params.push(("compartmentId".to_string(), self.compartment_id.clone()));

        if let Some(v) = self.lifecycle_state {
            params.push(("lifecycleState".to_string(), v.to_string()));
        }
        if let Some(ref v) = self.display_name {
            params.push(("displayName".to_string(), v.clone()));
        }
        if let Some(ref v) = self.availability_domain {
            params.push(("availabilityDomain".to_string(), v.clone()));
        }
        if let Some(v) = self.limit {
            params.push(("limit".to_string(), v.to_string()));
        }
        if let Some(ref v) = self.page {
            params.push(("page".to_string(), v.clone()));
        }
        if let Some(v) = self.sort_order {
            params.push(("sortOrder".to_string(), v.to_string()));
        }
        if let Some(v) = self.sort_by {
            params.push(("sortBy".to_string(), v.to_string()));
        }

        params
    }
}

#[derive(Debug)]
pub struct ListContainerInstancesRequestBuilder {
    request: ListContainerInstancesRequest,
}

impl ListContainerInstancesRequestBuilder {
    pub fn lifecycle_state(
        mut self,
        lifecycle_state: ContainerInstanceLifecycleState,
    ) -> Self {
        self.request.lifecycle_state = Some(lifecycle_state);
        self
    }

    pub fn display_name(mut self, display_name: impl Into<String>) -> Self {
        self.request.display_name = Some(display_name.into());
        self
    }

    pub fn availability_domain(mut self, availability_domain: impl Into<String>) -> Self {
        self.request.availability_domain = Some(availability_domain.into());
        self
    }

    pub fn limit(mut self, limit: u32) -> Self {
        self.request.limit = Some(limit);
        self
    }

    pub fn page(mut self, page: impl Into<String>) -> Self {
        self.request.page = Some(page.into());
        self
    }

    pub fn sort_order(mut self, sort_order: SortOrder) -> Self {
        self.request.sort_order = Some(sort_order);
        self
    }

    pub fn sort_by(mut self, sort_by: SortBy) -> Self {
        self.request.sort_by = Some(sort_by);
        self
    }

    pub fn build(self) -> ListContainerInstancesRequest {
        self.request
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListContainerInstancesResponse {
    pub items: Vec<ContainerInstance>,

    pub opc_request_id: Option<String>,

    pub opc_next_page: Option<String>,
}
