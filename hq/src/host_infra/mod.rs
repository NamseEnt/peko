pub mod oci;

use crate::*;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use std::{future::Future, net::IpAddr, pin::Pin, sync::Arc, time::Duration};
use tokio::time::{MissedTickBehavior, interval};

const SYNC_HOST_INFO_INTERVAL: Duration = Duration::from_secs(10);

pub type HostInfoMap = Arc<DashMap<HostId, HostInfo>>;

pub async fn run_sync_host_info_map(
    host_infra: Arc<dyn HostInfra>,
    host_info_map: HostInfoMap,
) -> Result<()> {
    let mut interval = interval(SYNC_HOST_INFO_INTERVAL);
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        interval.tick().await;
        if let Err(err) = host_infra.sync_host_info_map(host_info_map.clone()).await {
            println!("Failed to sync host info map: {err}");
        }
    }
}

#[derive(Debug, Clone)]
pub struct HostInfo {
    pub id: HostId,
    pub instance_created: DateTime<Utc>,
    pub ip: Option<IpAddr>,
    pub instance_state: HostInstanceState,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HostInstanceState {
    Starting,
    Running,
    Terminating,
}

pub trait HostInfra: Send + Sync {
    fn sync_host_info_map<'a>(
        &'a self,
        host_info_map: HostInfoMap,
    ) -> Pin<Box<dyn Future<Output = color_eyre::Result<()>> + 'a + Send>>;

    fn terminate<'a>(
        &'a self,
        host_id: &'a HostId,
    ) -> Pin<Box<dyn Future<Output = color_eyre::Result<()>> + 'a + Send>>;

    fn launch_instance<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = color_eyre::Result<()>> + 'a + Send>>;
}
