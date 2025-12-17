use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::enums::ContainerInstanceLifecycleState;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerInstance {
    pub id: String,

    pub display_name: String,

    pub compartment_id: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub freeform_tags: Option<HashMap<String, String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub defined_tags: Option<HashMap<String, HashMap<String, serde_json::Value>>>,

    pub availability_domain: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub fault_domain: Option<String>,

    pub lifecycle_state: ContainerInstanceLifecycleState,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle_details: Option<String>,

    pub container_count: i32,

    pub time_created: DateTime<Utc>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_updated: Option<DateTime<Utc>>,

    pub shape: String,

    pub shape_config: ContainerInstanceShapeConfig,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub vnics: Option<Vec<ContainerVnic>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub containers: Option<Vec<ContainerInstanceContainer>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub graceful_shutdown_timeout_in_seconds: Option<i64>,

    pub container_restart_policy: ContainerRestartPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerInstanceShapeConfig {
    pub ocpus: f32,

    pub memory_in_gbs: f32,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub processor_description: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub networking_bandwidth_in_gbps: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerVnic {
    pub vnic_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerInstanceContainer {
    pub container_id: String,

    pub display_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ContainerRestartPolicy {
    Always,
    Never,
    OnFailure,
}
