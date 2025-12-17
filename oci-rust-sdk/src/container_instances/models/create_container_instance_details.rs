use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateContainerInstanceDetails {
    pub compartment_id: String,

    pub availability_domain: String,

    pub shape: String,

    pub shape_config: CreateContainerInstanceShapeConfigDetails,

    pub containers: Vec<CreateContainerDetails>,

    pub vnics: Vec<CreateContainerVnicDetails>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub fault_domain: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub graceful_shutdown_timeout_in_seconds: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_restart_policy: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub freeform_tags: Option<HashMap<String, String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub defined_tags: Option<HashMap<String, HashMap<String, serde_json::Value>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateContainerInstanceShapeConfigDetails {
    pub ocpus: f32,

    pub memory_in_gbs: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateContainerDetails {
    pub image_url: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<Vec<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Vec<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment_variables: Option<HashMap<String, String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_config: Option<CreateContainerResourceConfigDetails>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateContainerResourceConfigDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vcpus_limit: Option<f32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_limit_in_gbs: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateContainerVnicDetails {
    pub subnet_id: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub hostname_label: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_public_ip_assigned: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_source_dest_check: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub nsg_ids: Option<Vec<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_ip: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub freeform_tags: Option<HashMap<String, String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub defined_tags: Option<HashMap<String, HashMap<String, serde_json::Value>>>,
}
