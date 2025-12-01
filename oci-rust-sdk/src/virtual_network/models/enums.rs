use serde::{Deserialize, Serialize};

/// Scope of the public IP (REGION or AVAILABILITY_DOMAIN)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Scope {
    Region,
    AvailabilityDomain,
}

/// Lifetime of the public IP (EPHEMERAL or RESERVED)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Lifetime {
    Ephemeral,
    Reserved,
}

/// Type of entity the public IP is assigned to
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AssignedEntityType {
    PrivateIp,
    NatGateway,
}

/// Lifecycle state of the public IP
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PublicIpLifecycleState {
    Provisioning,
    Available,
    Assigning,
    Assigned,
    Unassigning,
    Unassigned,
    Terminating,
    Terminated,
}
