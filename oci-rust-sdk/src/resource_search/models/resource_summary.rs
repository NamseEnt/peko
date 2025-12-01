use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::search_context::SearchContext;

/// A resource that exists in the cloud network that you're querying.
///
/// Contains information about a single resource returned from a search query.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSummary {
    /// The resource type name (e.g., "Instance", "VolumeAttachment").
    pub resource_type: String,

    /// The unique identifier for this particular resource, usually an OCID.
    pub identifier: String,

    /// The OCID of the compartment that contains this resource.
    pub compartment_id: String,

    /// The time that this resource was created.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_created: Option<DateTime<Utc>>,

    /// The display name (or name) of this resource, if one exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,

    /// The availability domain where this resource exists, if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub availability_domain: Option<String>,

    /// The lifecycle state of this resource, if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle_state: Option<String>,

    /// Free-form tags for this resource. Each tag is a simple key-value pair with no
    /// predefined name, type, or namespace.
    ///
    /// Example: `{"Department": "Finance"}`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub freeform_tags: Option<HashMap<String, String>>,

    /// Defined tags for this resource. Each key is predefined and scoped to a namespace.
    ///
    /// Example: `{"Operations": {"CostCenter": "42"}}`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defined_tags: Option<HashMap<String, HashMap<String, serde_json::Value>>>,

    /// System tags associated with this resource, if any. System tags are set by
    /// Oracle Cloud Infrastructure services. Each key is predefined and scoped to namespaces.
    ///
    /// Example: `{"orcl-cloud": {"free-tier-retain": true}}`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_tags: Option<HashMap<String, HashMap<String, serde_json::Value>>>,

    /// Additional search context showing what matched the query (when highlights are requested).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_context: Option<SearchContext>,

    /// Additional identifiers to use together in a "Get" request for a specified resource.
    /// Only required for resource types that explicitly cannot be retrieved by using a single
    /// identifier, such as the resource's OCID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_context: Option<HashMap<String, serde_json::Value>>,

    /// Additional resource attribute fields of this resource that match queries with a return clause.
    ///
    /// For example, if you ran a query to find the private IP addresses, public IP addresses,
    /// and isPrimary field of the VNIC attachment on instance resources, that field would be
    /// included in the ResourceSummary object as:
    /// ```json
    /// {"additionalDetails": {"attachedVnic": [{"publicIP": "172.110.110.110", "privateIP": "10.10.10.10", "isPrimary": true}]}}
    /// ```
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_details: Option<HashMap<String, serde_json::Value>>,
}

impl ResourceSummary {
    /// Create a new ResourceSummaryBuilder to construct a ResourceSummary.
    pub fn builder(
        resource_type: impl Into<String>,
        identifier: impl Into<String>,
        compartment_id: impl Into<String>,
    ) -> ResourceSummaryBuilder {
        ResourceSummaryBuilder {
            resource_type: resource_type.into(),
            identifier: identifier.into(),
            compartment_id: compartment_id.into(),
            time_created: None,
            display_name: None,
            availability_domain: None,
            lifecycle_state: None,
            freeform_tags: None,
            defined_tags: None,
            system_tags: None,
            search_context: None,
            identity_context: None,
            additional_details: None,
        }
    }
}

/// Builder for ResourceSummary.
#[derive(Debug, Clone)]
pub struct ResourceSummaryBuilder {
    resource_type: String,
    identifier: String,
    compartment_id: String,
    time_created: Option<DateTime<Utc>>,
    display_name: Option<String>,
    availability_domain: Option<String>,
    lifecycle_state: Option<String>,
    freeform_tags: Option<HashMap<String, String>>,
    defined_tags: Option<HashMap<String, HashMap<String, serde_json::Value>>>,
    system_tags: Option<HashMap<String, HashMap<String, serde_json::Value>>>,
    search_context: Option<SearchContext>,
    identity_context: Option<HashMap<String, serde_json::Value>>,
    additional_details: Option<HashMap<String, serde_json::Value>>,
}

impl ResourceSummaryBuilder {
    pub fn time_created(mut self, time_created: DateTime<Utc>) -> Self {
        self.time_created = Some(time_created);
        self
    }

    pub fn display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = Some(display_name.into());
        self
    }

    pub fn availability_domain(mut self, availability_domain: impl Into<String>) -> Self {
        self.availability_domain = Some(availability_domain.into());
        self
    }

    pub fn lifecycle_state(mut self, lifecycle_state: impl Into<String>) -> Self {
        self.lifecycle_state = Some(lifecycle_state.into());
        self
    }

    pub fn freeform_tags(mut self, freeform_tags: HashMap<String, String>) -> Self {
        self.freeform_tags = Some(freeform_tags);
        self
    }

    pub fn defined_tags(
        mut self,
        defined_tags: HashMap<String, HashMap<String, serde_json::Value>>,
    ) -> Self {
        self.defined_tags = Some(defined_tags);
        self
    }

    pub fn system_tags(
        mut self,
        system_tags: HashMap<String, HashMap<String, serde_json::Value>>,
    ) -> Self {
        self.system_tags = Some(system_tags);
        self
    }

    pub fn search_context(mut self, search_context: SearchContext) -> Self {
        self.search_context = Some(search_context);
        self
    }

    pub fn identity_context(
        mut self,
        identity_context: HashMap<String, serde_json::Value>,
    ) -> Self {
        self.identity_context = Some(identity_context);
        self
    }

    pub fn additional_details(
        mut self,
        additional_details: HashMap<String, serde_json::Value>,
    ) -> Self {
        self.additional_details = Some(additional_details);
        self
    }

    pub fn build(self) -> ResourceSummary {
        ResourceSummary {
            resource_type: self.resource_type,
            identifier: self.identifier,
            compartment_id: self.compartment_id,
            time_created: self.time_created,
            display_name: self.display_name,
            availability_domain: self.availability_domain,
            lifecycle_state: self.lifecycle_state,
            freeform_tags: self.freeform_tags,
            defined_tags: self.defined_tags,
            system_tags: self.system_tags,
            search_context: self.search_context,
            identity_context: self.identity_context,
            additional_details: self.additional_details,
        }
    }
}
