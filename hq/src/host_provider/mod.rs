pub mod oci_container;

use crate::*;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use std::{future::Future, net::IpAddr, pin::Pin, sync::Arc, time::Duration};
use tokio::time::{MissedTickBehavior, interval};

const SYNC_HOST_INFO_INTERVAL: Duration = Duration::from_secs(10);

pub type HostInfoMap = Arc<DashMap<HostId, HostInfo>>;

#[instrument(skip_all, name = "sync_host_info_loop")]
pub async fn run_sync_host_info_map(
    host_provider: Arc<dyn HostProvider>,
    host_info_map: HostInfoMap,
) -> Result<()> {
    info!("Starting sync host info loop");

    let mut interval = interval(SYNC_HOST_INFO_INTERVAL);
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        interval.tick().await;
        info!("sync host info tick");

        if let Err(err) = host_provider
            .sync_host_info_map(host_info_map.clone())
            .await
        {
            error!(%err, "Failed to sync host info map");
            telemetry::send_sync_host_info_status(false);
        } else {
            telemetry::send_sync_host_info_status(true);
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

pub trait HostProvider: Send + Sync {
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
