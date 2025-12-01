use crate::virtual_network::models::*;

/// Request to list public IPs
#[derive(Debug, Clone)]
pub struct ListPublicIpsRequest {
    /// Required: REGION or AVAILABILITY_DOMAIN
    pub scope: Scope,

    /// Required: The OCID of the compartment
    pub compartment_id: String,

    /// For list pagination - maximum number of results per page
    pub limit: Option<u32>,

    /// For list pagination - the value of opc-next-page from previous response
    pub page: Option<String>,

    /// The name of the availability domain (required if scope is AVAILABILITY_DOMAIN)
    pub availability_domain: Option<String>,

    /// Filter by lifetime: EPHEMERAL or RESERVED
    pub lifetime: Option<Lifetime>,

    /// The OCID of the public IP pool
    pub public_ip_pool_id: Option<String>,
}

impl ListPublicIpsRequest {
    /// Create a new builder for ListPublicIpsRequest
    pub fn builder(scope: Scope, compartment_id: impl Into<String>) -> ListPublicIpsRequestBuilder {
        ListPublicIpsRequestBuilder {
            request: ListPublicIpsRequest {
                scope,
                compartment_id: compartment_id.into(),
                limit: None,
                page: None,
                availability_domain: None,
                lifetime: None,
                public_ip_pool_id: None,
            },
        }
    }

    /// Convert request to query parameters
    pub fn to_query_params(&self) -> Vec<(String, String)> {
        let mut params = Vec::new();

        // Required parameters
        params.push(("scope".to_string(), format!("{:?}", self.scope).to_uppercase()));
        params.push(("compartmentId".to_string(), self.compartment_id.clone()));

        // Optional parameters
        if let Some(v) = self.limit {
            params.push(("limit".to_string(), v.to_string()));
        }
        if let Some(ref v) = self.page {
            params.push(("page".to_string(), v.clone()));
        }
        if let Some(ref v) = self.availability_domain {
            params.push(("availabilityDomain".to_string(), v.clone()));
        }
        if let Some(v) = self.lifetime {
            params.push(("lifetime".to_string(), format!("{:?}", v).to_uppercase()));
        }
        if let Some(ref v) = self.public_ip_pool_id {
            params.push(("publicIpPoolId".to_string(), v.clone()));
        }

        params
    }
}

/// Builder for ListPublicIpsRequest
#[derive(Debug)]
pub struct ListPublicIpsRequestBuilder {
    request: ListPublicIpsRequest,
}

impl ListPublicIpsRequestBuilder {
    pub fn limit(mut self, limit: u32) -> Self {
        self.request.limit = Some(limit);
        self
    }

    pub fn page(mut self, page: impl Into<String>) -> Self {
        self.request.page = Some(page.into());
        self
    }

    pub fn availability_domain(mut self, availability_domain: impl Into<String>) -> Self {
        self.request.availability_domain = Some(availability_domain.into());
        self
    }

    pub fn lifetime(mut self, lifetime: Lifetime) -> Self {
        self.request.lifetime = Some(lifetime);
        self
    }

    pub fn public_ip_pool_id(mut self, public_ip_pool_id: impl Into<String>) -> Self {
        self.request.public_ip_pool_id = Some(public_ip_pool_id.into());
        self
    }

    pub fn build(self) -> ListPublicIpsRequest {
        self.request
    }
}

/// Response from listing public IPs
#[derive(Debug, Clone)]
pub struct ListPublicIpsResponse {
    /// The list of public IPs
    pub items: Vec<PublicIp>,

    /// Unique Oracle-assigned identifier for the request
    pub opc_request_id: Option<String>,

    /// For list pagination - token for next page
    pub opc_next_page: Option<String>,
}
