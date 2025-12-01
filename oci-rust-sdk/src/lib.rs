pub mod core;
#[cfg(feature = "os_management_hub")]
pub mod os_management_hub;
#[cfg(feature = "resource_search")]
pub mod resource_search;
#[cfg(feature = "virtual_network")]
pub mod virtual_network;

pub use core::{
    client::OciClient,
    error::{OciError, Result},
    retry::{Retrier, RetryConfiguration},
};
// Re-export os_management_hub types for convenience
#[cfg(feature = "os_management_hub")]
pub use os_management_hub::OsManagementHub;

// Re-export resource_search types for convenience
#[cfg(feature = "resource_search")]
pub use resource_search::ResourceSearch;

// Re-export virtual_network types for convenience
#[cfg(feature = "virtual_network")]
pub use virtual_network::VirtualNetwork;
