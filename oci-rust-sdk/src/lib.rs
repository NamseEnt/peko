#![allow(clippy::large_enum_variant)]

#[cfg(feature = "compute")]
pub mod compute;
#[cfg(feature = "container_instances")]
pub mod container_instances;
pub mod core;
#[cfg(feature = "os_management_hub")]
pub mod os_management_hub;
#[cfg(feature = "resource_search")]
pub mod resource_search;
#[cfg(feature = "virtual_network")]
pub mod virtual_network;
