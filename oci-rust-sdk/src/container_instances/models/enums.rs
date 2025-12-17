use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ContainerInstanceLifecycleState {
    Creating,
    Updating,
    Active,
    Inactive,
    Deleting,
    Deleted,
    Failed,
}

impl fmt::Display for ContainerInstanceLifecycleState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Creating => write!(f, "CREATING"),
            Self::Updating => write!(f, "UPDATING"),
            Self::Active => write!(f, "ACTIVE"),
            Self::Inactive => write!(f, "INACTIVE"),
            Self::Deleting => write!(f, "DELETING"),
            Self::Deleted => write!(f, "DELETED"),
            Self::Failed => write!(f, "FAILED"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum SortOrder {
    Asc,
    Desc,
}

impl fmt::Display for SortOrder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Asc => write!(f, "ASC"),
            Self::Desc => write!(f, "DESC"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum SortBy {
    TimeCreated,
    DisplayName,
}

impl fmt::Display for SortBy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TimeCreated => write!(f, "TIMECREATED"),
            Self::DisplayName => write!(f, "DISPLAYNAME"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ContainerLifecycleState {
    Creating,
    Updating,
    Active,
    Inactive,
    Deleting,
    Deleted,
    Failed,
}

impl fmt::Display for ContainerLifecycleState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Creating => write!(f, "CREATING"),
            Self::Updating => write!(f, "UPDATING"),
            Self::Active => write!(f, "ACTIVE"),
            Self::Inactive => write!(f, "INACTIVE"),
            Self::Deleting => write!(f, "DELETING"),
            Self::Deleted => write!(f, "DELETED"),
            Self::Failed => write!(f, "FAILED"),
        }
    }
}
