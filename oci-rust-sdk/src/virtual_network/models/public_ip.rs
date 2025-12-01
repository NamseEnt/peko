use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::enums::*;

/// A public IP address
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicIp {
    /// The OCID of the public IP
    pub id: String,

    /// The public IP address
    pub ip_address: String,

    /// Whether the public IP is regional or AD-specific
    pub scope: Scope,

    /// Defines when the public IP is deleted and released back to the pool
    pub lifetime: Lifetime,

    /// The OCID of the compartment containing the public IP
    pub compartment_id: String,

    /// The availability domain (for AD-scoped public IPs)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub availability_domain: Option<String>,

    /// The OCID of the entity the public IP is assigned to
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assigned_entity_id: Option<String>,

    /// The type of entity the public IP is assigned to
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assigned_entity_type: Option<AssignedEntityType>,

    /// The OCID of the private IP that the public IP is currently assigned to
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_ip_id: Option<String>,

    /// The public IP's current lifecycle state
    pub lifecycle_state: PublicIpLifecycleState,

    /// A user-friendly name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,

    /// The date and time the public IP was created
    pub time_created: DateTime<Utc>,

    /// The OCID of the public IP pool
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_ip_pool_id: Option<String>,

    /// Defined tags for this resource
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defined_tags: Option<HashMap<String, HashMap<String, serde_json::Value>>>,

    /// Free-form tags for this resource
    #[serde(skip_serializing_if = "Option::is_none")]
    pub freeform_tags: Option<HashMap<String, String>>,
}
