use crate::os_management_hub::models::*;

/// Request to list managed instances
#[derive(Debug, Default, Clone)]
pub struct ListManagedInstancesRequest {
    /// The OCID of the compartment that contains the resources to list
    pub compartment_id: Option<String>,

    /// A filter to return resources that match the given display names
    pub display_name: Option<Vec<String>>,

    /// A filter to return resources that may partially match the given display name
    pub display_name_contains: Option<String>,

    /// The OCID of the managed instance
    pub managed_instance_id: Option<String>,

    /// A filter to return only managed instances whose status matches the status provided
    pub status: Option<Vec<ManagedInstanceStatus>>,

    /// A filter to return only instances whose architecture type matches the given architecture
    pub arch_type: Option<Vec<ArchType>>,

    /// A filter to return only resources that match the given operating system family
    pub os_family: Option<Vec<OsFamily>>,

    /// A filter to return only managed instances that are acting as management stations
    pub is_management_station: Option<bool>,

    /// A filter to return only managed instances that are attached to the specified group
    pub group: Option<String>,

    /// A filter to return only managed instances that are NOT attached to the specified group
    pub group_not_equal_to: Option<String>,

    /// A filter to return only managed instances that are associated with the specified lifecycle environment
    pub lifecycle_stage: Option<String>,

    /// A filter to return only managed instances that are NOT associated with the specified lifecycle environment
    pub lifecycle_stage_not_equal_to: Option<String>,

    /// A filter to return only managed instances that are attached to the specified group or lifecycle environment
    pub is_attached_to_group_or_lifecycle_stage: Option<bool>,

    /// The OCID of the software source
    pub software_source_id: Option<String>,

    /// The assigned erratum name
    pub advisory_name: Option<Vec<String>>,

    /// A filter to return only managed instances in a specific lifecycle environment
    pub lifecycle_environment: Option<String>,

    /// A filter to return only managed instances that aren't in a specific lifecycle environment
    pub lifecycle_environment_not_equal_to: Option<String>,

    /// A filter to return only resources whose location matches the given value
    pub location: Option<Vec<ManagedInstanceLocation>>,

    /// A filter to return only resources whose location does not match the given value
    pub location_not_equal_to: Option<Vec<ManagedInstanceLocation>>,

    /// A multi filter to return only managed instances that match the given profile ids
    pub profile: Option<Vec<String>>,

    /// A multi filter to return only managed instances that don't contain the given profile OCIDs
    pub profile_not_equal_to: Option<Vec<String>>,

    /// A filter to return only managed instances with a registration profile attached
    pub is_profile_attached: Option<bool>,

    /// Indicates whether to list only resources managed by the Autonomous Linux service
    pub is_managed_by_autonomous_linux: Option<bool>,

    /// A filter to return only managed instances with the specified version of osmh-agent running
    pub agent_version: Option<String>,

    /// Management station OCIDs
    pub management_station: Option<Vec<String>>,

    /// Management station OCIDs to exclude
    pub management_station_not_equal_to: Option<Vec<String>>,

    /// A filter to return only managed instances that require a reboot to install updates
    pub is_reboot_required: Option<bool>,

    /// For list pagination. The maximum number of results per page
    pub limit: Option<u32>,

    /// For list pagination. The value of the opc-next-page response header from the previous "List" call
    pub page: Option<String>,

    /// The sort order to use
    pub sort_order: Option<SortOrder>,

    /// The field to sort by
    pub sort_by: Option<ListManagedInstancesSortBy>,

    /// Unique Oracle-assigned identifier for the request
    pub opc_request_id: Option<String>,
}

impl ListManagedInstancesRequest {
    /// Create a new builder for ListManagedInstancesRequest
    pub fn builder() -> ListManagedInstancesRequestBuilder {
        ListManagedInstancesRequestBuilder::default()
    }

    /// Convert request to query parameters
    pub fn to_query_params(&self) -> Vec<(String, String)> {
        let mut params = Vec::new();

        if let Some(ref v) = self.compartment_id {
            params.push(("compartmentId".to_string(), v.clone()));
        }
        if let Some(ref v) = self.display_name {
            for name in v {
                params.push(("displayName".to_string(), name.clone()));
            }
        }
        if let Some(ref v) = self.display_name_contains {
            params.push(("displayNameContains".to_string(), v.clone()));
        }
        if let Some(ref v) = self.managed_instance_id {
            params.push(("managedInstanceId".to_string(), v.clone()));
        }
        if let Some(ref v) = self.status {
            for status in v {
                params.push(("status".to_string(), format!("{:?}", status).to_uppercase()));
            }
        }
        if let Some(ref v) = self.arch_type {
            for arch in v {
                params.push(("archType".to_string(), format!("{:?}", arch).to_uppercase()));
            }
        }
        if let Some(ref v) = self.os_family {
            for os in v {
                params.push(("osFamily".to_string(), format!("{:?}", os)));
            }
        }
        if let Some(v) = self.is_management_station {
            params.push(("isManagementStation".to_string(), v.to_string()));
        }
        if let Some(ref v) = self.group {
            params.push(("group".to_string(), v.clone()));
        }
        if let Some(ref v) = self.group_not_equal_to {
            params.push(("groupNotEqualTo".to_string(), v.clone()));
        }
        if let Some(ref v) = self.lifecycle_stage {
            params.push(("lifecycleStage".to_string(), v.clone()));
        }
        if let Some(ref v) = self.lifecycle_stage_not_equal_to {
            params.push(("lifecycleStageNotEqualTo".to_string(), v.clone()));
        }
        if let Some(v) = self.is_attached_to_group_or_lifecycle_stage {
            params.push((
                "isAttachedToGroupOrLifecycleStage".to_string(),
                v.to_string(),
            ));
        }
        if let Some(ref v) = self.software_source_id {
            params.push(("softwareSourceId".to_string(), v.clone()));
        }
        if let Some(ref v) = self.advisory_name {
            for name in v {
                params.push(("advisoryName".to_string(), name.clone()));
            }
        }
        if let Some(ref v) = self.lifecycle_environment {
            params.push(("lifecycleEnvironment".to_string(), v.clone()));
        }
        if let Some(ref v) = self.lifecycle_environment_not_equal_to {
            params.push(("lifecycleEnvironmentNotEqualTo".to_string(), v.clone()));
        }
        if let Some(ref v) = self.location {
            for loc in v {
                params.push(("location".to_string(), format!("{:?}", loc)));
            }
        }
        if let Some(ref v) = self.location_not_equal_to {
            for loc in v {
                params.push(("locationNotEqualTo".to_string(), format!("{:?}", loc)));
            }
        }
        if let Some(ref v) = self.profile {
            for p in v {
                params.push(("profile".to_string(), p.clone()));
            }
        }
        if let Some(ref v) = self.profile_not_equal_to {
            for p in v {
                params.push(("profileNotEqualTo".to_string(), p.clone()));
            }
        }
        if let Some(v) = self.is_profile_attached {
            params.push(("isProfileAttached".to_string(), v.to_string()));
        }
        if let Some(v) = self.is_managed_by_autonomous_linux {
            params.push(("isManagedByAutonomousLinux".to_string(), v.to_string()));
        }
        if let Some(ref v) = self.agent_version {
            params.push(("agentVersion".to_string(), v.clone()));
        }
        if let Some(ref v) = self.management_station {
            for ms in v {
                params.push(("managementStation".to_string(), ms.clone()));
            }
        }
        if let Some(ref v) = self.management_station_not_equal_to {
            for ms in v {
                params.push(("managementStationNotEqualTo".to_string(), ms.clone()));
            }
        }
        if let Some(v) = self.is_reboot_required {
            params.push(("isRebootRequired".to_string(), v.to_string()));
        }
        if let Some(v) = self.limit {
            params.push(("limit".to_string(), v.to_string()));
        }
        if let Some(ref v) = self.page {
            params.push(("page".to_string(), v.clone()));
        }
        if let Some(v) = self.sort_order {
            params.push(("sortOrder".to_string(), format!("{:?}", v).to_uppercase()));
        }
        if let Some(v) = self.sort_by {
            params.push(("sortBy".to_string(), v.to_string()));
        }

        params
    }
}

/// Builder for ListManagedInstancesRequest
#[derive(Debug, Default)]
pub struct ListManagedInstancesRequestBuilder {
    request: ListManagedInstancesRequest,
}

impl ListManagedInstancesRequestBuilder {
    pub fn compartment_id(mut self, compartment_id: impl Into<String>) -> Self {
        self.request.compartment_id = Some(compartment_id.into());
        self
    }

    pub fn display_name(mut self, display_name: Vec<String>) -> Self {
        self.request.display_name = Some(display_name);
        self
    }

    pub fn display_name_contains(mut self, display_name_contains: impl Into<String>) -> Self {
        self.request.display_name_contains = Some(display_name_contains.into());
        self
    }

    pub fn status(mut self, status: Vec<ManagedInstanceStatus>) -> Self {
        self.request.status = Some(status);
        self
    }

    pub fn arch_type(mut self, arch_type: Vec<ArchType>) -> Self {
        self.request.arch_type = Some(arch_type);
        self
    }

    pub fn os_family(mut self, os_family: Vec<OsFamily>) -> Self {
        self.request.os_family = Some(os_family);
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

    pub fn sort_by(mut self, sort_by: ListManagedInstancesSortBy) -> Self {
        self.request.sort_by = Some(sort_by);
        self
    }

    pub fn build(self) -> ListManagedInstancesRequest {
        self.request
    }
}

/// Response from listing managed instances
#[derive(Debug, Clone)]
pub struct ListManagedInstancesResponse {
    /// The collection of managed instances
    pub managed_instance_collection: ManagedInstanceCollection,

    /// Unique Oracle-assigned identifier for the request
    pub opc_request_id: Option<String>,

    /// For list pagination. When this header appears in the response, additional pages of results remain
    pub opc_next_page: Option<String>,

    /// The total number of items in the result
    pub opc_total_items: Option<u32>,
}
