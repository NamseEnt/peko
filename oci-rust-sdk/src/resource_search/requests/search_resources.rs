use crate::resource_search::models::{ResourceSummaryCollection, SearchDetails};

/// Request to search for resources.
#[derive(Debug, Clone)]
pub struct SearchResourcesRequest {
    /// The search criteria (structured query or free text).
    pub search_details: SearchDetails,

    /// The maximum number of items to return. The value must be between 1 and 1000.
    pub limit: Option<u32>,

    /// The page token representing the page at which to start retrieving results.
    pub page: Option<String>,

    /// The tenancy ID for cross-tenancy searches. If not specified, searches within
    /// the tenancy of the authenticated user.
    pub tenant_id: Option<String>,

    /// Unique Oracle-assigned identifier for the request. If you need to contact Oracle about
    /// a particular request, please provide the request ID.
    pub opc_request_id: Option<String>,
}

impl SearchResourcesRequest {
    /// Create a new SearchResourcesRequestBuilder.
    pub fn builder(search_details: SearchDetails) -> SearchResourcesRequestBuilder {
        SearchResourcesRequestBuilder {
            search_details,
            limit: None,
            page: None,
            tenant_id: None,
            opc_request_id: None,
        }
    }

    /// Convert this request's query parameters to a vector of key-value pairs.
    pub fn to_query_params(&self) -> Vec<(String, String)> {
        let mut params = Vec::new();

        if let Some(limit) = self.limit {
            params.push(("limit".to_string(), limit.to_string()));
        }

        if let Some(ref page) = self.page {
            params.push(("page".to_string(), page.clone()));
        }

        if let Some(ref tenant_id) = self.tenant_id {
            params.push(("tenantId".to_string(), tenant_id.clone()));
        }

        params
    }
}

/// Builder for SearchResourcesRequest.
#[derive(Debug, Clone)]
pub struct SearchResourcesRequestBuilder {
    search_details: SearchDetails,
    limit: Option<u32>,
    page: Option<String>,
    tenant_id: Option<String>,
    opc_request_id: Option<String>,
}

impl SearchResourcesRequestBuilder {
    /// Set the maximum number of items to return (1-1000).
    pub fn limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Set the page token for pagination.
    pub fn page(mut self, page: impl Into<String>) -> Self {
        self.page = Some(page.into());
        self
    }

    /// Set the tenant ID for cross-tenancy searches.
    pub fn tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }

    /// Set the OPC request ID.
    pub fn opc_request_id(mut self, opc_request_id: impl Into<String>) -> Self {
        self.opc_request_id = Some(opc_request_id.into());
        self
    }

    /// Build the SearchResourcesRequest.
    pub fn build(self) -> SearchResourcesRequest {
        SearchResourcesRequest {
            search_details: self.search_details,
            limit: self.limit,
            page: self.page,
            tenant_id: self.tenant_id,
            opc_request_id: self.opc_request_id,
        }
    }
}

/// Response from a search resources request.
#[derive(Debug, Clone)]
pub struct SearchResourcesResponse {
    /// The collection of resource summaries matching the search criteria.
    pub resource_summary_collection: ResourceSummaryCollection,

    /// Unique Oracle-assigned identifier for the request.
    pub opc_request_id: Option<String>,

    /// For pagination of a list of items. When paging through a list, if this header appears
    /// in the response, then a partial list might have been returned. Include this value as
    /// the page parameter for the subsequent GET request to get the next batch of items.
    pub opc_next_page: Option<String>,

    /// For pagination of a list of items. When paging through a list, if this header appears
    /// in the response, then a partial list might have been returned. Include this value as
    /// the page parameter for the subsequent GET request to get the previous batch of items.
    pub opc_previous_page: Option<String>,
}
